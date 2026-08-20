# Changelog

All notable changes to FutureFin will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

Tren del **issue #4** — ergonomía del servidor MCP derivada de una sesión real de uso, más la
resolución de la cascada. Catálogo **47 → 50 tools**. Ninguna migración: nada de esto añade
columnas. Sin cambios de comportamiento en el engine (verificado con un diff antes/después de
`/v1/projection/series`, ver la entrada del refactor).

**Deuda de test cerrada antes de tocar nada**: el modo C solo ejercitaba `create` (ni PATCH ni
DELETE), y tres documentos afirmaban que «crear una regla de categorización no invalida la cache»
estaba pinneado por una regresión **que no existía** — la «regla» del test de modo A era la
*recurrente*. El contrato era correcto en el código; su red de seguridad, imaginaria.

### Fixed — La invalidación de la cache de proyección se espera (cierra una lectura obsoleta)

- **El bug de producción**: `refresh_projection_after_mutation` lanzaba la invalidación en un
  `tokio::spawn`, así que el orden real era `commit → responder → (en algún momento) invalidar`. Un
  GET que cayera en esa ventana servía la proyección **vieja**: el usuario edita algo, recarga
  rápido y la cifra no se mueve. Ahora se espera dentro del handler, de modo que cuando la mutación
  responde el estado de la cache ya es final. El coste es un `retain` sobre un `HashMap` pequeño
  bajo un lock sin contención — microsegundos.
- **El bug de los tests, que es el mismo**: cuatro tests de integración fallaban de forma
  intermitente (4 de 6 pasadas completas en rojo, con tests distintos cada vez). La causa no era
  falta de margen sino **el propio `sleep`**: bajo el runtime `current_thread` que usa todo
  `#[tokio::test]`, una tarea `spawn`-eada solo corre cuando el test cede, y el `sleep` era el único
  punto donde cedía. Es decir, el sleep no daba margen: se lo daba a una invalidación pendiente para
  colarse justo antes del assert. **Los 25 sleeps de la suite de integración han desaparecido.**
- **Los 15 asserts «esto NO debe invalidar» ahora prueban algo**. Un sleep fijo no puede demostrar
  una ausencia; con la invalidación esperada, son exactos. Verificado con mutantes: invalidar en
  modo A donde no toca los tumba, y quitar la invalidación del PATCH tumba los positivos.
- **El test de cache dejó de usar cronómetro**. `projection_series_caches_repeated_gets` comparaba
  `hit*2 < miss` y era el test más flaky del repo — con un household de un activo el miss ya baja a
  ~13 ms. Además tenía una rama de escape (`hit <= 5 ms`) por la que pasaba casi siempre, así que ni
  medía lo que decía. Lo sustituye `projection_series_serves_the_second_get_from_the_cache`, que
  **envenena la entrada cacheada con un centinela** y comprueba que el segundo GET lo devuelve:
  prueba binaria de que el read path leyó de la cache, sin reloj.
- El **warm-up post-login sigue en `tokio::spawn`** (D7: el login no espera al recompute). Es el
  único background que queda tocando la cache, y los tests que asertan sobre su contenido usan ahora
  `TestApp::settle_login_warmup`, una espera **por evento** y no un margen a ojo. Ese warm-up era la
  causa real de que `simulate_never_touches_the_projection_cache` fallara culpando a `simulate`.
- Helpers de test deduplicados: `warm`/`present`/`assert_invalidated`/`household_key` (2 copias
  idénticas + 1 inline) e `installation_id` (**4** copias) pasan a métodos de `TestApp`.

### Added — `GET /v1/allocation-rules/resolution` y tool `get_allocation_resolution` (tool 50)

- **El hueco que cerraba el issue #4**: no había forma de auditar la cascada desde fuera. Con la
  aportación del mes 1 sin explicar y `list_allocation_rules` devolviendo solo la *configuración*,
  un lector razonable concluía que la cascada repartía de más. No lo hacía.
- **La respuesta desglosa, no simplifica**: `base_cash` (lo que se reparte de verdad) separado en
  `recurring_net` y `planning_component`, con el flag `base_includes_transient`. Un flag de
  «sobreasignación» a secas habría dicho «sí» y habría sido igual de engañoso: el problema nunca fue
  el reparto, sino que la base incluye un término que se agota en 90 días.
- **Por regla**: `amount_intent` vs `amount_resolved` — si difieren sin `skipped_reason`, la regla
  fue **recortada** por el cap, que no es lo mismo que saltada y es la pregunta más frecuente —,
  `cap_ceiling`/`cap_room` y `skipped_reason`. Las reglas posteriores al corte por caja se emiten
  con `not_reached` en vez de desaparecer: `no_cash` («no te sobra dinero») y `not_reached` («las
  reglas de arriba se lo comieron») tienen remedios distintos.
- **Endpoint nuevo, no envelope** sobre `list_allocation_rules`: convertir aquel array en un objeto
  habría sido breaking. No pasa por la cache de proyección.
- Los ids de regla viajan desde el constructor del `ProjectionInput` (`allocation_rule_ids`,
  alineado posición a posición) porque el constructor **descarta** las reglas cuyo activo destino
  queda fuera del scope: re-derivar la alineación en el handler habría sido un cruce silencioso
  esperando a pasar.

### Added — `contribution_recurring_monthly` en `/v1/assets`: el número que sí es mensual

- **El defecto de contrato** (lo que el issue #4 etiquetó como `bug`, y lo es):
  `contribution_nominal_monthly` **no es mensual**. Es la cascada del primer mes e incluye el tramo
  transitorio de los planning flows sin fecha, así que **baja cada día** y **salta hacia arriba el
  día 1 de cada mes**. El doc-comment interno decía «aporte estimado del primer mes»; el nombre
  público y la descripción de la tool decían «aportación mensual objetivo». Un lector razonable que
  lo compare con `net_monthly_equivalent` concluye que la cascada reparte de más — que es
  exactamente lo que pasó, y no era cierto.
- **La solución no es renombrar** (rompería a los clientes) sino **decir la verdad y dar el número
  bueno**: se añade `contribution_recurring_monthly`, la misma cascada evaluada sobre el neto
  recurrente (`income − expense − debt_service`, sin el tramo de planning). Estable, reproducible y
  el único con el que tiene sentido hacer aritmética. Se calcula con una segunda pasada del engine
  sobre el mismo input con el ajuste de planning a cero: reutilizar la cascada en vez de aproximarla
  garantiza caps y precedencia idénticos, y no cuesta ningún SELECT extra.
- **Descripciones corregidas**: `list_assets` fundía en una frase tres campos distintos (aporte del
  mes 1, aporte estable y **tope** en euros) y ahora los separa nombrando la trampa del día a día;
  `list_allocation_rules` decía que «list_assets muestra el resultado resuelto» y ahora aclara que
  ella es la configuración y el resultado vive en `get_allocation_resolution`.
- Errata de tipos corregida de paso: `AssetResponse.contribution_nominal_monthly` estaba declarado
  **opcional** en el frontend cuando el servidor lo envía siempre — el mismo patrón de deriva que
  causó el bug de `savings_source` en la v2.2.0.

### Changed — Engine: `FirstMonthAllocation` expone la resolución de la cascada (salida bit-idéntica)

- **De dónde viene**: el issue #4 traía un «posible bug» de sobreasignación de la cascada.
  Investigado: **no lo había**. `distribute_contributions` acota `take` tres veces (intención de la
  regla, hueco del cap, caja restante) y corta en seco al agotarse la caja — es imposible repartir
  más de lo que hay. Lo que sí había era un hueco de observabilidad que hacía imposible demostrarlo
  desde fuera: la función devolvía solo `per_asset` y **tiraba** tanto el `leftover` (que ya
  calculaba) como la base de la que salía.
- `first_month_allocation` devuelve ahora `per_asset`, `base_cash`, `recurring_net`,
  `planning_component`, `debt_service`, `leftover` y una traza por regla.
  `first_month_per_asset_contribution_nominals` queda como wrapper de un renglón, así que los **11
  call-sites de test del engine siguen verdes sin tocar una línea**.
- **La traza sale por un sumidero opcional** (`Option<&mut Vec<RuleOutcome>>`): el bucle de
  proyección pasa `None` y no paga nada — corre hasta 840 veces por request y nadie lee la traza
  ahí. Una sola implementación de la cascada: dos divergirían en silencio al primer cambio de caps,
  y una explicación que no coincide con lo que el motor hace es peor que no tener explicación.
- **`skipped_reason` distingue cuatro causas reales**, no dos: `NoCash` («no te sobra dinero»),
  `NotReached` («las reglas de arriba se lo comieron»), `CapFull` y `ZeroAmount`. Tienen remedios
  distintos y colapsarlas destruiría el diagnóstico. Las reglas posteriores al corte por caja **se
  emiten** con `NotReached` en vez de desaparecer. Y `amount_intent` vs `amount_resolved` separa
  «recortada por el cap» —que no es un salto, y es la pregunta más frecuente— de «saltada».
- **Evidencia de no-cambio**: con un household sembrado (ingreso 3000, gasto 1000, cuota 450, tres
  activos, cascada fijo-con-cap + porcentaje + sumidero y dos planning flows sin fecha), se
  capturaron `/v1/projection/series?months=840`, `/v1/assets` y `/v1/summary` **antes y después**
  del refactor: los 841 puntos y todas las cifras salen **idénticos** (solo cambian los UUID, por
  ser bases distintas). El escenario reproduce además el mecanismo del issue al céntimo: el aporte
  del mes 1 suma 1.743,33 € frente a los 1.550 € de neto recurrente, y la diferencia son los
  193,33 € del tramo de planning del día (1.450/90 × 12 días).

### Added — `PATCH /v1/transactions/batch` y tool `update_transactions` (tool 49)

- **El problema, medido**: recategorizar el desglose de una categoría cajón costó 16 llamadas casi
  idénticas a `update_transaction`. Y no solo round-trips: `patch_transaction_core` invalida la
  cache de proyección en toda escritura, así que en modo C **cada una de las 16 llamadas tiró la
  cache**.
- **La solución, deliberadamente estrecha**: el lote admite `kind`, `category_id`/`clear_category` y
  `notes`/`clear_notes`. **No** admite `amount`, `op_date`, `concept` ni `value_date` — y eso es lo
  que lo hace seguro: ninguno de los campos admitidos entra en la huella de dedup ni en el
  emparejado de transferencias, así que el lote no recomputa huellas, no rompe pares conciliados y
  no dispara el pase de auto-conciliación. El lote clasifica; para reescribir está el PATCH de uno
  en uno.
- **Todo o nada** en una única transacción, con la carga y el owner-guard **antes** de cualquier
  escritura: un id ajeno o inexistente ⇒ 404 nombrando hasta 5 culpables y cero filas tocadas. El
  test mete el id ajeno en la **posición 2 de 6** justo para que un fallo a mitad de escritura se
  vea. Un resultado parcial obligaría al llamante a reconciliar estado, que es lo que un lote viene
  a evitar.
- **Una sola invalidación COND** al final, fuera del bucle.
- Variante de error nueva `ApiError::NotFoundWith(String)`: un 404 que propaga mensaje, para que un
  lote de 200 ids no obligue a buscar a ciegas cuál falló. Solo nombra ids que el llamante ya envió.
- Tope 200 (no los 1000 de `create_batch`): aquí el llamante enumera los ids uno a uno, y 200 cubre
  el caso real sin convertir un error de cliente en una reescritura masiva.

### Added — Backfill de reglas de categorización (`apply_categorization_rule`, tool 48)

- **El problema**: crear una regla solo afectaba a imports futuros y la tool lo decía con
  honestidad, pero el trabajo se duplicaba — creabas la regla para el futuro y recategorizabas el
  pasado a mano igualmente. Desglosar una categoría cajón costó 16 llamadas casi idénticas.
- **La solución**: `POST /v1/transactions/rules/{id}/apply` + tool `apply_categorization_rule`, con
  `apply_to_existing` (`uncategorized` | `all`), `from_month` y preview/confirm. El eje también
  existe en el body de `POST /v1/transactions/rules` para el round-trip único de la SPA; el default
  es `none`, así que el contrato histórico no se mueve.
- **Precedencia completa, no la regla suelta**: el backfill evalúa el conjunto ENTERO de reglas y
  solo escribe las filas cuya ganadora es la invocada, de modo que el pasado queda como habría
  quedado importando hoy. Las filas donde la regla casa pero pierde se reportan en
  `matched_by_other_rule` en vez de desaparecer del informe.
- **El no-op invisible, delatado**: `match_rule` descarta las reglas cuyo `source` no coincide con
  el del movimiento, así que una regla aprendida de MyInvestor no toca movimientos manuales — sin
  error y sin aviso. El backfill respeta esa semántica (una regla nunca hace en diferido lo que no
  haría en vivo) pero **cuenta** esas filas en `skipped_by_source`: un `matched: 0` con
  `skipped_by_source > 0` no es «no hay nada que hacer».
- **Contrato de cache separado en dos rutas**: crear la regla sigue siendo NONE; aplicarla es
  **COND** y solo si escribe algo, porque cambiar el `kind` de filas históricas cambia
  `transactions_12m_avg`, que es input del engine en los modos B y C. `would_change_kind` aparece en
  el preview justo por eso. `applying_a_rule_invalidates_cond_but_creating_it_still_does_not`
  recorre los tres modos y los tres momentos (crear / preview / backfill); verificado que cae si el
  backfill deja de invalidar y también si invalida cuando no ha tocado ninguna fila.
- Las patas de transferencia conciliadas se excluyen (`skipped_reconciled`): están fuera de todos
  los agregados de flujo. No se recalculan huellas ni se toca la conciliación — ni `kind` ni
  `category_id` entran en la huella de dedup ni en el emparejado.
- **Omisión deliberada**: la tool `create_categorization_rule` no expone `apply_to_existing`. En el
  momento del preview la regla aún no existe, así que no habría nada que simular; y un `create_*`
  capaz de reescribir cientos de filas haría mentir a sus propias annotations, que es lo que el
  cliente MCP usa para decidir si pide permiso. Desde el chat: crear y luego aplicar, con un único
  gate de confirmación.

### Added — Búsqueda en `GET /v1/transactions` (concepto, importe y rango de fechas)

- **El problema, medido**: el listado admitía cuatro ejes (`month`, `kind`, `category_id`,
  `import_id`) y ninguna búsqueda. Localizar cinco cargos de Amazon desde el chat obligaba a
  traerse julio entero y junio entero: 419 bytes por movimiento × 93 movimientos ≈ 38 KB ≈ 10k
  tokens por mes, para quedarse con cinco filas.
- **La solución**: `concept_contains` (1–200), `min_amount`/`max_amount` y `date_from`/`date_to`,
  en `list_transactions_core` para que HTTP y MCP compartan validación y devuelvan los mismos 400.
  Aditivo puro: sin filtros, el comportamiento es el de siempre byte a byte.
- **El plegado de tildes se replica en SQL con `translate()`, no con `upper()`**. `upper()` depende
  de la collation del cluster —bajo `C` no toca los no-ASCII— y esta imagen ya cambió de collation
  una vez (musl → glibc, con REINDEX de adopción en el entrypoint). Al meter también `a-z → A-Z` en
  la misma tabla, la expresión deja de depender de la collation y equivale carácter a carácter a
  `fold_diacritics_upper ∘ normalize_concept`. Como el `concept` se guarda sin normalizar, la
  expresión colapsa además los runs de whitespace. `sql_fold_tables_mirror_the_rust_fold` pinnea las
  dos tablas en las dos direcciones: cada entrada coincide, y ningún carácter que Rust pliegue falta
  en la tabla SQL (barrido del latín extendido).
- **Comodines escapados** (`LIKE … ESCAPE '\'`): sin eso, buscar `%` devolvía el conjunto entero.
- **Convenciones explícitas, porque son las que un cliente falla**: los importes se comparan **con
  signo** (`max_amount: "-50"` = gastos de 50 € o más) y las fechas son **inclusivas** en los dos
  extremos. `month` y `date_from`/`date_to` son **excluyentes** → 400: dos formas de decir lo mismo
  sin ganador implícito. Las bandas invertidas también son 400, no un conjunto vacío silencioso.
- **La paginación en SQL sigue intacta**: el `COUNT(*)` comparte los filtros nuevos, así que
  `truncated` no miente al buscar. Los filtros se agruparon en un `TxnFilters` porque el core ya
  tomaba diez parámetros posicionales y quince habrían sido terreno de cruces que el compilador no
  ve; `all_filters_combined_agree_with_each_axis` ejercita seis ejes a la vez y cae si el orden de
  los binds se desincroniza del de los placeholders.

### Added — `simulate_projection` devuelve la salud financiera del mes 1

- **El problema**: `SimKpis` devolvía cinco cosas (`jubilacion_month_index`, `final_net_worth`,
  `fire_target_base`, `runway_months`, `runway_is_indefinite`). Ni gasto, ni ahorro, ni tasa de
  ahorro. Para valorar un what-if desde el chat había que calcular el impacto sobre el gasto **a
  mano** — y ahí es donde se coló un doble conteo de una cuota de pasivo en la sesión que originó
  el issue #4.
- **La solución**: cada lado (baseline y escenario) añade `income_monthly`,
  `expense_total_monthly`, `debt_service_monthly`, `net_monthly` y `savings_rate`, con sus cuatro
  deltas. **Coste cero**: son valores que ya estaban calculados en el `ProjectionInput` de cada
  lado; lo único que faltaba era serializarlos.
- **Las definiciones no son las ingenuas, y esa es la parte que importa**: `expense_total_monthly`
  es `expense_regular_monthly + debt_service_monthly`, la misma base que alimentan el runway y el
  target FIRE. En modo A la cuota de pasivo vive **fuera** de `expense_regular_monthly` por diseño
  (fundirla ahí la contaría dos veces en toda la proyección, en silencio) y entra por el servicio
  de deuda: solo la suma cuadra con `/v1/summary` en los tres modos. Y `net_monthly` es
  `income − expense_total`, que **no** es el `net_cash_month` que reparte la cascada — ese lleva
  además el tramo de planning flows del mes en curso.
- **Pinneado entre superficies**: `sim_kpis_match_summary_financial_health_in_all_three_modes`
  compara los KPIs sin overrides contra el `financial_health` de `GET /v1/summary` en los tres
  modos, con un pasivo activo de 400 €/mes. Definir el gasto como `expense_regular_monthly` a
  secas hace fallar el modo A por exactamente esos 400 €, no por un epsilon.
- `savings_rate` se sirve con los mismos 6 decimales que `/v1/summary`, y `savings_rate_delta` se
  recalcula desde los componentes exactos en vez de restar dos ratios ya redondeados.

### Changed — Precisión de salida de los ratios (`/v1/summary`, `/v1/assets`)

- **El problema**: `rust_decimal` produce hasta 28 dígitos significativos en cada división y
  `serde::str` los serializaba enteros. Una sola respuesta de `GET /v1/summary` traía
  `"savings_rate": "0.2435991666666666666666666667"`, `"debt_to_assets_ratio":
  "0.0393680052666227781435154707"` y `"runway_months": "6.768981939754142082836931204"`. Además
  había una **incoherencia entre superficies**: el mismo `runway_months` salía con 1 decimal por
  `simulate_projection` y con 28 por `/v1/summary`.
- **La solución**: redondeo **en las cores** (nunca en la capa MCP, que devuelve la struct del
  endpoint intacta). `savings_rate`, `savings_rate_excluding_derived_debt`,
  `upcoming_coverage_ratio` y `debt_to_assets_ratio` a **6 decimales** de fracción — 4 decimales de
  porcentaje, `0,0001 %` de resolución, muy por encima del único decimal que pinta la UI.
  `runway_months` a **1 decimal**, alineado con `sim_kpis`. `contribution_nominal_monthly` de
  `/v1/assets` a **4 decimales** (política monetaria de la casa).
- **Es presentación, no semántica**: el gross-up, el umbral SWR y el propio runway se siguen
  calculando con la precisión completa; solo se recorta el valor publicado. Ninguna cifra derivada
  se mueve. Los dos `savings_rate` comparten `dp` a propósito: desde 3.7.0 son idénticos por
  construcción y el frontend se apoya en esa igualdad para decidir si pinta el paréntesis.
- **El invariante del runway sigue vivo, matizado**: la reducción exacta a `A/g` es una propiedad
  del **engine** (`liquid_runway_months` no redondea). Las dos aserciones de frontera de
  `summary_runway.rs` pasan a comparar contra `(A/g).round_dp(1)` — el mismo rigor, a la precisión
  que se publica.
- **Borde de contrato, documentado**: con 1 decimal, un runway inferior a `0,05` meses serializa
  `"0.0"`. El guard de la tarjeta de Runway (`SummaryView`) miraba **cero**, así que la tarjeta
  habría desaparecido justo en el escenario donde el dato más importa (líquidos casi nulos con
  gasto alto). Ahora mira **ausencia** (`isAbsentMetric`): el servidor omite el campo cuando no hay
  dato — rama indefinida o sin base de gasto — y un cero explícito es información, no falta de ella.
  El borde equivalente de `isZeroFractionMetric` (un ratio inferior a `5e-7` pasa a leerse como
  cero) queda anotado y sin cambio: una tasa de ahorro de 0,00005 % no es un caso real.

## [3.7.0] - 2026-08-19

### Changed — La cuota del pasivo es una partida más del presupuesto (**API breaking** de `/v1/budget`)

- **El problema**: `GET /v1/budget` servía las cuotas de los pasivos en un bloque aparte
  (`derived_from_liabilities`) que se sumaba por debajo del presupuesto en
  `totals.expense_derived_monthly_equivalent`. Desde la 3.4.0 el pasivo ya declara su
  **categoría de gasto** (`expense_category_id`) y la comparativa de Movimientos empareja ahí su
  recibo real, así que el bloque había dejado de tener razón de ser: era el único sitio donde la
  cuota se leía como «algo que se añade al presupuesto» en vez de como gasto presupuestado. Y
  arrastraba una incoherencia visible desde fuera — `expense_total_monthly_equivalent` existía en
  `/v1/budget` y en `/v1/summary` midiendo cosas distintas (en los modos B/C el de summary es el
  gasto real promedio, mientras el de budget sumaba una componente derivada que summary ya no
  usaba), sin forma de saber cuál era la buena.
- **La solución (formulación del owner)**: el bloque derivado **deja de existir como concepto de
  flujo**. Las cuotas de los pasivos activos entran en `entries` como una partida de gasto más,
  atribuida a la categoría de gasto que declara el pasivo, **no editable** para no confundirse con
  la partida que el usuario presupueste en esa misma categoría. Presupuesto y realidad siguen sin
  cuadrar — eso es la información de valor, no un bug — pero ya no hay dos conceptos de gasto.
- **Contrato nuevo de `GET /v1/budget`**: cada `entries[]` trae `source` (`"manual"` |
  `"liability"`); una cuota añade `liability_id` y `label`, su `id` es el del pasivo, su
  `category_id` es el `expense_category_id` del pasivo, su `amount` es el **equivalente mensual**
  del plan (`weekly` → ×52/12) y su `expense_end_date` es el fin del plan. `PATCH`/`DELETE
  /v1/budget/entries/{id}` sobre una cuota devuelven 404: se editan con `PATCH /v1/liabilities/{id}`.
- **Breaking**: se retiran `derived_from_liabilities` y `totals.expense_derived_monthly_equivalent`
  de `/v1/budget`, y `entries[].category_id` pasa a **opcional**. `expense_regular_monthly_equivalent`
  absorbe las cuotas (es ya la suma exacta de los `entries` de gasto) y
  `expense_total_monthly_equivalent` vale lo mismo. **Ninguna cifra de cabecera se mueve**: el
  gasto total y el neto del presupuesto son los de siempre — la fusión reparte, no suma.
- **Pasivos sin categoría de cuota asignada** (anteriores a la 3.4.0, y los que importa un
  `.ffbackup` viejo): su partida **sigue existiendo y sigue sumando**, omitiendo `category_id` y
  marcada «Sin categoría de cuota» en la UI, al final de la lista. Descartarlas habría bajado el
  gasto presupuestado en silencio — el modo de fallo caro de este repo.
- **El engine NO cambia** (cero diffs en `crates/engine`, cero en la base de gasto de la
  proyección). `ledger_regular_monthly_income_and_expense` sigue devolviendo solo lo persistido:
  el engine cobra la cuota por su lado (`ProjectionLiabilityInput::monthly_payment`, con
  amortización y fecha fin), así que fundirla también ahí la contaría dos veces en todo el
  horizonte del modo A, en silencio. Clavado con cifras predichas a mano por
  `liability_quota_stays_out_of_the_engine_expense_base` (`monthly_delta_assumption` = 3.000 −
  1.000 = **2.000**, no 1.800; y NW(12) = 2.000·12 − 100.000 = **−76.000**, la cuota cobrada una
  sola vez).
- `expense_retirement_monthly_equivalent` tampoco recibe la cuota: termina con su plan de pago, así
  que no es gasto post-jubilación. Es el campo que alimenta la previa FIRE de `RetirementView`, con
  incidente propio (v1.3.0, divergencia 2–3×).

### Changed — `/v1/summary`: tres campos quedan degenerados (contrato intacto)

- `expense_derived_monthly_equivalent` pasa a ser **0 en los tres modos** (antes ya lo era en B/C
  por la reforma 3.4.0; ahora también en A, porque la cuota vive dentro del gasto del presupuesto),
  y `monthly_net_excluding_derived_debt` / `savings_rate_excluding_derived_debt` pasan a ser
  **idénticos** a `net_monthly_equivalent` / `savings_rate`: ya no queda deuda derivada que
  excluir. Los tres se **mantienen** en el JSON por compatibilidad — no son breaking.
- Ninguna cifra de cabecera se mueve en ningún modo: en A el gasto total sigue siendo el mismo
  (cambia solo su reparto entre `expense_regular` y `expense_derived`) y en B/C nada cambia. La UI
  del Resumen se adapta sola: el paréntesis «excluyendo deuda derivada» de las KPIs de ahorro solo
  se pinta cuando los dos valores difieren, así que se apaga sin tocar una línea de frontend.

### Changed — Presupuesto (UI)

- Desaparece el panel «Derivado de pasivos». Las cuotas salen dentro de la tabla de **Gastos**,
  cada una como **segunda línea de su categoría** (el orden coloca la partida manual primero y la
  cuota justo detrás), con el distintivo «Cuota · <pasivo>», sin acciones de editar/borrar y sin
  ser pulsables en móvil. Nota al pie cuando hay alguna, apuntando a Pasivos para editarlas.
- Incidental, en la misma tabla: la cabecera de Gastos pintaba la columna de acciones en móvil
  aunque el cuerpo no emite esa celda (columna de más sin nada debajo). Ahora usa la misma
  condición que la tabla de Ingresos.
- El presupuesto deja de pedir `/v1/categories?scope=liability`: solo lo usaba el bloque retirado
  (una petición menos por carga de la pestaña).

### MCP

- **Tools actualizadas** (evaluación de paridad de `futurefin-mcp-parity` §1 → «tool actualizada»;
  el catálogo sigue en **47**, sin altas ni bajas): `get_budget` hereda el contrato nuevo por la
  core compartida, y su descripción —que hablaba de «cuotas derivadas» como bloque— pasa a
  explicar `source` y que los totales ya incluyen las cuotas. `list_liabilities` menciona que la
  cuota aparece además como partida en `get_budget`.

### Tests

- `budget_derived.rs` → **`budget_liability_quotas.rs`** (5 → 10 tests): forma de la partida y
  convivencia con la manual de la misma categoría, totales sin componente derivada, la cuota fuera
  de `expense_retirement_*`, **la cuota fuera de la base de gasto del engine**, el predicado de
  pasivo activo intacto (fecha fin NULL, vencido, borde `>=`, sin plan), semanal ×52/12, scoping, y
  la cuota sin `expense_category_id` que sigue sumando.
- `summary_runway.rs`: actualizado el reparto de modo A (regular 1.000 → 1.200, derived 200 → 0)
  con el total y el runway sin moverse — la evidencia de que la fusión no suma.
- Nuevo `apps/web/src/lib/ledger.test.ts` (4 tests): el orden que coloca la cuota detrás de la
  partida manual de su categoría, incluso cuando su importe es mayor, y manda al final las cuotas
  sin categoría.

## [3.6.0] - 2026-08-19

### MCP — Disciplina de paridad con la API HTTP + cierre de la asimetría CRUD del ledger

- **El problema (deriva silenciosa, no un bug)**: el catálogo `/mcp` es una superficie
  **derivada** de la API HTTP, pero nada obligaba a mantenerlo al día. Un endpoint nuevo podía
  mergear sin que nadie se preguntara si merecía tool: no falla ningún test, `tools/list` sigue
  pareciendo completo a cualquier tamaño, y el cliente MCP simplemente nunca se entera de que la
  funcionalidad existe. La norma que sí existía (architecture-contract D14, «las tools llaman a
  las mismas core fns») gobierna **cómo** se implementa una tool ya decidida — jamás **si** debe
  existir. La auditoría ruta a ruta lo confirmó: ~83 % de cobertura sobre las rutas de datos
  financieros, con los huecos concentrados exactamente en los handlers que aún no tenían `*_core`
  extraída (la deuda era de la capa handler, no de la capa MCP).
- **La norma nueva**: todo cambio de la superficie de API debe terminar en **exactamente uno** de
  tres desenlaces — tool añadida/actualizada, omisión deliberada registrada, o n/a — nunca en
  silencio. Vive en la skill nueva
  [`futurefin-mcp-parity`](.claude/skills/futurefin-mcp-parity/SKILL.md), que posee el proceso y
  el juicio (rúbrica de pertinencia, registro de omisiones deliberadas y de gaps pendientes,
  recipe paso a paso de añadir/actualizar una tool, contadores reproducibles) sin duplicar
  ningún fact con dueño: D14 sigue siendo el porqué, `.claude/api-routes.md` §MCP el catálogo.
  Anclada en los tres choke points por los que se pasa de verdad: `futurefin-change-control` §1
  (clase «API contract») y su checklist pre-merge, `.claude/adding-handler.md` (paso 7 nuevo, más
  el split extractor+`*_core` en Key patterns como prerrequisito) y la tabla de routing de
  CLAUDE.md.
- **`update_liability` (tool nueva)** — la asimetría que la auditoría destapó: el catálogo tenía
  `create_liability` y `delete_liability` pero **no** editar. Ante «el TIN de mi hipoteca ha
  bajado al 2,1 %» el agente solo tenía un camino: borrar y recrear, que pone a NULL el
  `linked_liability_id` de todos los movimientos asociados y pierde el `expense_category_id` de
  la cuota (3.4.0). Una tool ausente no es neutral: **empuja hacia la alternativa destructiva**.
  Se extrae `patch_liability_core` del handler `PATCH /v1/liabilities/{id}` (merge campo a campo,
  re-derivación del principal si `derive_principal_from_plan` sigue activo, invalidación FULL
  dentro) y la tool la reutiliza — cero SQL nuevo.
- **`update_asset` (tool nueva)**: `patch_asset_core` ya aceptaba el body completo, pero
  `update_asset_value` escribía `None` en seis campos, así que por MCP no se podía renombrar un
  activo, recategorizarlo ni marcarlo (i)líquido — y `is_liquid` no es cosmético: gobierna el
  runway y el disparador SWR de `runway_is_indefinite`. `update_asset_value` se mantiene intacta
  como subset de valoración (su descripción ahora remite a la hermana). El tri-state
  omitir/null del PATCH, que un JSON Schema no puede expresar, se modela con
  `clear_purchase_price` (precedentes: `clear_cap`, `clear_due_date`).
- **Catálogo 45 → 47 tools**, ambas FULL y sin preview/confirm (editar campos no destruye filas).
  Regresión: `update_asset_and_update_liability_share_cores_and_invalidate_full` en
  `apps/api/tests/mcp_write.rs` (17 tests) cubre el cuarteto que este repo exige a toda tool de
  escritura — core compartida (la fila editada por MCP es indistinguible por HTTP), contrato de
  cache, error de dominio compartido y el toggle `mcp_write_enabled` cortándolas en vivo — más
  el catálogo congelado en `mcp_http.rs`.
- **Deriva documental corregida de paso** (todos los contadores congelados sin fecha de esta
  biblioteca han estado mal al menos una vez — docs-and-writing §7): `.claude/api-routes.md`
  decía «Tools de lectura (20)» sobre una enumeración de 10; `.claude/tests.md` decía que
  `mcp_http.rs` congela «el catálogo de 19 tools de lectura» cuando congela las 47 completas; y
  `futurefin-validation-and-qa` arrastraba cuatro filas ausentes (`mcp_write.rs`,
  `mcp_simulate.rs`, `transactions_reconcile.rs`, `budget_derived.rs`), una fila de `mcp_http.rs`
  que aún describía «the 10-tool catalog» y todos los totales una release atrás (206/23 → 284/27,
  36 → 40 migraciones, engine 56 → 61, Vitest 309/12 → 321/13, `lib/navigation.test.ts` sin
  documentar en ninguno de los dos sitios). Su tabla lleva ahora una nota permanente de que
  `tests.md` gana en caso de desacuerdo. Los diez contadores por fichero de la tabla suman ahora
  exactamente los 284 del código (`grep -c '#\[tokio::test\]' apps/api/tests/*.rs`).
### Fixed — `restore-postgres.sh` abortaba en macOS antes de tocar la base

- **Encontrado ejecutando el drill C de la release** (restore real, exigido por llevar migración
  nueva): `scripts/restore-postgres.sh` moría en el paso 1/6 con `SERVICE…: unbound variable`. La
  causa: `"$SERVICE…"` pega el carácter `…` (multibyte) al nombre de la variable y el **bash 3.2
  que trae macOS** se traga esos bytes dentro del identificador; con `set -u` eso es un aborto. En
  Linux (bash 5) funcionaba, así que el fallo solo aparecía restaurando desde el Mac — justo el
  escenario de «me llevo el backup a casa». Arreglado con `${SERVICE}…` / `${POSTGRES_DB}…`.
- **`backup-postgres.sh`**: `mapfile` también es de bash 4+, así que en macOS la retención fallaba
  con `mapfile: command not found` **después** de escribir el backup — el dump salía bien y los
  backups viejos se acumulaban en silencio. Sustituido por un `while read` portable.
- Sin cambios de comportamiento en Linux. `shellcheck -S warning` limpio sobre entrypoint y
  scripts; drill C re-ejecutado de punta a punta: censo antes = censo después (2 transacciones,
  1 rechazo, 1 usuario, 40 migraciones) y el stack vuelve a `/v1/ready`.

- **Nota de publicación**: 3.5.0 se cerró en este CHANGELOG pero nunca llegó a tener tag ni
  imagen — las dos tools llegaron antes de publicarla. 3.6.0 es por tanto la primera imagen del
  tren y **contiene también toda la 3.5.0** (conciliación de transferencias incluida); no existe
  ni existirá un `maxlainz/futurefin:3.5.0`. Bump minor, no patch: dos tools nuevas son
  superficie pública aditiva.

## [3.5.0] - 2026-08-19

> Versión **no publicada como imagen**: sus cambios se distribuyen dentro de 3.6.0 (ver arriba).

### Added — Conciliación de transferencias: nada se descarta, solo se oculta lo conciliado (arregla el gasto que desaparecía)

- **El bug (reporte del owner)**: las transferencias se ELIMINABAN del gasto aunque fueran gasto
  real. La raíz: la marca «transferencia» solo existía como sugerencia efímera del preview del
  import (`suggested_transfer`, heurística por tokens `TRANSFERENCIA/TRASPASO/AHORRO/…` o par
  opuesto dentro del MISMO CSV), la UI desmarcaba esas filas por defecto y el confirm las
  descartaba **antes del INSERT** — sin dejar huella. El alquiler pagado por transferencia, un
  envío a un tercero: fuera del gasto, en silencio, y re-ofrecidos como «nuevos» en cada
  re-import. No existía conciliación alguna entre extractos distintos.
- **El modelo nuevo (conciliar ≠ borrar)**: todas las filas se importan con su kind natural. Un
  movimiento solo deja de contar como gasto/ingreso cuando está **CONCILIADO** con su
  contrapartida — la otra pata del mismo traspaso, normalmente de otro extracto. El **pase
  automático** (determinista, punto fijo) empareja importes exactamente opuestos, misma divisa,
  mismo usuario, a **≤5 días**, cruzando TODA la base tras cada mutación y tras cada import
  (`ImportConfirmResponse.reconciled_pairs`); también bajo demanda con
  `POST /v1/transactions/reconcile` (botón «Conciliar ahora»). Conciliado ⇒ **visible** en
  Movimientos con badge «Conciliada» (contrapartida en el tooltip), **excluido** de: totales del
  mes, comparativa por categoría, `MIN(op_date)` de la ventana «Todo», serie por categoría,
  promedio real 12m del engine (modos B/C — numerador Y denominador: un mes solo-conciliadas es
  un mes vacío) y `months[]` del cashflow.
- **Asimetría deliberada del cashflow**: la curva fina **SÍ** cuenta las conciliadas — modela el
  SALDO de cada cuenta y un traspaso interno mueve saldo real; excluirlas haría divergir la curva
  de los snapshots anclados (test `reconciled_excluded_from_months_but_not_from_fine_curve`).
- **Desconciliar como válvula**: un falso positivo (p. ej. un reembolso que casualmente cuadra a
  ≤5 días — esperado y documentado) se rompe con «Desconciliar» (modal de edición /
  `DELETE /v1/transactions/{id}/reconcile`) y queda **rechazado**: el pase automático no lo
  resucita (`transfer_match_rejections`). Un PATCH que cambia importe/fecha rompe el par SIN
  rechazo (revertir re-empareja); borrar una pata desconcilia la otra
  (`transfer_counterpart_id`, self-FK `ON DELETE SET NULL` — migración
  `20260819120000_transactions_transfer_reconciliation.sql`, aditiva pura). Conciliación manual
  de un par por API (`POST /v1/transactions/{id}/reconcile`): sin ventana de fecha, pero exige
  importes exactamente opuestos — conciliar jamás altera el neto del hogar.
- **Vía retroactiva**: las filas descartadas en su día no dejaron huella → **re-importar los
  CSVs antiguos** las recupera (las ya importadas se detectan como duplicadas) y el pase las
  concilia contra lo que ya haya; «Conciliar ahora» cubre los pares que ya estuvieran metidos a
  mano. Letra pequeña de la pata `savings`: si importas AMBAS cuentas de una aportación, sus dos
  patas se concilian y `savings_actual` baja — la subida del activo sigue visible por snapshots
  y curva fina; desconcilia si prefieres contarla.
- **Cambio de comportamiento del import (no de contrato)**: las filas sugeridas como
  transferencia llegan **marcadas** al preview (el hint «Transferencia» se mantiene, ya
  informativo) y el descarte explícito sigue disponible. `suggested_transfer` no cambia de forma.
- **MCP**: tools nuevas `reconcile_transfers` y `unreconcile_transfer` (catálogo 43 → **45**),
  mismas cores que HTTP; `list_transactions` expone los campos de conciliación.
- **Backup `.ffbackup` v7 → v8**: las transacciones llevan su par por índice
  (`transfer_counterpart_index`, simétrico) y el payload añade `transfer_match_rejections` — sin
  exportar los rechazos, un restore los resucitaría todos en el primer pase. Import en tres
  pasadas (la self-FK puede apuntar hacia delante); un backup ≤v7 importa intacto y el pase
  post-import re-concilia retroactivamente sus pares. Cadena v1→…→v8 completa.
- **Cache de proyección**: conciliar/desconciliar cambia qué cuenta en el promedio → invalida
  COND (solo modos B/C); un pase sin pares nuevos no tira la cache caliente. Regresión ampliada
  en `transactions_projection_cache.rs`.
- **Tests**: suite nueva `transactions_reconcile.rs` (19) + regresión en import/summary/
  savings_source/cashflow/backup/MCP con números predichos antes de ejecutar (p. ej. modo B:
  delta 2000 conciliado → 1000 al desconciliar; curva fina 1187.5/750 idéntica con y sin
  conciliar).

## [3.4.0] - 2026-08-18

### Added — Categoría de gasto de la cuota: el pasivo publica su cuota en el presupuesto (arregla el Δ rojo de Movimientos)

- **El problema**: el recibo real de la hipoteca cuenta como gasto del mes en su categoría, pero
  el presupuesto por categorías no contenía la cuota (la derivada vivía en un panel aparte, sin
  categoría de gasto, y la comparativa la excluía desde v1.8.0 para no contarla dos veces) →
  `Real − Presupuesto` con lados desiguales → **siempre +cuota en rojo**, aunque gastases clavado.
- **La solución (formulación del owner)**: los movimientos no se tocan — ya llevan categoría. Es
  el pasivo el que declara **`expense_category_id`** (categoría de GASTO de su cuota) y su
  equivalente mensual entra en el lado Budget de ESA categoría, en Presupuesto y en la comparativa
  → `Hipoteca · Real 512 · Budget 500 · Δ +12` (Δ informativo por fin: revisión de tipo,
  amortización extra). El lado Real cae solo: las reglas de import ya aprenden categorías.
- **Obligatoria al crear** un pasivo desde ahora (HTTP y tool MCP `create_liability` — **API
  breaking interno** del create); en PATCH es set-only. **Los pasivos existentes quedan `NULL`**
  («sin asignar», comportamiento previo intacto — cero breaking de números) y se asignan desde el
  formulario de Pasivos (marcador «sin categoría de cuota» en la tabla). Migración
  `20260818150000_liabilities_expense_category.sql` (columna nullable, FK `ON DELETE SET NULL`;
  el remap de categorías de gasto la arrastra; no bloquea borrados).
- La atribución en la comparativa es **month-aware** (pasivo activo en el mes seleccionado) y una
  categoría solo-cuota materializa su fila (budget = plan, actual = 0). Letra pequeña documentada:
  presupuestar a mano la cuota en la misma categoría infla su Budget — visible en la fila y
  autocorregible, frente al doble conteo silencioso pre-v1.8.0.
- Backup `.ffbackup`: campo aditivo `expense_category_ref` (por `(scope, name)`, `#[serde(default)]`,
  **sin bump** de `CURRENT_SCHEMA_VERSION`); los backups viejos importan con `NULL`. Fix incluido:
  `fetch_categories_used` ahora también exporta la categoría usada solo por `expense_category_id`
  (sin él, esos backups no importaban). Engine, Resumen y modos B/C intactos.

### Fixed — Coherencia del predicado «pasivo activo» (**cambia números**)

- **La línea derivada del presupuesto ahora incluye los pasivos sin fecha fin** (`/v1/budget`,
  y con ella los totales del modo A en `/v1/summary`). La query era el único outlier del
  sistema: exigía `payment_end_date IS NOT NULL AND > today` mientras el resto usa
  `IS NULL OR >= today` (NULL = plan indefinido; el día exacto de fin aún cuenta). Consecuencia
  del bug: un pasivo con cuota y sin fecha fin no aparecía en «Derivado de pasivos» pero el
  engine SÍ cobraba su cuota → el modo A no contaba la cuota «una vez» de forma consistente y el
  KPI «ahorro real vs esperado» del Resumen descuadraba exactamente en esa cuota, para siempre.
  Quien tenga pasivos con plan y sin fecha fin verá subir `expense_derived`/`expense_total` (y
  bajar el net) — es la cifra correcta que faltaba.
- **La proyección filtra los pasivos vencidos** (fix C-10): `build_installation_projection_input`
  cargaba TODOS los pasivos del scope y el engine restaba su principal del net worth en cada mes
  del horizonte — `projection.starting_net_worth` divergía de `summary.net_worth` exactamente en
  el principal vencido, contra el contrato D5/I5 de la arquitectura. Pinned por
  `projection_excludes_expired_liability_principal`.
- Cobertura dedicada nueva de las líneas derivadas (`budget_derived.rs`, 5 tests: NULL end,
  vencido, borde `>=`, sin plan, semanal ×52/12, scoping) — hueco real detectado en la
  investigación: no existía ningún test directo de `derived_from_liabilities`.

### Changed — Modos B/C: promedio real crudo; los pasivos solo restan patrimonio (**breaking de números**)

- **Reforma de las cuotas de pasivo en los modos reales** (`savings_source ∈ {transactions_avg,
  budget_income_real_expense}`), decisión de producto del owner: el promedio de gasto 12m se usa
  **crudo** — las cuotas pagadas ya viven dentro de los movimientos (amortización incluida) — y
  los pasivos **no tocan la caja de la simulación**: su principal pendiente resta el patrimonio
  como **constante** en todo el horizonte (sin cargo mensual, sin amortización proyectada, sin
  escalón al vencer el plan). El modo A (presupuesto) no cambia: budget + cuota derivada
  time-limited, contada exactamente una vez.
- Se elimina la **resta híbrida** (`effective_avg_income_expense` + `per_liability_linked_avg`):
  su corrección dependía de un vínculo (`linked_liability_id`) manual, invisible y que las reglas
  no aprenden, y su clamp `max(0, …)` podía tragarse gasto ajeno con histórico parcial.
  `linked_liability_id` queda como metadata (sin consumidor numérico).
- **Por qué cambian los números en B/C** (ejemplo: hipoteca 500 €/mes nominal, cuota real media
  450 € dentro del gasto): antes `net = income − (gasto − 450) − 500`; ahora
  `net = income − gasto`. La fecha FIRE proyectada es **conservadora** cuando un préstamo vence
  dentro del horizonte (la cuota sigue pesando en el promedio y el equity amortizado no aflora);
  a cambio desaparecen la dependencia del vínculo y el descuadre del KPI «real vs esperado». La
  realidad entra en cada recomputación (promedio y principal actualizados).
- `GET /v1/summary` en B/C: `expense_derived_monthly_equivalent` pasa a **0**,
  `expense_total_monthly_equivalent = expense_avg`, `net = income − expense_avg`; las identidades
  `expense_total = expense_reg + expense_der` y `net = income − expense_total` siguen valiendo en
  los tres modos. El runway usa el gasto crudo (burn rate real, cuotas incluidas). Tipos y
  nullabilidad del API intactos — **breaking de números**, no de contrato.
- El engine **no cambia** (cero diffs en `crates/engine`): el handler anula `monthly_payment` en
  memoria en los modos reales y el engine, que ya restaba el principal de todo pasivo de entrada,
  produce la resta constante por sí solo.
- Tests: `mode_b_raw_avg_ignores_liability_links`, `mode_b_liability_static_nw_subtraction`
  (NW(k) = k·delta − principal en toda la serie), `mode_b_no_step_up_at_liability_end` (pin del
  coste aceptado), espejos de summary/runway actualizados con números predichos a mano.

## [3.3.0] - 2026-08-18

### Added — MCP: tools de escritura, tramo 1 (el MCP deja de ser solo lectura)

- Primeras 8 tools de escritura: `create_transaction` («apunta 23,50 € de cena de ayer», con
  `recurring` opcional que crea la plantilla y backfillea meses cerrados), `update_transaction`
  (recategorizar/corregir, owner-guard → not_found), `capture_snapshot` («guarda una foto de mi
  patrimonio hoy», upsert por día civil que sobrescribe), `materialize_recurring` (idempotente
  por cursor), `create_planning_flow`/`update_planning_flow` («en octubre pago 800 € de IRPF»),
  `create_category` y `create_categorization_rule` («todo lo de MERCADONA es supermercado», solo
  imports futuros).
- **Cero deriva con HTTP**: cada tool llama a la misma core fn de mutación que su handler
  (extraídas en este cambio: `create_transaction_core`, `patch_transaction_core`,
  `capture_snapshots_core`, `materialize_recurring_core`, `create_planning_flow_core`,
  `patch_planning_flow_core`, `create_category_core`, `create_categorization_rule_core`), y la
  invalidación de cache vive DENTRO de la core — el contrato FULL/COND/NONE no puede divergir
  entre caminos (regresión por el camino MCP en `mcp_write.rs`).
- Toda tool de escritura pasa por `require_mcp_write` (rol vivo + toggle) y devuelve una
  respuesta compacta `{id, resumen}`; annotations con `readOnlyHint=false` y hints de
  destructividad/idempotencia según la tabla del issue. `get_info` y las instrucciones del
  servidor ya no anuncian «solo lectura».

### Added — MCP: tools de escritura, tramo 2 (ledger y presupuesto)

- 7 tools más: `update_asset_value` («mi fondo vale ahora 52.300 €», subset deliberado con
  before/after), `create_asset`, `create_liability` (con `derive_principal_from_plan`),
  `create_budget_entry`/`update_budget_entry`, `update_allocation_rule` («aporta 200 € más al
  mes al fondo», subset amount/cap/enabled — la edición estructural de la cascada queda fuera de
  chat y la invariante del sink vive en la core compartida) y `delete_recurring_rule`, que
  estrena el patrón **preview/confirm**: sin `confirm: true` devuelve un preview como éxito
  (información para el LLM, no un fallo) y no toca nada.
- Cores de mutación extraídas: `create_asset_core`, `patch_asset_core`, `create_liability_core`,
  `create_budget_entry_core`, `patch_budget_entry_core`, `patch_allocation_rule_core` y
  `delete_recurring_rule_core` — como en el tramo 1, la invalidación (FULL, o NONE en la regla
  recurrente) vive dentro de la core.
- La capa API valida ahora `expected_annual_return_percent > −100` al crear/editar activos
  (HTTP y MCP): el engine clampa ≤ −100 a pérdida total, pero los inputs nuevos absurdos se
  rechazan con error tipado (misma cota que los overrides de `simulate_projection`).

### Added — MCP: tools de escritura, tramo 3 (destructivas + configuración FIRE) — issue #3 completo

- Los 7 deletes con **preview/confirm**: `delete_transaction` (preview = el movimiento completo),
  `delete_planning_flow`, `delete_budget_entry`, `delete_asset` y `delete_liability` (preview con
  los contadores de desvinculación — los movimientos vinculados quedan con el link a NULL, no se
  borran), `delete_snapshot` (`items_deleted`) y `delete_import` (`transactions_deleted`; borra el
  lote y sus transacciones en cascada, mismo contrato que el `?confirm=true` HTTP). Sin
  `confirm: true` ninguna toca nada.
- **`update_fire_settings`** — el mayor radio del catálogo, SOLO para el owner: SWR, inflación
  asumida, fuente del ahorro (A/B/C), modo del objetivo, importe manual, impuestos y tramos.
  Opera **campo a campo sobre el estado actual** vía `patch_fire_settings_core`: jamás
  deserializa a `FireSettings` (cuyo `#[serde(default)]` a nivel de struct resetearía los campos
  ausentes — el bug que un PATCH parcial por HTTP sí dispara), y sin confirm devuelve el
  `{before, after}` ya validado. Regresión explícita: cambiar solo `swr_pct` deja los
  `tax_brackets` personalizados intactos.
- Con esto el issue #3 queda completo: 23 tools de escritura + 20 de lectura/simulación = 43
  tools en el catálogo, todas sobre cores compartidas con HTTP y con el contrato de cache
  pinneado por tests en ambos caminos.

### Changed — Ajustes: pestaña dedicada «MCP» (y «Acceso» pasa a «Usuarios»)

- Todo lo relacionado con el servidor MCP vive ahora en una sub-tab propia de Ajustes: panel
  «Servidor MCP» (endpoint + explicación del modelo de permisos), el toggle owner-only
  **«Permitir escritura vía MCP»** (autosave, respaldado por `mcp_write_enabled`; los demás roles
  lo ven en solo lectura), los «Tokens de API (MCP)» y las «Conexiones» OAuth. La antigua
  «Acceso» queda como **«Usuarios»** (aprobar pendientes), visible solo para el owner; su slug
  `/ajustes/acceso` se conserva para URLs guardadas.
- El switch de la barra de Proyección se extrajo al componente compartido `components/Switch.tsx`
  (clases `.ff-switch*`, tokens del tema; `variant="chart"` mantiene el label small-caps) y lo
  reutiliza el toggle nuevo. Primer test de `lib/navigation.ts` (mapeo slug↔id de sub-tabs).
- Copy actualizado en tokens, conexiones y consentimiento OAuth: el acceso ya no promete «solo
  lectura» — hereda el rol vivo y respeta el interruptor de escritura.

### Added — Kill-switch de escritura MCP: `installation.mcp_write_enabled`

- Columna nueva (`20260818120000`, `BOOLEAN NOT NULL DEFAULT TRUE`) + campo en el snapshot y en el
  `PATCH /v1/installation` (owner-only, como todo el PATCH). Es un **ajuste en DB con toggle en la
  GUI** (Ajustes → MCP), no una env var: `require_mcp_write` (mcp/auth.rs) lo lee en vivo en cada
  llamada de escritura, así que apagarlo corta la escritura en el siguiente request — misma
  filosofía que el rol vivo. Con el toggle apagado las tools de escritura devuelven un error
  tipado `mcp_write_disabled` que el LLM puede explicar (viaja como `bad_request`, la única
  variante que propaga mensaje); un `viewer` recibe `forbidden`. `FUTUREFIN_MCP_ENABLED` sigue
  siendo el kill-switch de `/mcp` entero. Fuera del `.ffbackup` (settings de instalación, no
  datos financieros).

### Added — MCP: `simulate_projection` (what-if de proyección/FIRE sin persistir)

- La capacidad que faltaba para un asistente conversacional: «¿y si me compro X?», «¿y si gasto
  200 € más al mes?», «¿y si el SWR fuera 3?» sin tocar el estado guardado. Simula baseline y
  escenario con el mismo contexto (today, horizonte, inflación, fire_settings) y devuelve KPIs +
  deltas (mes de jubilación, patrimonio final, base del target FIRE, runway de líquidos con la
  fórmula de `/v1/summary`); series decimadas opt-in.
- Dos semánticas de gasto mensual con nombres distintos: `extra_monthly_expense` (gasto REAL —
  mueve el target FIRE y las bases de caps, aplicado dentro del ensamblado vía `SimOverrides`) y
  `extra_monthly_cash_adjustment` (NEUTRO — solo resta caja por el mecanismo planning-adjustment);
  `extra_monthly_savings` es el espejo positivo neutro. `one_off_expense` acepta `month_index` o
  `date` (mismo mapeo fecha→mes que un planning flow real). Los overrides de settings re-aplican
  las cotas del PATCH (`swr_pct` 0..4, inflación 0..50) y `asset_return_overrides` admite tasas
  negativas (> −100) gracias al fix del engine.
- **Cache-neutral por construcción**: nunca pasa por `projection_series_cached` (regresión:
  `mcp_simulate.rs`, 6 tests).

### Added — MCP: 9 tools de lectura nuevas + endpoint de serie mensual por categoría

- El catálogo pasa de 10 a 19 tools de lectura, cerrando los huecos de superficie del dominio:
  `list_allocation_rules` (la cascada como reglas — antes solo era visible su resultado resuelto
  por activo), `list_categories` (resolver nombre→id, prerrequisito de la escritura),
  `get_category_monthly_series`, `get_history_cashflow` (`window_months`, `include_curve` opt-in),
  `list_recurring_rules` y `list_categorization_rules` (own-user, sin `view`),
  `list_transaction_months`, `list_snapshots` (`include_items` opt-in) y
  `list_transaction_imports`; `list_transactions` gana el filtro `import_id` que el HTTP ya tenía.
  Cada tool llama a la misma fn `_core` que su endpoint (paridad byte a byte pinneada en tests).
- **Nuevo endpoint** `GET /v1/transactions/category-series` (`kind` expense|income,
  `category_id?`, `window_months` 1..=60 default 12): serie mensual **cero-rellena** por categoría
  con magnitudes ≥ 0 (Decimal-string, escala 2). El dato ya se materializaba en memoria para la
  comparativa; ningún endpoint lo emitía mes a mes. La tool `get_category_monthly_series` es su
  espejo exacto.

### Changed — MCP: annotations, verbosidad e identidad en las 10 tools existentes

- **Tool annotations en todo el catálogo** (`#[tool(annotations(...))]` de rmcp): `title` legible,
  `read_only_hint = true` y `open_world_hint = false` en las 10 tools. Sin ellas un cliente
  conforme al spec MCP debe asumir el peor caso y tratar cada lectura como escritura destructiva.
- **`get_history`** gana `window_months` (1..1200) e `include_asset_series` (default `false` en la
  tool): un backfill de años ya no vuelca toda la rejilla + un array por activo en cada llamada.
  Los mismos knobs llegan a `GET /v1/history/series` (aditivos; default `include_asset_series =
  true` — contrato REST intacto). La interpolación sigue anclándose en todos los snapshots; solo
  se recortan puntos y markers emitidos.
- **`list_transactions`** pagina **en SQL** (`LIMIT`/`OFFSET` + `COUNT(*)` para `total_count`,
  nuevo parámetro `offset`, filtro `import_id` que el HTTP ya tenía): la DB ya no materializa el
  conjunto entero para servir una página. El endpoint HTTP conserva su shape sin paginar.
- **`get_projection`** declara el rango real de `months` (12..840) en el schema publicado y avisa
  en la descripción de que un `months` explícito recomputa sin cache.
- **`get_settings`** incluye `user {id, username, birth_date}` del usuario del token (la DOB que
  fija el horizonte de proyección). El endpoint HTTP `GET /v1/installation` no cambia.

### Changed — **cambio de comportamiento**: las rentabilidades negativas componen de verdad en el engine

- `monthly_multiplier` (engine) trataba cualquier tasa anual ≤ 0 como crecimiento 0: un activo
  guardado con retorno esperado −5 % se proyectaba **plano**, y un what-if pesimista era imposible.
  Ahora una tasa negativa compone su factor real — la raíz 12ª de `1 + p/100` — mientras el factor
  anual sea positivo (−100 < p < 0); `p ≤ −100` se clampa a factor 0 (pérdida total; la capa API
  rechaza esos inputs con error tipado allí donde se aceptan overrides). `None` y `0` siguen siendo
  factor 1, y las tasas positivas conservan la fórmula exacta anterior (regresión pinneada:
  10 % anual ⇒ 1,0079741…).
- **Números trabajados**: 10.000 € al −50 % anual ⇒ factor mensual 0,5^(1/12) ≈ 0,94387 ⇒ ≈ 5.000 €
  a los 12 meses (antes: 10.000 € intactos). 12.000 € líquidos al −5 % con gasto 1.000 €/mes ⇒ el
  runway baja de 12,0 meses exactos a ≈ 11,7 (el saldo decrece mientras se consume).
- **Radio**: afecta a toda proyección persistida con activos de tasa negativa (pasan de plano a
  decrecer) y al runway de `/v1/summary` (un retorno negativo ahora lo **acorta**). El colapso de
  la **inflación** ≤ 0 en el target FIRE se mantiene intacto (deflación sostenida sigue fuera del
  modelo), y la inflación del gasto del runway nunca es negativa (la instalación valida 0..50).
  Sin impacto en la paridad Rust↔TS: `fire.ts` no duplica el multiplicador mensual.

## [3.2.0] - 2026-08-17

Dos cambios sobre la misma base: las estadísticas de movimientos. `schema_version` del `.ffbackup`
sube a **7** (los v1..v6 siguen importando). **Breaking acotado** en las reglas recurrentes (abajo).

### Added — KPI «Ahorro real vs esperado» en el Dashboard

- Nueva card en «Salud financiera» (Resumen): el ahorro **real** en grande (promedio mensual de los
  movimientos de los últimos 12 meses civiles completos) y debajo «(de X € esperados)» (el neto del
  presupuesto). **Por qué**: la tasa de ahorro sola no dice si el plan se cumple — hasta ahora el
  Dashboard mostraba una única base (presupuesto en modo A, promedio real en B/C), nunca las dos a
  la vez, así que la pregunta «¿ahorro lo que planifiqué?» no tenía respuesta a la vista.
- Tres campos aditivos en `financial_health` de `GET /v1/summary` (no breaking; también visibles
  vía la tool MCP `get_summary`): `savings_expected_monthly_equivalent` (neto del presupuesto,
  capturado antes del override B/C — no sigue el modo), `savings_actual_monthly_avg_12m` (promedio
  **bruto** `income − expense`, sin resta híbrida de cuotas: las cuotas pagadas ya cuentan como
  gasto, simétrico al esperado que incluye las cuotas derivadas; **ausente** sin meses con datos) y
  `savings_actual_months_with_data`. Idénticos en los tres modos `savings_source`; para servir el
  real también en modo A, `/v1/summary` calcula ahora siempre el promedio 12m (1 query extra sin
  transacciones, 3 con; el endpoint no tiene cache). Sin movimientos la card muestra «—»; con
  esperado ≤ 0 se muestra igualmente (el numerador sigue siendo información).

### Changed — **breaking**: reglas recurrentes con resolución mensual (sin `day_of_month`)

- **Por qué**: las instancias recurrentes se fechaban al día configurado (típicamente el 1) y
  aparecían al principio del mes en curso, distorsionando sus estadísticas — el flujo real del
  usuario registra el resto de operaciones al cerrar el mes. Un día configurable por regla no
  aporta nada a una estadística mensual y era la fuente de la distorsión, así que se elimina la
  resolución diaria en vez de parchearla.
- **Semántica nueva** (materializador y backfill del alta comparten el mismo loop): la instancia
  del mes M se fecha en el **último día de M** (cuenta en las estadísticas de M — `op_date` es la
  única atribución mensual) y solo se crea con M ya **cerrado** (servidor en M+1). El mes en curso
  jamás se materializa, ni siquiera en su último día. Se descartó fechar en el 1 de M+1: movería la
  nómina de enero a las estadísticas de febrero.
- **Breaking** (sign-off del owner en la sesión de diseño):
  - Migración SQL que **elimina la columna** `recurring_transaction_rules.day_of_month`
    (data-loss deliberado: se pierde la configuración por-regla del día; las instancias ya
    materializadas conservan su `op_date` histórico — para meses cerrados el bucket mensual es el
    mismo, así que promedios y comparativas no cambian).
  - `RecurringRuleResponse` pierde `day_of_month`; `recurrence` en `POST /v1/transactions[/batch]`
    pasa a ser un marcador vacío `{}` — un cliente ≤3.1.0 que aún envíe `day_of_month` no falla:
    el campo se **ignora** (y el error `recurrence_day_out_of_range` desaparece).
  - `.ffbackup` `schema_version` **6 → 7**: `BackupRecurringRule` pierde `day_of_month`
    (`payload_v6_to_v7` lo descarta al importar backups viejos; la cadena v1→…→v7 completa sigue
    importando).
- Las reglas existentes adoptan la política automáticamente (era un atributo de la plantilla, no
  de las instancias). Las instancias del mes en curso ya materializadas a día 1 se conservan: un
  único mes residual que desaparece al cambiar de mes.

## [3.1.0] - 2026-08-17

**Conector de claude.ai web: OAuth 2.1 embebido**. El límite conocido de la 3.0.0 — «el conector de
claude.ai exige OAuth 2.1, fuera de scope» — desaparece: el mismo binario actúa ahora de
**authorization server + resource server OAuth 2.1** para `/mcp`, sin IdP externo ni contenedores
nuevos. Añadir FutureFin como conector personalizado en claude.ai (web/móvil/Desktop) pasa a ser:
pegar `https://tu-host/mcp`, iniciar sesión en la pantalla de consentimiento de FutureFin y
autorizar. Los tokens `ffp_…` de la 3.0.0 siguen funcionando igual (Claude Code y clientes MCP
genéricos); OAuth es el **tercer esquema de credencial**, no un reemplazo. El login de la app no
cambia (username+password Argon2id): OAuth aquí delega acceso a una app, nunca inicia sesión.
Una migración SQL nueva (5 tablas `oauth_*`); `schema_version` del `.ffbackup` sigue en **6**.
**No breaking**.

### Added — Authorization server OAuth 2.1 en el propio binario

- **Protocolo completo en rutas raíz** (fuera de OpenAPI, como `/mcp`): metadata de descubrimiento
  RFC 8414 (`/.well-known/oauth-authorization-server`) y RFC 9728
  (`/.well-known/oauth-protected-resource`) — **ambas también con el sufijo `/mcp`**, porque la
  inserción de path del §3.1 de esas RFC es lo que consulta claude.ai y montarlas solo en la raíz
  es la causa #1 de «connection failed» —, registro dinámico de clientes RFC 7591
  (`POST /oauth/register`, abierto: la fila de cliente no da acceso a nada, el gate es el
  consentimiento), token endpoint (`POST /oauth/token`, grants `authorization_code` + PKCE
  **S256-only** y `refresh_token` con rotación) y revocación RFC 7009 (`POST /oauth/revoke`).
  El 401 de `/mcp` anuncia la metadata vía `WWW-Authenticate: Bearer resource_metadata="…"`
  (RFC 9728 §5.1) — **solo el 401**: un 403 (usuario pendiente, membership revocada) con ese header
  metería a claude en un bucle de re-autorización infinito.
- **Credenciales con el contrato D14 de siempre, nada de JWT**: access tokens opacos `ffo_…` (1 h) y
  refresh tokens `ffr_…` (90 días **sin uso**; cada refresh rota y renueva la ventana), solo se
  persiste el SHA-256, y cada request `/mcp` re-resuelve la membership viva — revocar corta al
  instante. Reusar un authorization code ya canjeado o un refresh token ya rotado es la señal de
  robo del OAuth 2.1: **revoca el grant entero** (`revoked_reason` = `code_reuse` /
  `refresh_token_reuse` queda como auditoría). Todas las caducidades las calcula Postgres
  (`now() + interval`), nunca el reloj de Rust.
- **El grant es la unidad de consentimiento**: una fila por (app, usuario) — índice UNIQUE parcial
  `WHERE revoked_at IS NULL` — y re-consentir la misma app la reutiliza en vez de duplicarla.
  Revocar el grant mata sus access/refresh tokens sin tocarlos (el lookup de auth hace JOIN y exige
  el grant vivo): una fila que actualizar para cortar todo, como borrar una sesión.
- **`resource` (RFC 8707) validado en la emisión, no re-validado en `/mcp`** — decisión documentada
  (D15): FutureFin es el único AS y el único RS de sus tokens; re-comparar contra el Host de cada
  request rompería el caso real «consiento por el dominio del túnel, consulto por la IP de LAN».
- **URL pública derivada del request** (`X-Forwarded-Proto`/`X-Forwarded-Host`/`Host`, con charset
  estricto anti header-injection) — **ninguna env var nueva es obligatoria**. Para proxies que no
  mandan esos headers: `FUTUREFIN_PUBLIC_URL` (opcional, validada al arrancar, fail-loud como
  `CORS_ORIGINS`).
- **Anti-flood del registro abierto**: GC perezoso dentro del propio `POST /oauth/register` (borra
  clientes de >24 h sin ningún grant; nunca en un GET — D5) y cupo de 1000 clientes → 503.
  `client_id` desconocido en el token endpoint responde **401 `invalid_client`**, la señal exacta
  con la que claude.ai re-registra vía DCR — y por la que un restore de backup sin tablas OAuth se
  auto-recupera sin intervención.

### Added — Pantalla de consentimiento en la SPA y panel de conexiones

- **`/oauth/authorize` es una vista de la SPA** (chunk lazy propio enganchado en `main.tsx`, fuera
  del router de pestañas): valida los parámetros vía `GET /v1/oauth/authorize-details`, reutiliza
  el login existente si no hay sesión (los query params OAuth sobreviven porque el login es un
  fetch, sin navegación) y muestra el consentimiento con el design system — el **host del
  redirect** destacado como único dato verificado, el nombre del cliente marcado como declarado
  por la app, «Autorizas como {usuario}» con cambio de usuario, y el detalle de permisos (solo
  lectura). Autorizar/Cancelar van por `POST /v1/oauth/authorize` (cookie; deny devuelve
  `error=access_denied` al cliente). Errores fatales (cliente desconocido, redirect sin match
  exacto) se **pintan y nunca redirigen** — redirigir sería un open redirect.
- **Ajustes → Acceso gana el panel «Conexiones»**: apps conectadas por usuario (nombre, host,
  fecha, último uso con el throttle de 60 s) y revocación con confirmación — corte inmediato.
  `GET/DELETE /v1/oauth/connections` se montan **siempre**, incluso con
  `FUTUREFIN_MCP_ENABLED=0` (precedente `/v1/api-tokens`: apagar MCP no puede dejarte sin poder
  revocar grants existentes).
- **Anti-clickjacking global**: toda respuesta (SPA incluida) lleva `X-Frame-Options: DENY` —
  protege sobre todo la pantalla de consentimiento; nada de FutureFin se embebe legítimamente en
  iframes.

### Migración / compatibilidad

- **Migración `20260817090000_oauth.sql`**: crea `oauth_clients`, `oauth_grants`,
  `oauth_authorization_codes`, `oauth_access_tokens` y `oauth_refresh_tokens` (FKs con
  `ON DELETE CASCADE` colgando de grants; soft-revoke con `revoked_at`/`revoked_reason`).
  Sin pérdida de datos; el resto del esquema es idéntico al de 3.0.0.
- **Backups `.ffbackup`**: `schema_version` sigue en **6**. Las cinco tablas `oauth_*` quedan
  **excluidas a propósito** del export/import (mismo criterio que `api_tokens`: credenciales, no
  datos financieros). Tras un restore, claude.ai se reconecta solo: su `client_id` ya no existe →
  401 `invalid_client` → re-registro DCR → nuevo consentimiento.
- **API**: endpoints existentes sin cambios. Nuevos: rutas raíz `/.well-known/*` y `/oauth/*`
  (protocolo, fuera de OpenAPI) y `/v1/oauth/*` (SPA, en OpenAPI).
- **Rollback**: volver a la imagen 3.0.0 con la migración aplicada es seguro — las tablas `oauth_*`
  quedan huérfanas e inertes (ningún código 3.0.0 las toca) y el conector de claude.ai deja de
  funcionar hasta re-actualizar.
- **Fuera de scope** (documentado): conectividad/TLS/túnel (sigue siendo del usuario), scopes
  granulares (MCP v1 es 100 % lectura), RFC 7592 (editar un registro: los clientes re-registran) y
  rate-limit del token endpoint (secretos de 256 bits `OsRng`, lookup por hash exacto — no hay
  adivinación online viable).

## [3.0.0] - 2026-08-16

**Imagen autocontenida + servidor MCP**: PostgreSQL pasa a vivir **dentro de la propia imagen** de
FutureFin. El stack deja de ser dos contenedores (app + `futurefin-database`) y pasa a ser **uno solo**,
con lo que un `docker compose pull && up -d` — o watchtower con `:latest` — actualiza todo el sistema de
una pieza. Además la release estrena un **servidor MCP embebido de solo lectura** (`/mcp`) con **tokens
de API por usuario**, para conectar Claude u otro cliente MCP a la instalación. Una migración SQL nueva
(`api_tokens`); el `schema_version` del `.ffbackup` sigue en **6**. **Breaking operacional** (topología
de despliegue), no de API ni de backups.

### Changed — PostgreSQL 16 embebido, un solo contenedor

- **Por qué**: la pareja app+DB unida por `depends_on` era fricción pura para una app monoinstalación —
  dos servicios que gestionar, una `POSTGRES_PASSWORD` obligatoria que nadie usaba desde fuera, y
  actualizaciones desatendidas frágiles (watchtower actualizaba la app pero la DB y su healthcheck
  quedaban a su suerte). El volumen y el binario ya estaban acoplados de facto.
- **Cómo**: el runtime sigue siendo `debian:bookworm-slim` (digest-pinned) con los binarios de PostgreSQL
  **copiados de las imágenes oficiales** `postgres:16-bookworm` y `postgres:15-bookworm` (digests de
  índice multi-arch; gate `ldd` en build; JIT/llvmjit eliminado: ~120 MB de libLLVM sin uso aquí).
  Deliberadamente **no** se usa `postgres:*` como base ni se declara `VOLUME`: el `VOLUME` heredado crea
  volúmenes anónimos en un `docker run` sin `-v`, y watchtower los pierde al recrear — pérdida silenciosa.
  En su lugar, el entrypoint comprueba con `mountpoint` que hay un volumen real y **aborta** sin él
  (`FUTUREFIN_ALLOW_EPHEMERAL_DB=1` solo para uso desechable).
- **Postgres es socket-only**: sin listener TCP en absoluto (`listen_addresses=''`), auth local `trust`
  — no hay puerto que proteger ni contraseña que gestionar; `POSTGRES_PASSWORD` deja de ser obligatoria
  (si viene, se aplica al rol y nada más). La API conecta por
  `postgres:///futurefin?host=/var/run/postgresql&user=futurefin`.
- **Apagado ordenado supervisado**: el entrypoint (PID 1) para primero la API — que ahora hace *graceful
  shutdown* de verdad (`with_graceful_shutdown` + cierre del pool; tokio gana la feature `signal`) — y
  después el postmaster con **SIGINT** (*fast shutdown* con checkpoint; SIGTERM sería *smart* y puede
  colgarse). `stop_grace_period: 60s` en compose; con watchtower configura `WATCHTOWER_TIMEOUT=60s`.
  Un SIGKILL no corrompe (WAL), solo fuerza recovery al siguiente arranque.
- **Healthcheck**: pasa de `/v1/health` (liveness puro) a **`/v1/ready`** (`SELECT 1`) — en un contenedor
  único, "healthy" debe implicar base de datos viva. Se retira el fallback `</dev/tcp` (enmascaraba
  justamente ese 503); el `CMD-SHELL` se mantiene (incidente v1.0.2 sigue vigente). La imagen además
  declara su propio `HEALTHCHECK` para quien use `docker run` pelado.
- **Procesos sin privilegios**: `postgres` (uid 999, como la imagen oficial Debian) para el postmaster y
  un usuario dedicado `futurefin` (uid 10001) para la API vía `gosu`; root solo en el supervisor.
- **Logs**: un único flujo — `docker compose logs -f futurefin` mezcla entrypoint
  (`[futurefin-entrypoint]`), PostgreSQL y la API.
- La API gana `connect_with_retry` (backoff 0,5→4 s, `FUTUREFIN_DB_CONNECT_TIMEOUT_SECS`, default 30):
  el modo con DB externa pierde el `depends_on: service_healthy` que suplía la falta de retry.
- Tamaño de imagen: ~120 MB → ~330-360 MB descomprimida; a cambio desaparece la descarga separada de
  `postgres:16.4-alpine`, así que el total transferido es comparable.

### Added — backup automático pre-migración (con retención)

- Antes de arrancar la API con una **versión nueva o migraciones pendientes** (comparando los manifiestos
  `/app/VERSION` y `/app/migration-versions.txt` contra `_sqlx_migrations`), el entrypoint escribe
  `pre-migration-<desde>-a-<hasta>-<ts>.sql.gz` en el volumen nuevo **`ffdata`** (`/var/lib/futurefin`).
  Si el backup **falla, el arranque se aborta**: el momento en que no se puede escribir el backup es
  exactamente el momento en que más falta hace (bypass deliberado: `FUTUREFIN_PREMIGRATION_BACKUP=off`).
- **Retención** para no hinchar el volumen: los `FUTUREFIN_BACKUP_KEEP` (10) más recientes son intocables;
  del resto se borran los de más de `FUTUREFIN_BACKUP_KEEP_DAYS` (90) días; bajo presión de disco
  (<256 MB libres) se poda de viejo a nuevo sin tocar nunca los 3 últimos.
- Mismo formato `.sql.gz` que `scripts/backup-postgres.sh` ⇒ **un único procedimiento de restore**:
  el nuevo `scripts/restore-postgres.sh <dump> [--yes]`, que usa el modo rescate **`db-only`**
  (`FUTUREFIN_MODE=db-only`: solo PostgreSQL, sin API — también útil para psql/inspección manual).

### Added — auto-`pg_upgrade` de versiones mayores de PostgreSQL

- La imagen empaqueta **16 (activa) + 15**, y el entrypoint detecta un `PGDATA` de un major anterior y lo
  actualiza solo: parada limpia del cluster viejo → `pg_dumpall` **obligatorio** → cluster nuevo en
  staging con locale/encoding/checksums idénticos → `pg_upgrade` en modo **copia** (no `--link`: el
  cluster viejo queda utilizable si algo falla) → verificación por **censo de filas** → swap reanudable.
  El cluster antiguo se conserva en `$PGDATA/pgdata_old_15` (borrado manual, nunca automático).
- El 15 se incluye hoy sin usuarios que lo necesiten **a propósito**: permite ejercitar el camino completo
  en CI en vez de estrenarlo en producción el día que toque 16→17 (la lección del auto-repair). Política:
  cada imagen lleva el major actual + el anterior (la 4.x llevará 17+16).

### Deprecated — base de datos externa (`DATABASE_URL`)

- Definir `DATABASE_URL` sigue funcionando pero queda **deprecado; se elimina en 4.0.0**, con aviso
  enmarcado en cada arranque. Es lo que mantiene vivo, sin intervención, a un usuario 2.x cuyo watchtower
  le plantó la imagen 3.x sin tocar su compose: sin volumen montado en el contenedor de la app, la 3.x
  usa su `futurefin-database` de siempre (probado en CI).
- **Automigración one-shot**: con `DATABASE_URL` definida **y** un volumen vacío montado, el entrypoint
  copia la base externa a la embebida una única vez — dump (la externa solo se **lee**), restore,
  **verificación por censo de filas**, marcador de idempotencia (jamás re-migra; máximo 3 reintentos y
  los intentos fallidos se apartan con `mv`, nunca `rm`). Si la externa no responde, **aborta** en vez de
  arrancar vacío en silencio. Opt-out: `FUTUREFIN_DB_MODE=external`.

### Added — Servidor MCP embebido (solo lectura) y tokens de API

- **`/mcp` (Streamable HTTP) dentro del mismo binario y puerto**: FutureFin expone un servidor
  [MCP](https://modelcontextprotocol.io) con **10 tools de solo lectura** — `get_summary`,
  `get_projection`, `get_budget`, `get_transactions_summary`, `list_transactions`, `get_history`,
  `list_assets`, `list_liabilities`, `list_planning_flows`, `get_settings` — para consultar las
  finanzas desde Claude Code/Desktop u otro cliente MCP. Implementado con el SDK oficial Rust
  (`rmcp` 3.1, spec 2026-07-28 sessionless + compatibilidad con clientes legacy con
  `Mcp-Session-Id`). Cero contenedores nuevos: sale por el mismo `EXPOSE 8080`; compose e imagen
  no cambian.
- **Cero deriva handler↔tool por construcción**: cada tool llama a la MISMA core fn que su endpoint
  HTTP (los handlers de lectura se partieron en «extractores + auth» y `*_core(pool, iid, user_id,
  view, …)`, sin cambiar SQL ni tipos) y serializa el mismo struct serde → el contrato
  Decimal-as-string sobrevive intacto (test de paridad byte a byte `get_summary` vs `GET
  /v1/summary` en `mcp_http.rs`). `get_projection` comparte la cache de proyección con el handler
  (misma key, mismo TTL) y va **fijo a `density=hybrid`** (~82 puntos ≈5 KB) con `asset_series`
  opt-in — la serie mensual completa (~260 KB) no aporta nada a un LLM.
- **Tokens de API por usuario (`ffp_…`)**: nueva tabla `api_tokens` y CRUD `GET/POST /v1/api-tokens`
  + `DELETE /v1/api-tokens/{id}` (auth por cookie, en OpenAPI). El secreto son 32 bytes de `OsRng`
  en base64url con prefijo reconocible y **solo se persiste su SHA-256**; se muestra **una única
  vez** al crear. El token NO congela rol ni installation: cada request MCP re-resuelve
  `require_installation_member`, así que revocar la membership mata el token al instante (misma
  filosofía que las sesiones en DB). Cualquier miembro — viewer incluido — puede crear los suyos:
  un token no puede hacer nada que su dueño no pueda ya y el MCP v1 es 100 % lectura. Máximo 10
  activos por usuario; revocación soft (`revoked_at`, la fila queda como auditoría);
  `last_used_at` con throttle de 60 s.
- **Errores con el contrato de siempre**: validación/dominio → `CallToolResult{is_error}` con el
  mismo JSON `{error, message}` del wire HTTP (el LLM puede leerlo y corregir); `Db/Unavailable` →
  error de protocolo sanitizado (detalle solo a tracing), espejo exacto de `error.rs`.
- **UI**: Ajustes → **Acceso** gana el panel «Tokens de API (MCP)» (crear con label + caducidad
  opcional 90 días/1 año, copiar-una-vez, último uso, revocar con confirmación). El sub-tab Acceso
  pasa a ser visible para **cualquier miembro** (aprobar usuarios pendientes sigue siendo
  owner-only dentro del tab).
- **Config**: `FUTUREFIN_MCP_ENABLED` (default `true`; con `0` el router `/mcp` ni se monta →
  404). El endpoint es inerte sin tokens (todo responde 401), así que el default habilitado no
  abre nada por sí solo. CORS gana `Authorization` y `Mcp-Session-Id` en `allow_headers` (para MCP
  Inspector/clientes de navegador).
- **Límite conocido**: el conector de claude.ai (web/móvil) exige OAuth 2.1 — fuera de scope en
  esta versión; el middleware Bearer es el punto de extensión si algún día se añade. Claude
  Code/Desktop y clientes genéricos funcionan con el token:
  `claude mcp add --transport http futurefin https://tu-host/mcp --header "Authorization: Bearer ffp_…"`.

### Migración / compatibilidad

- **Migración `20260816120000_api_tokens.sql`**: crea la tabla `api_tokens` (id, user_id FK→users
  ON DELETE CASCADE, label, token_hash UNIQUE, token_prefix, created_at, expires_at, last_used_at,
  revoked_at). Sin pérdida de datos; el resto del esquema es idéntico al de 2.3.0.
- **Backups `.ffbackup`**: `schema_version` sigue en **6**. `api_tokens` queda **excluida a
  propósito** del export/import: son credenciales de la instalación, no datos financieros — un
  restore no debe resucitar secretos. API: sin cambios de contrato en los endpoints existentes.
- **Datos**: **sin pérdida**. El volumen `futurefin_pgdata` se reutiliza tal cual — mismo nombre y misma
  ruta de montaje (`/var/lib/postgresql/data`) en el compose nuevo. En el **primer arranque** tras
  actualizar, una sola vez: (1) ajuste de propiedad de los ficheros (la imagen Alpine de 2.x usaba uid 70;
  la Debian usa 999), y (2) `REINDEX DATABASE` + `REFRESH COLLATION VERSION`, porque los índices de texto
  se construyeron con la colación de musl y ahora los lee un PostgreSQL glibc — sin ese REINDEX habría
  índices únicos silenciosamente corruptos (comprobado en CI: el username duplicado devuelve 409, no éxito).
- **Primer arranque tras actualizar**: sustituye tu `docker-compose.yml` por el de 3.0.0 y ejecuta
  `docker compose up -d --remove-orphans` (retira el contenedor `futurefin-database`). Tarda más de lo
  normal una única vez (chown + REINDEX + backup automático; `start_period: 120s`). Verifica con
  `/v1/ready` (no `/v1/health`) y `docker compose logs futurefin | grep -E "migrations applied|ERROR"`.
  Recomendado antes: exportar tu `.ffbackup` y un `pg_dump`.
- **Rollback a 2.x**: la imagen 2.x no arranca PostgreSQL. `docker compose down`, restaura tu
  `docker-compose.yml` y `.env` de 2.x (con `POSTGRES_PASSWORD`) y levanta: el volumen `pgdata` no cambió
  de forma y `postgres:16.4-alpine` reajusta la propiedad al arrancar. Si la 3.x llegó a aplicar
  migraciones de una futura 3.y, aplica la regla forward-only de siempre (VersionMissing). El volumen
  `ffdata` queda huérfano — consérvalo si quieres los backups automáticos.
- **Breaking operacional**: desaparece el servicio `futurefin-database` — cualquier script/cron que haga
  `docker compose exec futurefin-database …` debe apuntar a `futurefin` y añadir `-h /var/run/postgresql`
  (así lo hacen ya `scripts/backup-postgres.sh` y `db-stats.sh`). `docker-compose.split-dev.yml`
  desaparece: el Postgres de desarrollo es ahora el compose autónomo `docker-compose.dev.yml` (project
  `futurefin-dev`, volumen `devdata` — nota en el propio fichero para reutilizar el volumen antiguo).
  Quien siga el tag `:2` no salta a 3.x automáticamente.

## [2.3.0] - 2026-08-15

El caso «infinito» del **runway** deja de decidirlo el tope de simulación de 100 años y pasa a decidirlo el
**SWR configurado en Jubilación** (cierra el issue #1, con una modificación acordada sobre su propuesta
original). Sin migración; el `schema_version` del `.ffbackup` sigue en **6**.

### Changed — el runway «infinito» lo decide el SWR, no el tope de 100 años

- **Por qué**: el tope de 1.200 meses era un proxy tosco («Cubierto (más de 100 años)») y la condición
  analítica de perpetuidad `A·j ≥ g` que proponía el issue seguía siendo una propiedad del modelo de
  rentabilidad — que el engine no modela con pérdidas ni volatilidad. El SWR es el parámetro que el usuario
  **ya configura** en Jubilación y el que define «puedo dejar de trabajar»: usarlo como umbral hace del
  runway un proxy de FIRE coherente con el resto de la app. Se descartan por tanto **ambos** disparadores
  anteriores (tope y perpetuidad).
- **La condición**, con el mismo gross-up fiscal que el target FIRE (`gross_up_net_annual_fire`, tramos de
  `fire_settings.tax_brackets` y `taxes_enabled`):
  `infinito ⟺ gross_up(12 × expense_total) ≤ líquidos × (swr_pct/100)`. La comparación se hace sin
  división (`gross·100 ≤ A·swr`), así que la frontera es **exacta** en `Decimal`. Con `swr_pct = 0` nunca
  hay infinito. El disparador es deliberadamente independiente de rentabilidad e inflación (que siguen
  gobernando el caso finito): es la definición de SWR, que ya asume una cartera cuyo retorno real sostiene
  esa retirada.
- **Ejemplos antes/después** (SWR 3,5 % por defecto): `1.000.000 € al 7 %` con `4.000 €/mes` de gasto —
  antes «Cubierto» (el saldo sobrevivía el tope), ahora **«+100 años» finito** porque la retirada bruta
  (48.000 €) supera el 3,5 % del saldo (35.000 €). Y el converso: `240.000 €` **sin rentabilidad** con
  `700 €/mes` (impuestos off) — antes ~28,5 años, ahora **«Infinito»** (8.400 = 8.400, frontera exacta).
  La semántica del KPI pasa de «el dinero no se acaba en 100 años» a «tu tasa de retirada cabe en tu SWR».
- **Engine (breaking para la capa handler)**: `liquid_runway_months` gana dos parámetros — `swr_pct` y
  `annual_expense_for_swr` (el gasto anual ya grosseado por el handler) — y `MAX_RUNWAY_MONTHS` deja de ser
  centinela de infinito: sobrevivir el tope devuelve `Months(1200)`, un **suelo** («al menos 100 años»).
  El orden de checks es contrato: `NoExpenseBase` → `Months(0)` → umbral SWR → bucle finito (con gasto 0
  la desigualdad SWR sería trivialmente cierta). La reducción exacta a `A/g` bajo el umbral sigue intacta
  (`runway_pre_change_baseline_liquid_over_expense` sigue dando 10,000… exacto).
- **API no breaking**: `runway_months` y `runway_is_indefinite` conservan tipo, nullabilidad y significado
  («infinito ⇒ months null»); solo cambia el disparador. El valor `1200` en `runway_months` es el suelo.
  `installation_calendar_inflation_savings` pasa a llamarse `installation_calendar_inflation_fire` y
  devuelve los `FireSettings` completos (misma única query; summary ya no descarta `swr_pct` ni los tramos).
- **UI**: la tarjeta pasa de «Cubierto (más de 100 años)» a **«Infinito (dentro del SWR 3,5 %)»**
  — el paréntesis (`runwaySwrParenthetical`, helper puro en `lib/fire.ts`) muestra el SWR realmente
  configurado, no promete supervivencia — y el suelo se muestra como «+100 años» (`formatRunwayValue`).
- **Regresión**: `runway.rs` 8 → 13 tests unitarios (frontera exacta por igualdad, un euro por debajo,
  `swr = 0` y `swr < 0` nunca infinitos, tope como suelo, y que el gasto grosseado participa);
  `summary_runway.rs` 7 → 10 (frontera exacta end-to-end con impuestos off, flip del umbral al activar
  impuestos — fija que runway y target FIRE comparten gross-up — y suelo `1200` con SWR 0). El escenario
  del test indefinido histórico (1M @ 7 % / 1.000 €/mes) sigue siendo infinito con ambos criterios.



Coherencia de **todas** las métricas con `fire_settings.savings_source` (modos B `transactions_avg` y C
`budget_income_real_expense`) y un **runway** que ya no es una división: compone la rentabilidad esperada de
los activos líquidos y la inflación del gasto. Incluye el fix del bug que hacía que la pestaña **Jubilación**
ignorara el modo activo y divergiera del target del servidor. Sin migración; el `schema_version` del
`.ffbackup` sigue en **6**.

### Fixed — Jubilación usaba SIEMPRE presupuesto en los modos B y C

- **Síntoma → causa → fix**: en modo B/C, la pestaña **Jubilación** («Gasto actual», «Ingresos actuales»,
  «Patrimonio objetivo», «Primer cruce») mostraba cifras de **presupuesto** y su «Patrimonio objetivo»
  divergía del `jubilacion_target_net_worth` que devolvía el servidor; los paréntesis «promedio de N meses»
  del Resumen tampoco aparecían. Causa raíz: el backend serializa `savings_source` y
  `savings_source_months_with_data` **dentro de `financial_health`** (`FinancialHealthMetrics`), pero
  `apps/web/src/api/types.ts` los declaraba en la **raíz** de `SummaryResponse` → `SummaryView` y
  `RetirementView` leían siempre `undefined`, y `savingsSourceUsesTransactions(undefined)` es `false`, así
  que el cliente se comportaba como si el modo fuera siempre A. TypeScript no lo detectaba: campos
  opcionales inexistentes en el JSON son `undefined` legítimo. Fix: los dos campos se mueven a
  `FinancialHealthMetrics` en `types.ts` (el `typecheck` señaló los dos consumidores) y ambas vistas leen de
  `summary.financial_health`. **Sin cambio de servidor** — el JSON siempre fue el correcto.
- **No regresiona**: el paréntesis pasa por un helper puro compartido, `savingsAvgParenthetical(source,
  months)` en `lib/fire.ts` (`"promedio de N meses"`, singular incluido; `undefined` en modo A o tras el
  fallback del servidor), consumido por Resumen y por el chart de proyección — una sola definición que los
  tests de Vitest fijan.

### Fixed — caps `months_expense` / `income_multiple` de Activos se resolvían con presupuesto

- **Objetivo mostrado incoherente con la simulación**: `GET/POST/PATCH /v1/assets` resolvía los caps de las
  reglas de asignación (`months_expense` = N × (gasto + servicio de deuda), `income_multiple` = N × income)
  con los escalares del **presupuesto**, incluso en modo B/C — mientras la aportación del mes 1 mostrada en
  la misma respuesta ya salía del promedio real. El objetivo en € no casaba ni con esa aportación ni con la
  proyección. Ahora ambos salen del **mismo** build: `assets_projection_context` (`handlers/projection.rs`)
  sustituye a `first_month_asset_contribution_nominals_map` + `monthly_income_expense_debt_for_view` (ambos
  eliminados) y devuelve `{nominals, income_monthly, expense_with_debt}` con los escalares **efectivos** que
  usa el engine. De paso, cada call site pasa de **dos** construcciones de proyección por request a **una**.
- **Regresión**: `assets_cap_targets_follow_savings_source_mode` (`savings_source.rs`) — con el mismo
  ledger, los caps valen 18.000 € / 10.000 € en modo A y 6.000 € / 8.000 € en modo B; el test falla contra
  el código anterior.

### Changed — `/v1/summary`: base de gasto real en B/C y runway con rentabilidad e inflación

**Cambio de contrato (no breaking de schema)**: no se añade, quita ni renombra ningún campo obligatorio; lo
que cambia es el **valor** de tres campos ya existentes de `financial_health` en escenarios concretos. Un
cliente que solo los pinte sigue funcionando.

- **Base de gasto en modo B/C con datos**: `expense_derived_monthly_equivalent` pasa a ser exactamente el
  **servicio de deuda** de los pasivos activos (mismo filtro `payment_end_date IS NULL OR >= today` que el
  resto de lecturas) y `expense_total_monthly_equivalent` pasa a `expense_eff + debt_service` (gasto real
  promedio 12m con resta híbrida de cuotas, más el servicio de deuda). Hasta 2.1.0, en esos modos
  `expense_reg` y `net` se sustituían por la base real pero `expense_der`/`expense_tot` se quedaban con los
  del presupuesto, así que las dos identidades que en modo A siempre valen estaban **rotas**:
  `expense_total = expense_regular + expense_derived` y `net = income − expense_total`. Ahora vuelven a
  valer en los tres modos (`mode_b_runway_uses_effective_expense_base`).
- **`runway_months` compone rentabilidad e inflación**: era `liquid_assets_total / expense_total`. Ahora lo
  calcula la función pura nueva `liquid_runway_months` (`crates/engine/src/runway.rs`): bucle mes a mes en
  `Decimal` en el que los líquidos crecen a la **media ponderada por valor** de sus multiplicadores
  mensuales y el gasto se **infla** con `annual_inflation_assumption_percent`, con retirada antes del
  crecimiento (el mismo orden que la simulación) y cap de 1.200 meses (100 años). Sin rentabilidad ni
  inflación se reduce **exactamente** a la división anterior, así que la captura de regresión previa al
  cambio (`runway_pre_change_baseline_liquid_over_expense`) sigue verde sin tolerancias.
- **Sin datos, sin cambio**: en modo B/C con `months_with_data == 0` el fallback al presupuesto sigue
  devolviendo un `financial_health` **idéntico** al de modo A (`mode_b_zero_months_falls_back_to_budget_runway`).
- **Backend**: nuevo helper `installation_calendar_inflation_savings` (una query para fecha civil +
  inflación clampada a ≥ 0 + `savings_source`) que sustituye en summary a `installation_naive_today` +
  `projection_savings_source` — un round-trip menos. `liquid_sql` pasa a devolver filas
  `(current_value, expected_annual_return_percent)` y la suma `liquid_assets_total` se hace en Rust (el
  runway necesita la rentabilidad por activo). `monthly_multiplier` pasa a `pub(crate)` para que el runway
  use **exactamente** la misma conversión anual→mensual que la simulación (y su regla «tasas ≤ 0 →
  crecimiento 0»).

### Números worked before/after (runway, verificados ejecutando el engine)

12.000 € en activos líquidos, gasto total 1.200 €/mes. «Antes» es siempre la división
`liquid_assets_total / expense_total` = 10 meses, insensible a rentabilidad e inflación:

| Escenario | Antes (2.1.0) | Ahora (2.2.0) |
|---|---|---|
| Rentabilidad 0 %, inflación 0 % | 10 meses | **10 meses** (idéntico, por construcción) |
| Rentabilidad 5 %, inflación 0 % | 10 meses | **10,19 meses** |
| Rentabilidad 0 %, inflación 3 % | 10 meses | **9,89 meses** |
| Rentabilidad 5 %, inflación 3 % | 10 meses | **10,07 meses** |
| 1.000.000 € al 7 %, gasto 1.000 €/mes | 1.000 meses | **«Cubierto»** (`runway_is_indefinite`) |

Y el efecto del cambio de base (test `mode_b_runway_uses_effective_expense_base`): 16.000 € líquidos sin
rentabilidad, presupuesto de gasto 8.000 €/mes, dos pasivos activos con 800 €/mes de cuotas y un único mes
real con 800 € de gasto:

| `financial_health` | Modo A (`budget`) | Modo B — antes (2.1.0) | Modo B — ahora (2.2.0) |
|---|---|---|---|
| `expense_regular_monthly_equivalent` | 8.000 | 800 (`expense_eff`) | 800 (`expense_eff`) |
| `expense_derived_monthly_equivalent` | 800 | 800 (línea derivada del presupuesto) | 800 (ahora **por definición** el debt service; aquí coinciden porque ambos pasivos están activos) |
| `expense_total_monthly_equivalent` | 8.800 | 8.800 (presupuesto) | **1.600** |
| `net_monthly_equivalent` | 200 | 1.400 (≠ income − total ✗) | 1.400 (= 3.000 − 1.600 ✓) |
| `runway_months` | 1,8 | 1,8 | **10** |

### Added

- **`financial_health.runway_is_indefinite` (`bool`)**: `true` cuando la rentabilidad esperada de los
  líquidos cubre el gasto durante ≥ 100 años; en ese caso `runway_months` **no se serializa**
  (`skip_serializing_if`, igual que hoy con gasto 0). Distingue el caso «cubierto» del «sin base de gasto»
  (`expense_total == 0`), donde el flag es `false`.
- **`GET /v1/projection/series`: `savings_source` y `savings_source_months_with_data`** (aditivos, mismo
  naming y semántica que en `/v1/summary`): fuente **efectiva** tras el fallback que produjo
  `monthly_delta_assumption`. Permite etiquetar la base del Δ mensual en el chart sin un fetch extra.
- **UI — runway legible**: `formatMonthsRough` pasa a años + meses a partir de 24 meses («2 años», «2 años y
  6 meses»; por debajo de 24 sigue en meses con un decimal, sin cambios), y el nuevo `formatRunwayValue`
  muestra **«Cubierto»** cuando el runway es indefinido, con el paréntesis «más de 100 años». La tarjeta
  Runway se muestra también en ese caso (antes se ocultaba: `runway_months` null se leía como cero).
- **UI — base visible en las métricas derivadas**: paréntesis «promedio de N meses» en Ahorro, Tasa y Runway
  del Resumen en modo B/C, y la línea de meta del chart de proyección pasa de «Δ regular presup.» a
  «Δ regular prom. N meses» cuando la base viene de movimientos.

### Compatibilidad

- **Sin migración de DB ni de backup**: los tres campos nuevos son de respuesta; `CURRENT_SCHEMA_VERSION`
  del `.ffbackup` **sigue en 6**. Rollback a 2.1.0 sin pasos manuales.
- **Los números pueden moverse tras actualizar**: quien tenga rentabilidades esperadas en sus activos
  líquidos o inflación > 0 verá un runway distinto (mayor con retorno, menor con inflación), y quien esté en
  modo B/C verá cambiar `expense_total`/`expense_derived` y, con ellos, el runway. Es precisamente el fix
  buscado, no un efecto colateral.

## [2.1.0] - 2026-07-09

Tercer modo de «fuente del ahorro» de la simulación y endurecimiento del promedio real 12m para que un
backfill de recurrentes no infraestime el gasto/ingreso medio. Sin migración, sin subir el
`schema_version` del `.ffbackup`.

### Proyección — tercer modo `budget_income_real_expense` (income de presupuesto + gasto real)

- **Nuevo valor de `fire_settings.savings_source`**: `budget_income_real_expense` (modo C, label UI
  «Ingresos de presupuesto + gasto real»), que se suma a `budget` (modo A) y `transactions_avg` (modo
  B). Toma el **income del presupuesto** y el **gasto real** promediado (mismo `expense_eff` que el modo
  B: promedio ponderado 12m + resta híbrida de cuotas de préstamos activos + clamp `≥ 0`). Útil cuando la
  nómina es estable pero se quiere que el gasto refleje lo que se gasta de verdad. Ejemplo (test
  `mode_c_income_budget_expense_real`): budget income 5.000, budget expense 2.000; mes real income 3.000,
  gasto 800 → pendiente modo C = 5.000 − 800 = **4.200 €/mes** (modo A daría 3.000; modo B daría
  3.000 − 800 = 2.200).
- **Fallback**: `months_with_data == 0` → cae en silencio al presupuesto completo, igual que el modo B.
- **Target FIRE**: en modo C, `annual_expense` usa el **gasto real** (`expense_eff`) como base y
  `current_income` usa el **income del presupuesto** (no el de las transacciones); `manual` intacto. Sin
  cambios en `compute_fire_target_nw` — todo se resuelve en `EffectiveInputs` de `projection.rs`.
- **`GET /v1/summary`** en modo C: `income_monthly_equivalent` conserva el income del **presupuesto** (no
  se sobreescribe), `expense_regular_monthly_equivalent = expense_eff` y
  `net_monthly_equivalent = income − expense_eff − debt_service`. El `match` sobre `savings_source` es
  exhaustivo (una variante futura fuerza decisión del compilador en vez de heredar el `else`).
  `financial_health.savings_source` ecoa el modo **efectivo** tras el fallback.
- **Backend**: gate único `SavingsSource::uses_transactions()` (`true` para B y C) sustituye al chequeo
  `== TransactionsAvg` disperso; el helper de invalidación de cache pasa a llamarse
  `invalidate_projection_if_savings_uses_transactions`; en `EffectiveInputs` el flag `use_avg` se
  renombra a `expense_from_avg`. Frontend: helpers `savingsSourceUsesTransactions` / `parseSavingsSource`
  en `lib/fire.ts` centralizan el gating de las 3 variantes (el `<select>` de Ajustes → Proyección gana
  una tercera opción; el parenthetical «promedio de N meses» y el fetch gating sirven a B y C).
- **Cache**: las mutaciones de transactions invalidan la proyección en modo B **y** C (regresión
  `mode_c_mutation_invalidates_projection_cache` en `transactions_projection_cache.rs`).

### Proyección — el promedio real 12m solo cuenta «meses reales» (excluye meses pseudovacíos)

- **Síntoma → causa → fix**: al backfillear movimientos recurrentes (nómina/gastos fijos) meses atrás,
  esos meses tenían instancias materializadas (`recurring_rule_id NOT NULL`) pero **ningún** movimiento
  real. `transactions_12m_avg` (consumido por los modos B y C y por las KPIs de Resumen) los contaba como
  meses con datos, diluyendo el promedio → gasto/ingreso medio infraestimado → proyección optimista.
  Ahora el denominador `months_with_data` y las sumas por kind/liability se restringen a **meses reales**
  (mes del tramo con ≥1 transacción `recurring_rule_id IS NULL`, cualquier kind, mismo scope). El
  predicado de «mes real» vive en **una sola fuente** (`real_months_predicate`/CTE `real_months` en
  `handlers/transactions/summary.rs`), reutilizada por las tres queries con los mismos binds.
- **Regla exacta**: un mes vacío o «pseudovacío» (solo instancias recurrentes) queda excluido **por
  completo** — ni numerador ni denominador; un mes real cuenta **entero**, incluidas sus transacciones
  recurrentes. Worked example (test `pseudo_empty_month_excluded_from_avg`): mes real M−2 con income
  manual 2.000 € + mes solo-recurrente M−1 con nómina recurrente 3.000 € → **antes** `months_with_data = 2`
  e `income_avg = (2000 + 3000)/2 = 2.500`; **ahora** `months_with_data = 1` e `income_avg = 2.000`.
  Casos hermanos: `real_month_counts_recurring_too` (M−2 con 2.000 manual + 3.000 recurrente → avg 5.000,
  el mes real cuenta su recurrente) y `mode_b_all_pseudo_empty_falls_back_to_budget` (una ventana
  entera de meses solo-recurrentes tras un backfill → 0 meses reales → fallback al presupuesto).
- **Divergencia deliberada**: la pestaña **Movimientos** (`GET /v1/transactions/summary`) **NO cambia** —
  su promedio ponderado sigue contando cualquier mes con datos (incluidos los solo-recurrentes), porque
  ahí el usuario quiere ver el gasto que realmente ocurrió. Solo el promedio que **alimenta el engine**
  (`transactions_12m_avg`) excluye los pseudovacíos. La diferencia está anotada con un comentario
  cross-ref en el código.
- **Cambio de números (aceptado, documentado)**: usuarios ya en modo B (o C) que hayan backfilleado
  recurrentes verán su pendiente/target moverse — es precisamente el fix buscado, no un efecto colateral.

### Compatibilidad

- **Sin migración de DB ni de backup**: `savings_source` es aditivo (`FireSettings` tiene
  `#[serde(default)]`); `CURRENT_SCHEMA_VERSION` del `.ffbackup` **no** sube.
- **Backup con modo C ↔ servidores ≤ 2.0.1**: un `.ffbackup` exportado con
  `savings_source = "budget_income_real_expense"` importado en un servidor ≤ 2.0.1 falla con **400**
  `unknown variant` (la deserialización es estricta). Aceptado y documentado: subir
  `CURRENT_SCHEMA_VERSION` penalizaría a **todos** los backups por una sola variante nueva; el
  work-around es actualizar el servidor destino antes de importar.

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
