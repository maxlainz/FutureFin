# RFC — «Planes» (6.0.0): la jubilación deja de ser un modelo y pasa a ser una hipótesis

Documento de diseño **vivo** del rediseño 6.0.0. Recoge lo decidido con el owner, lo que sigue
abierto y el inventario de impacto en el código. Es la referencia para cualquier sesión que
continúe este trabajo: **léelo entero antes de tocar nada de 6.0.0** y actualízalo en el mismo PR
cuando una decisión cambie o un abierto se cierre.

- **Estado**: diseño en debate. No hay código todavía.
- **Rama de trabajo**: ver §10. Este documento vive en `main`.
- **Fecha del debate de origen**: 2026-10-17.

## 1. Motivación

- **La jubilación es una hipótesis sin garantías, no una página.** Puede que con los datos de hoy
  no llegue nunca, y en ese caso el modelo actual no da un resultado útil. Con `asap` imposible, la
  serie publicada sigue en acumulación hasta el horizonte. Si además hay déficit, se venden
  ilíquidos y aparece un patrimonio negativo **sin ningún fallo**: Monte Carlo solo cuenta F1/F2
  con `phase ∈ {Retired, Partial}` (`crates/engine/src/sim_core.rs:1401`).
- **Lo «especial» de la jubilación es poco y está escondido.** En el mes de jubilación el bucle solo
  cambia ingreso y gasto (`sim_core.rs:1196-1240`). La venta mensual es la cobertura genérica de
  déficit que ya existe en cualquier fase (`sim_core.rs:1297-1358`), y el SWR es solo una
  comprobación en el primer mes jubilado (`sim_core.rs:1286-1294`). Pensión, media jornada, pausa
  de ingresos y coast son, en el fondo, flujos de caja con fecha.
- **Vicio oculto de inflación.** El gasto se indexa al IPC y el ingreso queda plano (#139,
  `sim_core.rs:1224-1236`). El motor supone en silencio que el sueldo real pierde ~2,5 % al año
  para siempre, y por eso el patrimonio tiende a cero con mucha facilidad.

## 2. Decisiones tomadas

| # | Decisión |
|---|---|
| D-1 | La pestaña «Simulación» (antes «Próximos») pasa a llamarse **«Planes»**. La ruta pasa a ser `/planes`; `/proximos`, `/simulacion` y `/jubilacion` quedan como alias. |
| D-2 | **Realidad** (activos, pasivos, presupuesto, transacciones) **no se toca** desde un plan. |
| D-3 | Un **plan** = supuestos + bloques de movimientos con fecha. Cada bloque y cada movimiento tiene interruptor. |
| D-4 | **Todo movimiento se expresa como delta sobre la realidad de hoy**, nunca señalando una partida del presupuesto: «desde 03/2028 mi gasto baja 900 €/mes». Así sirve igual con `savings_source` A, B o C. |
| D-5 | Los importes se escriben en **euros de hoy**, y tiene que ser **evidente** (§5). El motor sigue calculando en nominal; la conversión la hace el servidor. |
| D-6 | Cada movimiento lleva «☑ mantiene su poder adquisitivo» (por defecto) / «☐ importe fijo». Los préstamos son siempre nominales. |
| D-7 | **Se elimina el modelo de jubilación**: perfil, estrategias (asap, retire_at_age, coast, partial), SWR, F2, número FIRE, umbral de éxito, solves y la pestaña Jubilación. **No se migra nada**: los perfiles existentes se borran. |
| D-8 | La jubilación, la pensión o la media jornada son simples movimientos (ingreso −100 % desde X, ingreso +N desde Y…). |
| D-9 | **Éxito = poder pagar los gastos todos los meses hasta el horizonte.** Monte Carlo evalúa el **plan entero**, no solo la jubilación. |
| D-10 | **Sin «vendible».** Lo ilíquido no se vende nunca de forma automática. Vender un activo, entero o en parte, es un movimiento explícito («vendo X en fecha F»), con su plusvalía. |
| D-11 | **Sin búsqueda de fecha.** Las fechas son siempre fijas: el usuario las mueve y ve cómo cambia la probabilidad. |
| D-12 | **Plan por defecto invisible.** Es la realidad sin movimientos, con los supuestos base, y es lo que se usa cuando no hay ningún plan activo. |
| D-13 | **Un solo plan activo a la vez**: activar uno desactiva los demás. |
| D-14 | **El plan activo alimenta todo**: Resumen, gráfico de proyección, bandas, metas y MCP. |
| D-15 | **La inflación es por plan.** Los supuestos base (los del plan por defecto) son el valor inicial de cada plan nuevo. |
| D-16 | **Finanzas compartidas**: un interruptor de la instalación, solo para el owner. Es una cuestión de **permisos y visualización, no de propiedad**. Activado: todos los miembros ven y editan todo, los planes individuales se **ocultan** (no se borran) y solo hay planes de hogar, que se simulan como **una sola unidad**. Desactivado: un plan activo por persona y el hogar como suma, como hoy. |
| D-17 | **Plantillas** (comprar vivienda, dejar de trabajar…): **más adelante**, no entran en 6.0.0. |
| D-18 | **Se eliminan las fechas de caducidad del presupuesto**: `expense_end_date`, `ends_at_retirement` y `persists_after_retirement`. En la realidad nada caduca; lo que cambia en el futuro es un movimiento de un plan. (`payment_end_date` de un pasivo se queda: es un dato del contrato.) |
| D-19 | Los **viewers** ven el hogar en solo lectura. Que puedan crear planes-hipótesis propios (un hijo que estudia un Erasmus, por ejemplo) queda **para más adelante**. |
| D-20 | **Issues** que se cierran con 6.0.0 porque desaparecen o se resuelven: #219, #221, #222, #223, #224, #225, #226, #227, #260. Siguen vigentes: #217 (modelo Monte Carlo) y #218 (mortalidad). |

## 3. Modelo (borrador)

**Tipos de movimiento** (verbos sobre un sujeto):

| Sujeto | Verbos | Ejemplo |
|---|---|---|
| Mi ingreso / mi gasto | sube · baja (en € o en %), puntual o mensual, con inicio y fin opcional | «Desde 2031 mi ingreso sube 200 €/mes *por encima de la inflación*» |
| Un activo (real o del plan) | entra · retiro dinero de él · lo vendo entero | «En 2029 entra una casa de 250.000 €, ilíquida, al 2 %» |
| Un préstamo | lo firmo (importe, TAE, plazo, sistema) | «Hipoteca de 200.000 € en 03/2028, 3 %, 25 años, francés» |

**Esquema de tablas (borrador, se revisa en la fase 2):**

- `plans`: `id`, `installation_id`, `owner_user_id` (NULL = plan de hogar), `name`, `is_active`,
  `assumptions` JSONB.
- `plan_blocks`: `id`, `plan_id`, `name`, `enabled`, `sort_index`.
- `plan_movements`: `id`, `block_id`, `kind`, `start_date`, `end_date`, `amount`, `amount_unit`
  (`eur`/`pct`), `keeps_purchasing_power`, `asset_ref`, `params` JSONB, `enabled`.
- Los `planning_flows` actuales se convierten en movimientos del plan por defecto, o en un plan
  «Importado» — **por decidir**.

**Motor:**

- `SimAsset` y `ProjectionLiabilityInput` ganan un mes de inicio. Hoy no lo tienen
  (`crates/engine/src/projection.rs:45-52`, `:109-158`).
- Ingreso y gasto pasan a ser vectores mensuales, cada uno con su indexación.
- Las retiradas y ventas explícitas reutilizan la venta con gross-up que ya existe
  (`sim_core.rs:491-597`).
- **Fallo**: un mes en que lo líquido no alcanza para el gasto. Se quita la puerta de fase de
  `sim_core.rs:1401`. Lo ilíquido no entra en `drain_order_g` (`sim_core.rs:299-315`).
- Se mantienen el núcleo genérico `MoneyOps`, `mc.rs` y la puerta de degeneración
  (`crates/engine-stochastic/tests/degeneration.rs`).

## 4. Supuestos del plan

| Supuesto | Dónde | Por defecto |
|---|---|---|
| Inflación | Plan | La de los supuestos base |
| Ingresos siguen la inflación | Plan | Sí |
| Gastos siguen la inflación | Plan | Sí |
| Horizonte (edad final) | Plan | 90; en un plan de hogar, del miembro más joven |
| Ajuste de mercado (rentabilidades ± pp) | Plan, opcional | 0 |
| Reglas de reparto del excedente | **Abierto** (§8.2) | — |
| Rentabilidad y volatilidad por activo | Global, en el activo | — |
| Impuestos y tramos | **Abierto** (§8.1) | — |
| `savings_source` y sus ventanas | Global | Es cómo se mide la realidad |

- Los supuestos se ven **arriba del plan, en la misma pantalla que el resultado**. Nada que mueva
  mucho la cifra puede quedar escondido.
- **Dónde se editan los supuestos base** (los del plan invisible): propuesta, una sección «Base» al
  principio de la pestaña Planes. Se puede reconsiderar en Ajustes.
- Cada respuesta de proyección **ecoa** la inflación que usó. Los deflactores del frontend leen
  solo ese eco y nunca recurren a la inflación de la instalación
  (`ProjectionNetWorthChart.tsx:305-353` hoy sí lo hace).
- `deflate_amount` (MCP) toma el plan activo por defecto y ecoa qué inflación aplicó.

## 5. «Euros de hoy» evidente

Requisito del owner: nadie puede pasarse 15 años pensando que con X nominal se jubila y descubrir
al llegar que no lo había entendido.

- **El gráfico se muestra en € de hoy por defecto**, con un interruptor a nominal y la etiqueta
  siempre visible.
- **Cada hito lleva las dos cifras**: «Patrimonio a los 65: **560.000 € de hoy** (≈ 1.200.000 € de
  2061)». Las cifras son de ejemplo.
- **El tooltip de cualquier punto** muestra el valor nominal y la inflación acumulada hasta esa
  fecha.
- **En el formulario**, con «ingresos siguen la inflación», el texto dice «+200 €/mes *por encima
  de la inflación*».

## 6. Finanzas compartidas (D-16) — implicaciones

- **Permisos.** Hoy un único gate exige ser dueño de la fila (`require_row_owner`,
  `apps/api/src/handlers/person_view.rs:23-41`). Se llama en assets, liabilities, budget,
  allocation_rules y planning, y los DELETE además re-filtran por `owner_user_id`. Con el
  interruptor activado, ese gate lee el flag y deja editar a cualquier miembro con rol de
  escritura. Los viewers siguen sin escribir (`membership.rs:56`).
- **Simulación.** Una sola corrida con `LedgerView::Household`. El camino ya existe para las metas
  (`apps/api/src/handlers/allocation_rules.rs:1548`). La cache y la semilla de Monte Carlo pasan a
  ser por plan de instalación, y desaparecen `members[]`, `household_not_simulable` y
  `household_bands_unavailable`.
- **Edades.** Los movimientos guardan fechas. «A los 67 de Ana» es una ayuda del formulario que
  guarda edad y miembro, para que se recalcule si cambia su fecha de nacimiento.
- **Apagar el interruptor otra vez**: los planes de hogar se ocultan y reaparecen los individuales.

## 7. Qué se elimina o reescribe (inventario, 2026-10-17)

- **API**:
  - **Se borran**: `handlers/retirement_profile.rs` (rutas `/v1/auth/me/retirement-profile`) y
    `handlers/retirement_solver.rs`, con sus caches en `state.rs`.
  - **Se reescriben**: `projection.rs`, para quitar `jubilacion_*`, `strategy`, `phase_transitions`,
    `pension_*`, `partial_*`, `safe_date_*`, `needed_capital_*`, `fire_number_classic_*` y la
    agregación por miembro; `projection_bands.rs`, por el éxito sobre el plan entero;
    `summary.rs`, donde la tarjeta «Tu plan» pasa a ser «Plan activo» y el colchón deja de
    depender del SWR; `budget.rs`, que pierde los equivalentes de jubilación y los flags de D-18;
    y los inputs de las metas en `allocation_rules.rs`.
- **Motor**:
  - **Se borran**: `phases.rs`, `target.rs`, `withdrawal.rs` y `solve.rs`; en
    `engine-stochastic`, `solve_mc.rs`, `needed_capital.rs` y `strategy_solves.rs`.
  - **Se mantienen**: `mc.rs`, `parallel.rs` y la puerta de degeneración.
- **MCP**:
  - **Se borran**: `get_retirement_profile` y `update_retirement_profile`.
  - **Se reescriben**: `get_projection`, `get_projection_bands`, `simulate_projection`,
    `get_summary`, `get_budget`, las tools de budget entries y de planning flows, y las
    instrucciones del servidor.
  - **Se añaden** las tools de planes.
  - **Mismo PR**: `tests/mcp_http.rs` (`tools_list_returns_exactly_the_v1_catalog`),
    `tests/fixtures/mcp-catalog.json`, `mcp/schema_bounds_parity.rs` y
    `tests/fixtures/error-codes.json`. Evaluación: skill `futurefin-mcp-parity`.
- **Frontend**:
  - **Se borran**: `views/RetirementView.tsx` y sus helpers (`lib/retirement*.ts`, `plan-*.ts`,
    `phase-strip.ts`, `household-plan-lines.ts`, casi todo `fire.ts`), además de `fire-parity.json`.
  - **Se reescriben**: `UpcomingView.tsx`, que pasa a ser la vista de Planes, `ProjectionView`,
    `ProjectionNetWorthChart`, `MiniProjection`, `SummaryView`, `OnboardingWizard` (con
    `onboarding-plan.ts`), `SettingsView`, `navigation.ts` y `helpTexts.ts` (este último pasa por la
    skill `futurefin-metric-definitions`).
- **Copias de seguridad**: subir `CURRENT_SCHEMA_VERSION` (`backup_user/schema.rs:23`) e ignorar
  `retirement_profile` al importar copias antiguas.
- **Números**: con los ingresos indexados por defecto **cambian todas las cifras**. Se regeneran
  los golden pins (`UPDATE_ENGINE_PINS=1`) y `projection_pins.rs`. Los tests estocásticos que
  dependían del ingreso plano se rediseñan, y el CHANGELOG declara el delta.
- **Docs**: reescritura fuerte de la skill `futurefin-fire-domain-reference`, `api-routes.md`,
  `mcp-catalog.md`, `frontend-structure.md`, `financial-contracts.md` y `engine.md`; se borra
  `docs/jubilacion.md`. En `futurefin-architecture-contract` se retiran o reescriben D2, D11, D23,
  D24 y D25, y se añaden entradas en `futurefin-failure-archaeology`. El párrafo «View scoping» de
  CLAUDE.md también cambia.

## 8. Abierto

1. **Impuestos en activos grandes.**
   - Vender la vivienda habitual y reinvertir en otra está exento en proporción a lo reinvertido.
     Con 65 años o más, la venta de la vivienda habitual está exenta.
   - Propuesta: los tramos siguen siendo globales, porque son ley. Cada venta lleva un **tratamiento
     fiscal** elegido a mano: «tramos del ahorro» (por defecto) o «exenta (vivienda habitual:
     reinversión o ≥ 65)». Si en la fecha de la venta la persona tiene 65 años o más, el formulario
     lo sugiere.
   - No se modela la ley entera; la reinversión parcial y la plusvalía municipal quedan fuera.
   - **Por decidir con el owner.**
2. **Reglas de reparto del excedente** (hoy en `AllocationRulesPanel`, dentro de Presupuesto).
   Propuesta: moverlas al plan. Los supuestos base llevan unas, y cada plan las hereda y puede
   cambiarlas. «Cómo invierto lo que sobra» es una decisión de plan, y en Presupuesto queda
   escondido. **Por decidir.**
3. **Multipropiedad de activos, pasivos y movimientos** (una casa al 50 %, una cuenta conjunta).
   Con finanzas compartidas activadas el problema desaparece, porque todo es común. Con ellas
   desactivadas, hoy se resuelve con filas duplicadas a mano. Una opción futura son las cuotas de
   propiedad por fila. **Hay que pensarlo bien antes de la fase 4.**
4. **Colchón de liquidez.** Es el KPI del Resumen «meses que cubre tu líquido sin ingresos». Hoy se
   marca como «indefinido» cuando el SWR del líquido cubre el gasto (`summary.rs:124-131`,
   `:834-886`). Sin SWR, propuesta: dejarlo en meses de gasto cubiertos y quitar la marca.
5. **Qué pasa con los `planning_flows` existentes** (§3).
6. **Retirada explícita que agota su activo.** Propuesta: el déficit lo cubre la venta automática de
   lo líquido y, si no alcanza, es fallo.
7. **Copia de seguridad de todo el hogar por el admin**, y verificar que el flujo `.ffbackup`
   funciona en el add-on de Home Assistant, donde no hay login explícito. Va **fuera de 6.0.0**,
   como tarea aparte.

## 9. Fases

| Fase | Contenido |
|---|---|
| F0 | Corregir #261 (texto de `deflate_amount`). #260 se cierra con D-18, salvo que 6.0.0 se retrase mucho. |
| F1 | Motor: activos y pasivos con mes de inicio, ingreso y gasto como vectores indexables, ventas y retiradas explícitas, éxito sobre el plan entero; se eliminan las fases. |
| F2 | API y MCP: tablas de planes, plan activo, proyección, bandas, Resumen y metas; se elimina la jubilación. |
| F3 | Finanzas compartidas: permisos y simulación única del hogar. |
| F4 | UI: pestaña Planes, formulario de frase, € de hoy; se elimina la pestaña Jubilación. |
| F5 | Docs y skills; release 6.0.0. |

## 10. Rama de trabajo

`main` publica una imagen por cada bump. Si las fases F1 a F5 se mergearan una a una en `main`, un
parche 5.0.x quedaría a medio camino de 6.0.0. **Propuesta**: una rama de integración
**`next/6.0`** que sale de `main`. Las fases llegan a ella por PR, `main` se mergea en ella con
regularidad, y entra en `main` con un único PR que lleva el bump a 6.0.0. Es una excepción
explícita a la norma de rama única de [`git-and-releases.md`](../git-and-releases.md).
**Pendiente del OK del owner**; cuando se confirme, este párrafo pasa a §2.
