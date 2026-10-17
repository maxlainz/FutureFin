# RFC — «Planes» (6.0.0): la jubilación deja de ser un modelo y pasa a ser una hipótesis

Documento de diseño **vivo** del rediseño 6.0.0. Recoge lo decidido con el owner, lo que sigue
abierto y el inventario de impacto en el código. Es la referencia para cualquier sesión que
continúe este trabajo: **léelo entero antes de tocar nada de 6.0.0** y actualízalo en el mismo PR
cuando una decisión cambie o un abierto se cierre.

- **Estado**: diseño cerrado salvo los abiertos de §8. No hay código todavía. Propuesta visual: D-25.
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
| D-3 | Un **plan** = supuestos + bloques de cambios con fecha. Cada bloque y cada cambio tiene interruptor. |
| D-4 | **Todo cambio se expresa como delta sobre la realidad de hoy**, nunca señalando una partida del presupuesto: «desde 03/2028 mi gasto baja 900 €/mes». Así sirve igual con `savings_source` A, B o C. |
| D-5 | Los importes se escriben en **euros de hoy**, y tiene que ser **evidente** (§5). El motor sigue calculando en nominal; la conversión la hace el servidor. |
| D-6 | Cada cambio lleva «☑ mantiene su poder adquisitivo» (por defecto) / «☐ importe fijo». Los préstamos son siempre nominales. |
| D-7 | **Se elimina el modelo de jubilación**: perfil, estrategias (asap, retire_at_age, coast, partial), SWR, F2, número FIRE, umbral de éxito, solves y la pestaña Jubilación. **No se migra nada**: los perfiles existentes se borran. |
| D-8 | La jubilación, la pensión o la media jornada son simples cambios (ingreso −100 % desde X, ingreso +N desde Y…). |
| D-9 | **Éxito = poder pagar los gastos todos los meses hasta el horizonte.** Monte Carlo evalúa el **plan entero**, no solo la jubilación. |
| D-10 | **Nada se vende solo, ni líquido ni ilíquido.** La venta automática por déficit es un vicio oculto: si el presupuesto no da, es el usuario quien planea qué vende, de forma visible. Vender un activo, entero o en parte, una vez o cada mes, es un cambio explícito («vendo 2.000 €/mes del fondo X desde F»), con su plusvalía y la reducción proporcional del coste del resto. No hay atributo «vendible». Ver §8.8 (efectivo). |
| D-11 | **Sin búsqueda de fecha.** Las fechas son siempre fijas: el usuario las mueve y ve cómo cambia la probabilidad. |
| D-12 | **Sin plan activo = solo la realidad.** No hay una entidad «plan por defecto»: los supuestos por defecto (inflación de referencia, reparto del excedente) viven en **Ajustes**, y cada plan puede sobrescribirlos. Mientras haya un plan activo que los sobrescribe, Ajustes los muestra atenuados con un aviso de que no están en uso. «Solo la realidad» aparece como una opción más entre las tarjetas de plan. |
| D-13 | **Un solo plan activo a la vez**: activar uno desactiva los demás. |
| D-14 | **El plan activo alimenta todo**: Resumen, gráfico de proyección, bandas, metas y MCP. |
| D-15 | **Patrón único de supuestos: valor global en Ajustes + override por plan.** Se aplica a la inflación y a las reglas de reparto del excedente (que salen de Presupuesto, donde quedaban escondidas). |
| D-16 | **Finanzas compartidas**: un interruptor de la instalación, solo para el owner. Es una cuestión de **permisos y visualización, no de propiedad**. Activado: todos los miembros ven y editan todo, los planes individuales se **ocultan** (no se borran) y solo hay planes de hogar, que se simulan como **una sola unidad**. Desactivado: un plan activo por persona y el hogar como suma, como hoy. |
| D-17 | **Plantillas** (comprar vivienda, dejar de trabajar…): **más adelante**, no entran en 6.0.0. |
| D-18 | **Se eliminan las fechas de caducidad del presupuesto**: `expense_end_date`, `ends_at_retirement` y `persists_after_retirement`. En la realidad nada caduca; lo que cambia en el futuro es un cambio de un plan. (`payment_end_date` de un pasivo se queda: es un dato del contrato.) |
| D-19 | Los **viewers** ven el hogar en solo lectura. Que puedan crear planes-hipótesis propios (un hijo que estudia un Erasmus, por ejemplo) queda **para más adelante**. |
| D-20 | **Issues** que se cierran con 6.0.0 porque desaparecen o se resuelven: #219, #221, #222, #223, #224, #225, #226, #227, #260. Siguen vigentes: #217 (modelo Monte Carlo) y #218 (mortalidad). |
| D-21 | **Impuestos**: tramos globales en Ajustes. Cada venta lleva su fiscalidad: **automática** (tramos del ahorro sobre la plusvalía), **exenta** (p. ej. vivienda habitual reinvertida o vendida con 65 años o más; la app no valida la ley) o **manual** (un importe en € o un % de lo vendido). |
| D-22 | **Colchón de liquidez** = meses de gasto que cubre lo líquido, sin la marca «indefinido» (que dependía del SWR). |
| D-23 | **Las piezas del plan se llaman «cambios»**, no «movimientos»: «Movimientos» ya es la pestaña de transacciones. |
| D-24 | **Rama de integración `next/6.0`** (§10): aceptada. |
| D-25 | **Propuesta visual** de referencia: el prototipo navegable del canvas «FutureFin — Planes 6.0» (artifact privado del owner, https://claude.ai/artifact/Ju6HKFPfszH3GXfANN1uE5): pestaña Planes, formulario de cambio de ingreso/gasto, venta de un activo con fiscalidad y Ajustes con los supuestos atenuados. |
| D-26 | **Selector de plan = pestañas** (propuesta B del canvas): una pestaña por plan, el **activo siempre el primero** (marcado con un punto) y «Solo la realidad» **siempre el último**. Abrir un borrador **no lo activa**: un aviso recuerda qué plan alimenta el Resumen y ofrece «Activar este plan» y «Comparar con el activo». Nada de tarjetas: no deben parecer KPIs. |
| D-27 | **Los flujos actuales de Simulación se importan** como cambios de un plan nuevo; conservan su interruptor. Los que no tienen fecha reciben la fecha de la importación + 90 días (hoy se reparten en 90 días). |
| D-28 | **Multipropiedad**: los activos y pasivos llevan cuotas de propiedad que se eligen en su configuración (p. ej. 50/50). Los cambios de un plan y las transacciones se asignan a un activo/cuenta y heredan de él la propiedad; un cambio sin activo (mi ingreso/gasto) es de quien lo crea. Consecuencia: `view=mine` deja de ser «filas mías» y pasa a ser **«mi parte»** (un piso al 50 % cuenta 125.000 € de 250.000 €). |

## 3. Modelo (borrador)

**Tipos de cambio** (verbos sobre un sujeto; D-23):

| Sujeto | Verbos | Ejemplo |
|---|---|---|
| Mi ingreso / mi gasto | sube · baja (en € o en %), puntual o mensual, con inicio y fin opcional | «Desde 2031 mi ingreso sube 200 €/mes *por encima de la inflación*» |
| Un activo (real o del plan) | entra · vendo una parte (una vez o cada mes) · lo vendo entero | «En 2029 entra una casa de 250.000 €, ilíquida, al 2 %» |
| Un préstamo | lo firmo (importe, TAE, plazo, sistema) | «Hipoteca de 200.000 € en 03/2028, 3 %, 25 años, francés» |

**Esquema de tablas (borrador, se revisa en la fase 2):**

- `plans`: `id`, `installation_id`, `owner_user_id` (NULL = plan de hogar), `name`, `is_active`,
  `assumptions` JSONB.
- `plan_blocks`: `id`, `plan_id`, `name`, `enabled`, `sort_index`.
- `plan_changes`: `id`, `block_id`, `kind`, `start_date`, `end_date`, `amount`, `amount_unit`
  (`eur`/`pct`), `keeps_purchasing_power`, `asset_ref`, `params` JSONB, `enabled`.
- Los `planning_flows` actuales: §8.5.

**Motor:**

- `SimAsset` y `ProjectionLiabilityInput` ganan un mes de inicio. Hoy no lo tienen
  (`crates/engine/src/projection.rs:45-52`, `:109-158`).
- Ingreso y gasto pasan a ser vectores mensuales, cada uno con su indexación.
- Las retiradas y ventas explícitas reutilizan la venta con gross-up que ya existe
  (`sim_core.rs:491-597`).
- **Fallo**: un mes en que el gasto no queda cubierto. Se quita la puerta de fase de
  `sim_core.rs:1401` y **desaparece la venta automática por déficit** (`drain_order_g`,
  `sim_core.rs:299-315`; D-10): solo venden los cambios explícitos. Qué cubre un déficit sin venta:
  §8.8.
- Se mantienen el núcleo genérico `MoneyOps`, `mc.rs` y la puerta de degeneración
  (`crates/engine-stochastic/tests/degeneration.rs`).

## 4. Supuestos del plan

| Supuesto | Dónde | Por defecto |
|---|---|---|
| Inflación | Ajustes + override por plan | La de Ajustes (2,5 % hoy) |
| Ingresos siguen la inflación | Plan | Sí |
| Gastos siguen la inflación | Plan | Sí |
| Horizonte (edad final) | Plan | 90; en un plan de hogar, del miembro más joven |
| Ajuste de mercado (rentabilidades ± pp) | Plan, opcional | 0 |
| Reglas de reparto del excedente | Ajustes + override por plan (D-15) | Las de Ajustes |
| Rentabilidad y volatilidad por activo | Global, en el activo | — |
| Impuestos y tramos | Global (Ajustes); fiscalidad por venta (D-21) | Automática |
| `savings_source` y sus ventanas | Global | Es cómo se mide la realidad |

- Los supuestos se ven **arriba del plan, en la misma pantalla que el resultado**. Nada que mueva
  mucho la cifra puede quedar escondido.
- **Los supuestos por defecto se editan en Ajustes** (D-12). Si el plan activo los sobrescribe,
  Ajustes los muestra atenuados con el aviso «ahora mismo no se usan», y un enlace al plan.
- Los indicadores de la realidad que necesitan inflación (colchón, rentabilidad real) usan la del
  plan activo, o la de Ajustes si no hay ninguno.
- Cada respuesta de proyección **ecoa** la inflación que usó. Los deflactores del frontend leen
  solo ese eco y nunca recurren a la inflación de la instalación
  (`ProjectionNetWorthChart.tsx:305-353` hoy sí lo hace).
- `deflate_amount` (MCP) toma el plan activo por defecto y ecoa qué inflación aplicó.

## 5. «Euros de hoy» evidente

Requisito del owner: nadie puede pasarse 15 años pensando que con X nominal se jubila y descubrir
al llegar que no lo había entendido.

- **El gráfico se muestra en € de hoy por defecto**, con un interruptor a nominal y la etiqueta
  siempre visible.
- **Cada hito lleva las dos cifras**: «Patrimonio a los 65: **560.000 € de hoy** (≈ 1.576.000 € de
  2061)», con inflación del 3 % desde 2026. Las cifras son de ejemplo.
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
- **Edades.** Los cambios guardan fechas. «A los 67 de Ana» es una ayuda del formulario que
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

1. ~~Impuestos en activos grandes~~ → D-21.
2. ~~Reglas de reparto del excedente~~ → D-15.
3. ~~Multipropiedad~~ → D-28. Queda por diseñar en F3: el esquema de cuotas (tabla
   `ownership_shares` o JSONB por fila), cómo prorratean los agregados de `view=mine` y qué pasa
   con la frontera de escritura D23 cuando una fila tiene varios dueños.
4. ~~Colchón de liquidez~~ → D-22.
5. ~~`planning_flows` existentes~~ → D-27. Por decidir solo si el plan importado queda activo
   (propuesta: sí, si había algún flujo encendido, para que la proyección no cambie de golpe por
   desaparecer).
6. **Venta explícita que agota su activo**: el cambio se detiene y, si el gasto queda sin cubrir,
   ese mes cuenta como fallo y se señala como «mes en riesgo».
7. **Copia de seguridad de todo el hogar por el admin**, y verificar que el flujo `.ffbackup`
   funciona en el add-on de Home Assistant, donde no hay login explícito. Va **fuera de 6.0.0**,
   como tarea aparte.
8. **Déficit.** El excedente se reparte según las reglas de reparto (el flujo de ahorro), y eso no
   cambia. La duda es el mes con **déficit**: con D-10 nada se vende solo. Propuesta: el déficit
   sale por el **mismo flujo al revés**, del destino del sobrante (la cuenta a la que cae lo que
   sobra), que es lo que pasa en la vida real al gastar de la cuenta. Si ese destino tiene
   plusvalía (un fondo), esa salida tributa y se **muestra** en la lista como «cobertura de déficit»,
   para que no sea un vicio oculto. Cuando el destino se agota, el mes queda sin cubrir: fallo.
   **Por confirmar con el owner.**

## 9. Fases

| Fase | Contenido |
|---|---|
| F0 | Corregir #261 (texto de `deflate_amount`). #260 se cierra con D-18, salvo que 6.0.0 se retrase mucho. |
| F1 | Motor: activos y pasivos con mes de inicio, ingreso y gasto como vectores indexables, ventas y retiradas explícitas, éxito sobre el plan entero; se eliminan las fases. |
| F2 | API y MCP: tablas de planes, plan activo, proyección, bandas, Resumen y metas; se elimina la jubilación. |
| F3 | Finanzas compartidas: permisos y simulación única del hogar. |
| F4 | UI: pestaña Planes, formulario de frase, € de hoy, Ajustes con supuestos atenuados (prototipo: D-25); se elimina la pestaña Jubilación. |
| F5 | Docs y skills; release 6.0.0. |

## 10. Rama de trabajo

`main` publica una imagen por cada bump. Si las fases F1 a F5 se mergearan una a una en `main`, un
parche 5.0.x quedaría a medio camino de 6.0.0. **Propuesta**: una rama de integración
**`next/6.0`** que sale de `main`. Las fases llegan a ella por PR, `main` se mergea en ella con
regularidad, y entra en `main` con un único PR que lleva el bump a 6.0.0. Es una excepción
explícita a la norma de rama única de [`git-and-releases.md`](../git-and-releases.md).
Aceptada (D-24). La rama se crea al empezar F1, y `git-and-releases.md` recoge la excepción en ese
mismo PR.
