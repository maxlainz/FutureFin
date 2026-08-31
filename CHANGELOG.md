# Changelog

All notable changes to FutureFin will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

## [4.7.0] - 2026-08-31

**Ola 3 de la resolución — «La deuda dice la verdad»** (issues #144, #122, #123, #145, #121,
#151, #129, #130). Es la ola que SÍ mueve cifras del motor: el catálogo de amortización pasa a
describir productos españoles reales, el vencido con saldo deja de esfumarse, y el histórico
interpola con la ley del modelo capturado. Todo número nuevo de esta sección está calculado a
mano antes de correr los tests que lo pinean.

### El catálogo de amortización dice la verdad (#144) — BREAKING con migración firmada

- **El default pasa a `french`** (columna + formulario): el sistema francés ES el préstamo
  español. El default histórico (`fixed_payments`, la cuota íntegra a principal, 0 % de interés)
  describía un producto que no existe y era lo que recibía cualquier alta sin tocar el
  desplegable.
- **Migración DATA-CHANGING firmada por el owner**: las filas `fixed_payments` que YA declaraban
  TIN > 0 y cuota mensual se convierten a `french` — su proyección empieza a cobrar los
  intereses que siempre anunciaron. El ejemplo canónico: una hipoteca de 200.000 € a 1.000 €/mes
  al TIN 3 % pasa de extinguirse en el mes **200 con 0 € de intereses** al mes **278 con
  ≈78.000 €** — el número honesto. Los dos números NO son intercambiables: bajar la cuota para
  que el francés también dure 200 meses cambia el producto entero. El TIN residual inexpresable
  como francés (cuota semanal o sin plan) se anula — el motor siempre lo ignoró — y un TIN
  fuera de cota (> 100 %: la errata «350» por «3,50») va al mismo residuo en vez de empezar a
  componer. Dos flancos más que la verificación adversarial encontró y la migración cubre: la
  fila convertida que llevaba el principal DERIVADO lo congela como explícito (era Σ cuotas — el
  número inflado; dejar el flag activo haría que el primer PATCH lo re-derivara a valor actual
  con una caída silenciosa de decenas de miles), y una `interest_only` SIN TIN (creable entre
  4.2.0 y 4.6.0) pasa a `fixed_payments` — la misma caja mensual que pagaba, solo que ahora
  amortiza; bajo el brazo nuevo habría pagado 0 €/mes con la deuda congelada y habría quedado
  ineditable.
- `fixed_payments` queda como lo que es —el préstamo **sin intereses (0 %)**— y **rechaza** el
  TIN (`apr_forbidden_for_model`): el «préstamo gratis silencioso» deja de ser representable.
- **`interest_only` es una carencia real**: la cuota del mes ES el interés del período
  (saldo × TIN/1200); la declarada solo topa por arriba y por debajo el déficit CAPITALIZA. A
  mano: 300 € de deuda al 6 % cuestan **1,50 €/mes** — hasta ahora salían 300 €/mes (252.000 €
  de «interés» sobre 300 € de deuda en 70 años). Con tope 400 € sobre 100.000 € al 12 %:
  cierres 100.600,00 y 101.206,00.
- **`revolving` cobra su cuota mínima real**: `max(min_payment_pct × saldo de apertura,
  min_payment_eur)` — columnas nuevas, exigidas al crear una revolving. A mano (TIN 18 %, mín
  3 % con suelo 30 €): saldo 3.000 € ⇒ cuota **90,00 €** y cierre 2.955,00; saldo 800 € ⇒ manda
  el suelo, **30,00 €** y cierre 782,00. Las revolving existentes reciben un backfill
  BIT-IDÉNTICO (pct 0, suelo = su cuota declarada = la recurrencia que ya tenían), pineado por
  test contra la francesa.
- El PATCH de `apr_percent` pasa a **tri-estado** (`null` limpia — patrón `purchase_price`), y
  al salir de `revolving` sus mínimos se anulan solos: sin ambas cosas, las filas quedarían
  atrapadas en su modelo. MCP: params `min_payment_*` y `clear_apr_percent` en
  `create_liability`/`update_liability`.
- El **import de `.ffbackup` ≤ v10 aplica la MISMA regla firmada** (tercer sitio del predicado):
  cada restore habría re-fabricado el estado que la migración elimina.
- La derivación del principal es **rama única**: valor actual de las cuotas al TIN
  (`present_value_of_payments`); sin TIN degenera EXACTO en Σ cuotas — bit a bit el caso
  `fixed_payments` de siempre.

### La etiqueta dice TIN (#122)

El campo siempre se consumió como **TIN nominal** (`i = apr/1200`, la convención correcta del
cuadro de amortización español) pero se rotulaba «TAE» en 5 superficies. Solo cambian etiquetas y
prosa — **ni una cifra**: formulario, columna, KPI («TIN medio ponderado»), schema MCP y textos
de ayuda. Cierra el escenario medido: teclear la TAE del contrato (3,26 %) donde va el TIN
(3,00 %) inventaba +11.000 € de intereses y 11 meses de deuda en una hipoteca de 200.000 €.

### Los vencimientos se cuentan desde el día ancla (#123)

`checked_add_months` encadenado degradaba el día 29-31 para siempre al cruzar febrero: un recibo
domiciliado el 31 giraba «13 cuotas» donde el banco gira 12. Ahora cada vencimiento se recalcula
desde el ancla (`hoy + n meses`). A mano: ancla 31-08-2026 → fin 30-08-2027 = **12** recibos
(13.000 € de principal derivado pasan a **12.000 €**); un día más de ventana y el 13.º sí entra.
Solo afecta a «derivar principal del plan» con día ancla 29-31.

### El plan vencido con saldo vivo sigue siendo deuda (#145) — BREAKING de cifras

La deuda no se extingue por calendario. El predicado de visibilidad de TODAS las lecturas pasa de
«plan vivo» a «plan vivo O saldo vivo»: el vencido con saldo aparece en listado, Resumen,
histórico y proyección — congelado (sin devengo ni cuota, decisión del owner: sin demora) y
marcado **`plan_expired_with_balance: true`**, con su chip en la UI. Solo el vencido y saldado
(principal 0) se oculta. Desaparece el salto de patrimonio de +saldo de un día para otro (48.000 €
en el escenario medido). El calendario del vencido-con-saldo deja de ser 404: se sirve congelado
con `payoff_absent_reason: no_payment_plan`. A mano en los tests invertidos: `total_liabilities`
100 → **10.099,0** (9.999 vencido + 100 activo); patrimonio −50.000 → **−59.999,0**; el
invariante `starting_net_worth == summary.net_worth` se CONSERVA. Excepción deliberada: el
presupuesto y Movimientos siguen filtrando por plan vivo (sus queries son de CUOTA, y un plan
vencido no gira cuota).

### Una sola base de coste de la deuda (#121)

Había tres copias de «¿cuánto cuesta esta deuda?» y las tres decían cosas distintas (hasta
83.000 € de intereses afirmados en portada y nunca cobrados por la simulación). Predicado ÚNICO
nuevo en el motor — `liability_interest_accrues`: modelo con intereses + TIN > 0 + plan vivo — y
lo consumen el `net_return` del Resumen y las dos KPIs de Pasivos (espejo TS
`liabilityAccruesInterest`). La fila que no devenga sigue pesando en el denominador, a coste 0. A
mano (dos-caras, el MISMO pasivo): activo 100.000 @5 % + francés 50.000 @5 % con plan vencido ⇒
**10,0000 %**; con plan vivo ⇒ **5,0000 %**. Préstamo sin intereses de 100.000 junto a 300.000
@4 % ⇒ **6,0000 %** (diluye, no resta). Interés mensual aprox.: 100.000 @6 % vivo + 40.000 sin
intereses + 9.999 vencido ⇒ **500,00 €** exactos.

### Amortización anticipada realista en el what-if (#151) — BREAKING del default

- **Compensación por reembolso anticipado**: default **2 %** del capital extra amortizado (el
  techo legal a tipo fijo, Ley 5/2019 art. 23; cota dura [0, 2]; opt-out explícito con `"0"`).
  Es la única línea de la ola que cambia el resultado de un caller existente de
  `simulate_projection`: el what-if de amortizar deja de ser gratis por defecto. La comisión sale
  de la caja y NO baja el principal — queda FUERA de la identidad `cuota + extra = interés +
  amortizado` del calendario. A mano: lump de 20.000 € ⇒ **400,00 €** exactos, cargados al mes
  del lump. KPIs nuevos `liability_early_repayment_fee_monthly`/`_total` + delta: «¿me compensa
  amortizar?» tiene ahora sus dos columnas (interés ahorrado vs comisión pagada). No se modela:
  la caída al 1,5 % tras el año 10, los topes de variable, la pérdida del prestamista.
- **«Reducir cuota»** (`early_repayment_effect: reduce_payment`): λ-escala la cuota por el factor
  que bajó el principal — en una renta francesa el plazo depende solo de `P·i/M`, así que con una
  amortización PUNTUAL el mes de extinción NO se mueve (teorema pineado: francés 150.000
  @2,5 %/800 € con lump de 20.000 € en el mes 12 extingue en el **239** con y sin amortizar; la
  cuota baja a **688,95 €** y libera 111,05 €/mes). Con extra RECURRENTE puede adelantarse algo
  (nunca atrasarse: el importe fijo cancela antes cerca del final). Sobre una revolving el efecto
  se rechaza (su caja es la cuota mínima, no la declarada que esto escala). El default
  `reduce_term` es bit-idéntico a 4.6.0.

### El modelo viaja al snapshot (#129) — `.ffbackup` 10 → 11

La interpolación histórica de pasivos usaba SIEMPRE la curva francesa; ahora usa **la ley del
modelo capturado** (`history_snapshot_items.repayment_model`, columna nueva): francés/revolving ⇒
curva compuesta; sin intereses/carencia/foto anterior a 4.7.0 ⇒ la cuerda — que para cuota fija
es exacta, no aproximada. A mano (50.000 → 26.000 en 365 días, TIN 15 %): al día 182 la curva
decía 38.361,75 €; la cuerda dice **38.032,8767 €**. El quiebro de pendiente falso al llegar a
«hoy» desaparece: pasado y futuro usan la misma ley. El `.ffbackup` sube a **v11** (el item de
snapshot lleva el modelo); v1..v10 siguen importando por la cadena completa, con `null` = «la
foto no lo sabía» ⇒ ley lineal. No se backfilla desde el ledger: el modelo de HOY no es el de la
foto.

### Una foto incompleta no desploma el patrimonio (#130)

Un item ausente de UNA captura valía 0 € en todo su tramo — un activo de 40.000 € que faltaba en
una foto desplomaba el agregado 40.000 € (test de integración con ese número). Ahora **arrastra
su último valor observado** (LOCF); la ÚNICA ausencia que vale cero es la del ledger vivo
(borrado/vendido de verdad, `last_is_live_ledger`). Y el empalme histórico↔proyección del chart
pasa de igualdad exacta de anclas a **mismo mes civil**: cruzar la medianoche ya no borra el
tramo histórico; cruzar la frontera de mes sigue siendo identidad (la rejilla se desplaza un mes
entero — un «±1 día» ingenuo fusionaría rejillas desalineadas).

### Tests

18 tests invertidos A PROPÓSITO con nota (contrato 4.2.0 → 4.7.0) — engine, purga, schedule,
modelo, backup v9, historia; suites nuevas de comisión, paridad de derivación (rama única),
LOCF de integración y roundtrip v11. Fixtures regenerados conscientemente: `mcp-catalog.json`
(params y descripciones nuevos), `error-codes.json` (+8 códigos),
`liability-derived-principal-parity.json` (el caso fixed+TIN retirado, con su porqué en el
propio fichero).

## [4.6.0] - 2026-08-31

**Ola 2 de la resolución — «Una sola fuente de verdad»** (issues #118, #119, #131, #132, #133,
#134 parcial, #136 parcial, #138 parcial, #147). El cliente deja de recalcular lo que el servidor
ya sabe, y los estados de fallo que el motor calculaba en silencio llegan al wire. **El motor no
cambia ninguna cifra**; cambian cifras MOSTRADAS que eran recomputaciones divergentes.

### Los estados de fallo llegan al wire (#119)

- `GET /v1/projection/series` publica lo que el motor ya calculaba y la superficie HTTP tiraba:
  `assets_depleted_month_index` (primer mes cuyo déficit iguala o supera TODO lo drenable — la
  cartera se vacía ese mes; pin a mano: 200.000 € al 0 % gastando 2.000 €/mes ⇒ mes **100**
  exacto y NW(360) = −520.000), `uncovered_deficit_total`, `liabilities_negative_amortization[]`
  (cuota < devengo ⇒ la deuda CRECE; pin: francés 200.000 €@6 %/800 € ⇒ P₁ = 200.200,00 — un
  `interest_only` congelado NO aparece, esa distinción es el campo) y `fire_target_absent_reason`
  (mismos literales que `simulate_projection` — paridad por construcción). `simulate_projection`
  gana el mes de agotamiento en ambos lados y su delta. Norma de la casa: NULL nunca es cero.

### La vista Jubilación lee el servidor (#118) — y el fixture gana la red que faltaba

- «Primer cruce», «Años hasta el cruce» y el objetivo leen los campos del servidor (exactos,
  serie completa); el cálculo local queda SOLO para la vista previa con ajustes sin guardar,
  marcada como tal. `grossUpNetAnnualFire` pasa de bisección f64 con techo mágico a la MISMA
  forma cerrada del servidor: con un tramo alto el techo saturaba EN SILENCIO y la misma pantalla
  podía enseñar un objetivo un 20 % más bajo que el del chart. El 10.º caso de `fire-parity.json`
  (tramo único abierto al 80 %) va rojo con la bisección (Δ 3,43 M€) y verde con la forma
  cerrada, en ambos lados. El cuarto cruce recalculado (`MiniProjection`) lee
  `jubilacion_series_position`; `findFirstMonthNetWorthAtLeastInflated` se borra.

### El jubilado ve su drawdown entero (#132)

- Regla del owner: el recorte «cruce + 12 meses» se mantiene mientras NO estás jubilado; con el
  cruce ya alcanzado (`jubilacion_month_index == 0`) el gráfico enseña el horizonte completo —
  los 348 de 360 meses que quedaban fuera eran justo la pregunta de esa vista. «Años hasta el
  cruce» pasa de `Math.round` a «años y meses» («5 meses», no «0 años»).

### El catálogo de métricas dice la verdad completa (#131, #133, #134, #138, #147)

- 6 entradas nuevas (rentabilidad esperada — «nominal, la que publica tu fondo, ya neta de
  comisiones»; ratio deuda/activos; los 4 KPIs de Pasivos con sus bases y divergencias
  declaradas) y 3 editadas: `summary.runway` y `settings.inflation` dejan de contradecirse
  (la Autonomía SÍ infla su gasto de comparación; la simulación NO infla flujos — hasta la
  Ola 5 —; el objetivo SÍ se infla), y los importes del presupuesto se declaran **NETOS** en
  etiquetas y ayuda (#147). Convenciones de #138 añadidas donde tocan.

## [4.5.0] - 2026-08-30

**Ola 1 de la resolución de la auditoría — «Las puertas y las formas»** (issues #95, #96, #97,
#99, #105, #113, #135, #137). Criterio de aceptación del tren: **cero euros de cambio** en ningún
agregado del motor — todo es contrato, validación y superficie.

### El `null` presente en un PATCH borra de verdad (#95, #113)

- Serde colapsa `"campo": null` con clave-ausente en `Option<T>`, así que las ramas `Value::Null`
  de SEIS campos eran código muerto: el contrato (OpenAPI incluido) prometía «`null` borra» y el
  binario devolvía **200 sin efecto** — en `birth_date` sobre un input del engine (el horizonte).
  Nuevos `deserialize_double_option`(+`_typed`) aplicados a `purchase_price`, `birth_date`,
  `amount`, `cap`, `due_date` y `fire_settings`; trío de tests por campo (null→borra,
  ausente→intacto, valor→aplica). Cambio observable: solo lo que antes NO hacía nada — y
  `amount: null` sobre una regla `fixed` pasa de 200 mudo a 400 (la validación que la rama muerta
  escondía). Las tools MCP siguen con sus flags `clear_*` (el tri-estado no es expresable en JSON
  Schema).

### El techo del cap se resuelve también sin sobrante (#96)

- `cap_ceiling`/`cap_room` salían `null` en meses sin caja y en jubilación, y eso obligó a
  duplicar la fórmula en el handler para publicar los objetivos. El motor los resuelve SIEMPRE
  (el techo depende de la regla y de los escalares, no de la caja) y la copia del handler queda
  como adaptador sin aritmética. Pin a mano: `MonthsExpense(6)` × gasto 1.500 ⇒ techo 9.000,00 €
  publicado también con caja 0. Breaking suave: la nulabilidad publicada de esos campos cambia.

### MCP: la familia entera pide `id` y habla inglés (#97) — breaking MCP

- `update_allocation_rule` y las tres tools de reglas de categorización pedían `rule_id` donde
  toda la familia usa `id` (un cliente que copiaba el `id` del listado recibía un error de campo
  desconocido), y el payload de confirmación emitía `{id, antes, despues}` — claves en español en
  el wire. Ahora `id` en las cuatro y `{id, before, after}`; catálogo congelado regenerado a
  conciencia.

### El owner-only de FIRE baja a la core (#99)

- La guardia de `update_fire_settings` vivía en la tool mientras la de
  `update_installation_settings` vivía en la core — el dual-branch drift que D14 nombra. Se muda
  a `patch_fire_settings_core`: protegida por construcción para cualquier llamante futuro. El
  error por MCP no cambia de forma (`member → forbidden`).

### Dos puertas de escritura se cierran (#135, parcial consciente)

- El import de `.ffbackup` valida el retorno con LA MISMA puerta que la escritura
  (`backup_asset_return_invalid`, con rollback y nombrando el activo): antes un backup con
  retorno ≤ −100 creaba la fila que dejaba la proyección en el overflow tipado del engine.
- `apr_percent` gana cota superior **100 %/año** (`apr_out_of_range`): ningún tipo real se acerca
  (usura ≈ 27 % TAE) y el vector real era el desliz de coma es-ES (350 por 3,50). Las puertas
  3-5 del barrido quedan razonadas en el issue (la forma cerrada del cliente llega en la Ola 2;
  `swr=0` es semántica documentada que #119 hará explicable; el literal corrupto tiene CHECK de
  columna y degradación documentada).

### Superficie y campos muertos (#105, #137)

- `fire_number_expense_adjustment_pct` se retira del tipo cliente (el servidor lo descartaba al
  deserializar; el cliente lo round-trippeaba a un agujero negro). `model_note` — los supuestos
  del modelo que viajaban tipados sin que nadie los renderizara — se pliega bajo el chart como
  «Supuestos del modelo». Las 6 sombras hardcoded pasan a tokens byte-idénticos y el freezer
  no-hex congela también `rgba(0,0,0` fuera de `theme.css`.

**Auditoría del modelo financiero — cubo «arreglar ahora»** (2026-08-30). La vara de medir fue la
realidad española (liquidación bancaria, escala del ahorro, IPC del INE), verificada con un oráculo
independiente que coincidió con el engine en 2.131/2.131 filas: el motor implementa exactamente su
contrato; lo que se corrige aquí son los puntos donde una superficie mentía sobre ese contrato o
donde un input válido lo rompía. El resto de divergencias de modelo quedan contabilizadas —
cada una con su coste en un escenario sintético y su issue — en el nuevo
[`.claude/financial-contracts.md`](.claude/financial-contracts.md) §4.

### Engine — desbordar componiendo una tasa absurda era un panic, ahora es un 400 tipado

- Sin cota superior de `expected_annual_return_percent`, un valor desorbitado (fat-finger `7.125`
  leído como 7.125 %, o un import) hacía que `values[i] *= m` desbordara `Decimal` y **panicara**:
  el pool blocking lo servía como 400 `task_panic` permanente que tumbaba `/v1/projection/series`,
  el chart, los KPIs de jubilación y la tool `get_projection` hasta editar el activo a mano. Ahora
  `checked_mul` + `EngineError::AssetValueOverflow` → 400 `engine_rejected_input` con causa
  legible. Deliberadamente NO se satura como el payoff de un pasivo: congelar el valor publicaría
  un patrimonio gigante y plausible. Ejemplo pineado: 1.000 € al 1000 % anual desborda en el mes
  ~298 de 840 → error tipado, ningún input válido cambia de valor.

### Engine/API — la resolución de la cascada decía aportaciones que la simulación jamás hace

- En jubilación con superávit el bucle manda TODO el sobrante a `surplus_cash` y la cascada no se
  ejecuta; pero `first_month_allocation` la ejecutaba igualmente, y `GET /v1/assets` y
  `GET /v1/allocation-rules/resolution` (y sus tools MCP, que comparten core) publicaban «aportas
  600 €/mes → Fondo» para un hogar ya por encima de su número FIRE. Ahora: `per_asset` a cero,
  sobrante íntegro en `leftover`, y cada regla con la razón nueva **`in_retirement`** — distinta
  de `no_cash` a propósito, porque caja HAY (viaja en `base_cash`). Cambia cifras publicadas SOLO
  para hogares en fase FIRE (antes eran ficticias). Ejemplo pineado a mano: NW 200.000 ≥ target
  100.000, pensión 2.200 − gasto 1.600 ⇒ per_asset [0], leftover 600, y el bucle coincide
  (NW(1) = 200.600 con el activo intacto).

### API — el desglose por activo no era determinista con empates

- `ORDER BY sort_index, name` no es un orden total: dos activos empatados podían salir en orden
  distinto entre peticiones, y ese orden alimenta `per_asset_series` y el desempate del drenaje.
  Ahora `ORDER BY sort_index, name, id` (proyección y `GET /v1/assets`). Agregados idénticos,
  0 € de cambio.

### Web — «6.000 €» en un umbral de tramo eran seis euros

- El input «Hasta base (€)» de la escala del ahorro hacía `replace(",", ".")` a pelo en vez de
  pasar por `toApiDecimalString`: la escala entera tecleada a la española
  (6.000/50.000/200.000/300.000) llegaba como 6/50/200/300 €, seguía siendo creciente, pasaba la
  validación y el objetivo FIRE subía ~+13 % **en silencio** (escenario sintético: 24.000 € netos
  → objetivo 978.852 € en vez de 863.653 €). Ahora usa la función canónica de la app. El campo de
  porcentaje del tramo no cambia (decisión del owner: la regla de millares de los campos % queda
  como trampa documentada).

### Paridad y prosa

- `fire-parity.json` ejercita por fin los tramos del 27 % y del 30 % (2 casos nuevos con la
  aritmética en `_calc_note`; el gross-up TS por bisección coincide con la forma cerrada del
  servidor también ahí). Hasta ahora el fixture topaba en gross 146.597 € y el lado cliente por
  encima de 200.000 € iba sin red.
- Dos erratas que el código desmentía: el texto de ayuda del Rendimiento neto omitía que la
  proyección solo devenga interés **con plan de pagos vivo**, y `.claude/engine.md` decía que el
  drenaje toca solo los líquidos cuando siempre ha continuado sobre los ilíquidos.

### Documentación

- Nace **`.claude/financial-contracts.md`**: contratos financieros canónicos (unidad, convención y
  por qué refleja la realidad española, con fuentes BdE/BOE/INE), lo que el modelo YA acierta y no
  hay que «arreglar», y la tabla de divergencias conocidas con su coste sintético, su estado y su
  issue. La tabla Phase 1 de `futurefin-projection-realism-campaign` se reduce a un puntero (dos
  copias fueron lo que dejó su fila #4 siendo falsa durante meses).
- Arnés permanente de auditoría: `crates/engine/tests/audit_dump.rs` vuelca en CSV los casos
  límite del modelo (vencimiento con saldo vivo, cuota < interés, degeneración sin TIN, déficit
  crónico, FIRE en mes 0, bordes ±100 %…) para compararlos con oráculos externos.

## [4.4.1] - 2026-08-30

**Cuatro verdades restauradas** — el lote «cero comportamiento observable» de la auditoría de
coherencia contractual (2026-08-30; la parte de docs viajó aparte, PR #104). Ningún número cambia;
cambia lo que el binario DICE de sí mismo.

### MCP — el schema de `get_projection.months` prometía clamp donde la core rechaza

- **El texto que lee el modelo mentía**: el doc-comment del parámetro (`server.rs`) decía «fuera de
  rango se clampa», cuando un `months` fuera de 12–840 es **400 `months_out_of_range`** desde
  4.4.0 (el clamp silencioso se retiró a propósito — la respuesta afirmaba «te he hecho caso»
  mientras contestaba otra pregunta). Misma clase de bug que el «default 15 vs 30» que motivó la
  norma de la casa, en otro parámetro. Sin cambio de comportamiento: la tool ya rechazaba (comparte
  `projection_series_cached`); solo se corrige la promesa. El fixture congelado no se mueve — las
  descripciones de parámetro no entran en él (las constraints, que sí, ya eran correctas).

### API — un error sin código estable y un literal que podía mentir

- **`category_not_in_installation`**: la validación de `category_id` en movimientos devolvía su
  error **sin prefijo `snake_code:`** — caía a la clase genérica `bad_request` mientras su
  validación hermana (`category_scope_mismatch`, mismo bloque) sí llevaba código. 359/362 del
  resto del binario cumplían; este era uno de los tres que no. Con entrada en el catálogo de
  traducciones de la SPA y en `error-codes.json` (regenerado). Cambio observable mínimo: el campo
  `code` de ese error pasa de `bad_request` a `category_not_in_installation` (el status 400 y el
  mensaje no cambian).
- **`schedule_window_out_of_range` interpola su cota**: el mensaje decía «between 1 and 480» con
  el 480 escrito a mano al lado de `MAX_SCHEDULE_WINDOW_MONTHS = 480` — si la constante cambiara,
  el mensaje mentiría en silencio. Ahora interpola la constante (bytes idénticos hoy).

### `/v1/changes` — la tercera categoría que el contrato no contemplaba

- El doc-comment del módulo prometía «ninguna tabla se omite en silencio» con dos listas
  (`covered` / `missing_updated_at`), y `persons` no estaba en ninguna: tiene `updated_at` pero
  no es ledger y hoy nada la escribe. Queda **excluida por diseño y documentada** en la cabecera
  del módulo, con la instrucción de revisar la exclusión el día que algo escriba en ella. No entra
  en `tables_covered`: ampliar esa lista cambiaría el wire por una tabla muerta.

## [4.4.0] - 2026-08-29

**Revisión adversarial del servidor MCP embebido** (issues #81–#88). Cinco agentes independientes
auditaron `/mcp` desde ángulos distintos —caja negra en vivo contra una instalación real, escrituras
bajo protocolo de crear-y-borrar, catálogo y paridad sobre las 3.300 líneas de `server.rs`,
protocolo y transporte, y frontera de capacidades— y salieron ~110 hallazgos. Se arreglaron en siete
fases, cada una con sus tests y su documentación.

**Qué cambia para ti.** El catálogo pasa de **52 a 68 tools**: ahora se puede preguntar cuánto
cuestan tus deudas en intereses y cuándo terminan, simular si compensa amortizar antes, sumar gastos
con filtros sin que el modelo tenga que contar filas a mano, ver el patrimonio en euros de hoy,
grabar el pasado desde la conversación y encaminar aportaciones a un activo nuevo. Y varias cifras
que salían mal ahora salen bien: la que decía que tu patrimonio histórico era otro, la que hacía
inobtenible el objetivo FIRE del mes en que te jubilas, y la que reportaba un mes sin datos como un
mes de gasto cero con deltas enormes.

**Qué cambia si dejas correr agentes.** Antes, un timeout duplicaba un movimiento que mueve tu fecha
de jubilación, un borrado masivo no dejaba rastro de quién lo hizo, y el `confirm` era un booleano
que rellenaba el propio modelo. Ahora hay auditoría de escrituras, credenciales de solo lectura,
confirmación en dos fases donde el daño es irreparable, e idempotencia opcional.

Las descripciones del catálogo pasan de 37 KB a 21 KB **sin perder ningún aviso**: los que evitaban
un error se movieron a campos de la respuesta, que es donde el modelo los lee en el momento de mirar
la cifra.

### Red de seguridad del catálogo MCP (Fase 0 — issue #81)

- **Por qué existe**: la invariante «toda tool de escritura pasa por `require_mcp_write`» solo
  vivía en un `grep` de una skill, y el catálogo congelado solo congelaba **nombres**. Una tool
  nueva que olvidara el gate pasaba la CI en verde, y una descripción se podía volver falsa sin
  que fallara nada — ya ocurrió tres veces en 4.0.0, y las tres se encontraron a mano.
- **Traza de escritura**: `require_mcp_write` recibe ahora el nombre de la tool y emite
  `tracing::info!` con tool, usuario, rol y credencial **antes** de resolver el gate, así que
  también quedan registrados los intentos rechazados. Sale a nivel `info`, el de la imagen por
  defecto: un operador lo ve sin reconfigurar nada. Hasta ahora un borrado por MCP no dejaba
  ningún rastro de qué tool lo hizo. `McpCredential` deja de ser `dead_code`.
- **Dos tests nuevos con dientes**: uno tabla-guiado que recorre `tools/list` y exige que las 31
  tools de escritura rechacen a un `viewer` (`forbidden`) y al toggle apagado
  (`mcp_write_disabled`); y otro **estructural sin BD** que trocea `server.rs` y exige que toda
  tool `read_only_hint = false` llame al gate **nombrándose a sí misma**, fijando de paso los
  contadores 52/21/31/31/11.
- **El contrato de entrada queda congelado**, no solo los nombres: nuevo fixture
  `apps/api/tests/fixtures/mcp-catalog.json` con las claves de `inputSchema`, el `required` y un
  hash de la `description` por tool. Regenerable con `UPDATE_MCP_CATALOG=1` (patrón de
  `UPDATE_ERROR_CODES=1`).
- **Cobertura de lo que nadie ejercitaba**: `delete_liability` (destructiva, con preview) no se
  invocaba en ningún test; el preview de `delete_planning_flow` no se ejecutaba jamás; `get_budget`
  y `list_liabilities` no tenían paridad byte a byte contra su GET.
- **Contenido de terceros marcado como dato**: las `instructions` del servidor advierten ahora de
  que `concept`, `notes`, `category_name`, `pattern` y los nombres de activos, pasivos y categorías
  contienen texto importado de extractos bancarios —el concepto de una transferencia recibida lo
  escribe quien la envía— y nunca son instrucciones.
- Test `every_input_schema_forbids_unknown_properties` añadido **como `#[ignore]`**: hoy 51 de 52
  tools aceptan campos desconocidos en silencio. Es la diana de la Fase 2 (#83).

### Deriva documental: verificar en vez de escribir (Fase 7 — issue #88)

Última fase. Los ocho puntos del issue ya estaban cerrados por los barridos de las fases 4–6, así
que ésta no fue escribir documentación: fue **comprobar afirmación por afirmación** lo que siete
fases de cambios habían dejado obsoleto.

- **Se ejecutaron los 528 comandos de re-verificación** que las skills llevan en sus secciones de
  procedencia. Varios ya no encontraban nada — y un `grep` que no encuentra nada es **deriva
  silenciosa**: o el comando está mal escrito (uno abortaba por un corchete sin escapar, otro
  buscaba en un fichero donde la función ya no vive) o describe algo que se retiró. Tres daban
  falsos positivos dentro de otra palabra, que se leen como «esto ya se empezó».
- **La afirmación más cara: «CI no corre los tests de integración contra Postgres».** Es falsa
  desde 4.0.0 y estaba en **cinco skills**. Cualquiera que la leyera se saltaría evidencia creyendo
  que no existe.
- **«Las rentabilidades negativas se clampan a 0 %»** también era falsa: el motor las compone hacia
  abajo y tiene test propio desde hace tiempo. El grep de re-verificación de esa misma fila salía
  vacío y nadie lo había mirado.
- El registro de paridad tenía cuatro filas bajo el título **«gaps pendientes»** que el texto de
  debajo declaraba cerradas: quien escaneara la tabla veía cuatro huecos que ya no existían.
- Y una que llegaba al usuario final: `SECURITY.md` y `docs/mcp.md` enumeraban **siete** tools con
  `confirm_token` y omitían la octava — la misma errata que el servidor ya había corregido en su
  `instructions`.

Además, arreglado en código lo que no era documentación: el esquema de `suggest_transfer_matches`
—el texto que **lee el modelo**— anunciaba «default 15» cuando el real es 30, y topaba en 60 días
mientras la core acepta 365, así que `window_days: 90` funcionaba por HTTP y fallaba por MCP. Y el
hex hardcodeado de los tooltips pasa a ser un token con el mismo valor en los dos temas: era una
excepción legítima, pero tácita, y una excepción que no se puede nombrar acaba copiándose a sitios
donde el tema sí importa.

**Cinco incoherencias código↔contrato quedan abiertas como issues** (#95, #96, #97, #99), todas
preexistentes y ninguna arreglable desde documentación.

### Capacidades nuevas: el catálogo pasa de 52 a 68 tools (Fase 6 — issue #87)

La paridad de **rutas** era casi perfecta, pero paridad de rutas ≠ paridad de **capacidades**: la
app calculaba cosas que no eran una ruta y que el chat no alcanzaba. **Cuatro de las cinco más
valiosas eran código que ya existía y solo necesitaba superficie.**

- **`get_liability_schedule`** — el engine calculaba el principal de cierre de cada pasivo hasta
  840 veces por request **y lo tiraba**, así que «¿cuánto pago de intereses?» y «¿cuándo termino la
  hipoteca?» eran **incontestables** en una app de finanzas personales. El calendario deriva el
  interés como **residuo** de los saldos, no lo devenga aparte: así `cuota + extra == interés +
  principal` es exacto **por construcción** en los cuatro modelos, y si alguien recalculara el
  devengo por su cuenta la igualdad se rompería justo en el mes en que las dos implementaciones se
  separaran. `principal_repaid` puede ser **negativo** cuando la cuota no cubre el devengo:
  clamparlo escondería exactamente ese caso. Contrastado contra la fórmula cerrada de la anualidad
  francesa, con los números predichos antes de ejecutar.
- **`liability_overrides` en `simulate_projection`** — «¿me compensa amortizar antes?». Había 12
  ejes de what-if y **ninguno tocaba pasivos**; lo más cerca era un gasto puntual, que drena caja
  pero no reduce deuda ni cuota. La cuota liberada al extinguir vuelve a la cascada, y **eso no es
  una decisión nueva**: es lo que el motor ya hacía con un préstamo que se acaba solo. Suprimirlo
  habría exigido *añadir* código para esconder caja que el modelo tiene, y habría hecho que dos
  préstamos en el mismo estado de balance se comportaran distinto. Contrapartida obligatoria: la
  amortización extra **se cobra a la caja del mes** — las dos mitades o ninguna, porque hacer solo
  la que baja el principal *imprimiría dinero*.
- **`aggregate_transactions`** — «¿cuánto llevo gastado en X este año?» obligaba a bajar hasta 500
  filas al contexto y sumarlas con un modelo que **no aplica** el predicado de transferencias
  conciliadas. En el escenario del test, ese olvido convierte un gasto mensual de 180 € en **680 €**:
  creíble y falso. La suma se compara **iterando** las categorías de `get_transactions_summary`, no
  contra constantes: si las dos no cuadran, una miente.
- **Deflactado servido** — el servidor ya deflactaba (es lo que produce `milestones_real`) y solo
  salía al aire ahí. Ahora `net_worth_real` va **dentro de cada punto**, que ya lleva su
  `month_index` — un array paralelo obligaría a alinear por posición, que es el bug que ya se pagó
  en la v1.4.2. Es **capa de presentación**, no el motor «real puro» rechazado en la v1.2.0 por
  drenar activos antes de la jubilación, y la forma testable de esa afirmación es que
  `net_worth_real` no lleva información que el motor no haya producido.
- **Trío de descubrimiento y conciliación sin footgun.** `uncategorized` no existía ni en HTTP;
  ver un par candidato de transferencia obligaba a **escribir**. Y la omisión histórica de
  `reconcile_pair` se cierra sin exponer dos UUID: `confirm_transfer_match` acepta **solo un
  `match_id` emitido por el servidor**, así que un par arbitrario **no es expresable en el
  esquema**. No es una barrera: es hacer imposible el error que motivaba la omisión.
- **Cuatro filas del registro de paridad, cerradas**: backfill y edición de snapshots (el
  «diferencial conversacional»: grabar el pasado es lo que el chat hace mejor que un formulario),
  crear y borrar reglas de reparto, editar y borrar categorías con `remap_to`, y los ejes de
  presentación de la instalación.
- **`prompts`** con tres flujos reales. Es el único punto del protocolo donde se gana **capacidad**
  y no formato. Aviso honesto: **el conector remoto de claude.ai no los expone hoy** (sus docs lo
  dicen); Claude Code y los clientes genéricos sí.

**Al encapsular el invariante del sumidero aparecieron tres agujeros reales.** Dos preexistentes: el
`PATCH` podía dejar el sumidero **en medio** de la cascada —y todo lo de debajo dejaba de recibir,
en silencio—, y la guardia del `reorder` **no comprobaba nada en vista de hogar** porque derivaba el
scope de la vista en vez del owner. Y uno que introdujo esta misma fase: la puerta que impide crear
el sumidero desde MCP era **saltable en dos pasos** (crear un `remainder` con tope y quitárselo
después), porque la política solo llegaba a la core de creación. Los tres se cierran con la misma
doctrina —post-condición sobre el estado resultante, con un **único punto de commit** en el módulo
fijado por un test que lee el propio fichero— y el tercero lleva su regresión de dos pasos, porque
una guardia probada solo en el `create` deja verde cualquier test que solo pruebe el `create`.

También: `delete_category` con `remap_to` **ignoraba el remap en silencio** cuando no había
referencias contadas, degradando a `NULL` la atribución de las cuotas de pasivo — justo lo que el
remap venía a evitar. Y el `instructions` del servidor enumeraba **siete** tools con `confirm_token`
y omitía la octava: es el único texto que toda sesión lee.

### Coste de contexto y ergonomía del catálogo (Fase 5 — issue #86)

El servidor defendía su corrección con **prosa**, y esa estrategia fallaba justo donde importa: en
la auditoría en vivo, la descripción de `get_summary` llegó al cliente **truncada**, y lo que quedó
fuera empezaba en mitad de una advertencia sobre inconsistencia entre tools.

Confesión de parte: **las fases 1–4 de este mismo trabajo empeoraron el problema** —de 27 KB a
36 KB de descripciones—, porque cada arreglo de una cifra añadía su aviso a la prosa. Esta fase
paga esa deuda.

- **Descripciones: 37.214 → 21.319 caracteres (−42,7 %)**, ninguna por encima de 600 (antes las superaban **26**,
  y la mayor eran 3.821). La idea no es «escribir menos» sino aplicar del todo lo que este servidor
  ya había inventado a medias: **campos de procedencia** en la respuesta. Un campo así le dice al
  modelo de dónde sale la cifra **en el momento en que la mira**, en vez de cobrarle el contexto en
  cada turno. Los 30 avisos retirados fueron a un campo, al `instructions` (una vez, en lugar de
  repetidos en doce descripciones) o al CHANGELOG, si lo que contaban era historia.
  Guardia nueva `tool_descriptions_stay_within_the_context_budget`, con la instrucción escrita de
  **no subir la constante** cuando falle.
- **Hallazgo que reordena lo que queda por hacer**: medido después del recorte, el `inputSchema`
  son 55 KB, **2,7× las descripciones**. La prosa ha dejado de ser el coste dominante; la palanca
  que queda son los ~250 doc-comments de parámetros.
- **`view` en la raíz de las respuestas.** Verificado en la auditoría: `?view=mine` y omitirlo
  devolvían payloads **byte a byte idénticos** en una instalación de un usuario, así que era
  imposible distinguir «mine == household» de «el parámetro se ignoró». En un hogar de dos, ésa es
  la pregunta que decide si la cifra es correcta.
- **Plan vs real, declarado en el dato.** Cuatro campos de `get_budget.totals` tienen nombre
  idéntico a cuatro de `get_summary.financial_health` y valen otra cosa. En vez de renombrar
  (breaking, sobre seis campos que la SPA lee, y que **no** haría la cifra más legible: seguirías
  sin saber en qué modo está el summary), ahora ambos declaran su `basis`: si
  `financial_health.basis != "plan"`, las dos cuartetas no son comparables. Comprobable en el dato.
- **El histórico dejó de traer el peor caso por defecto.** Sin `window_months` devolvía la serie
  **desde la fecha de nacimiento del usuario** —~290 puntos, con los primeros 200 interpolando
  entre 0 € y unos cientos, a 15 decimales cada uno—. Default de 120 meses (`1200` sigue
  significando «todo») y series redondeadas a 2 decimales: **−70 %** en el peor caso medido, de
  53,6 KB a 16,1 KB. La curva fina del cash-flow, acotada a 36 meses: **−69 %**, de 64 KB a 20 KB —
  y se **omite con motivo** en vez de dar error, porque el agregado mensual seguía siendo servible.
- **`simulate_projection`: `net_monthly` devolvía el ahorro del baseline**, un delta de exactamente
  0 en el campo que el usuario preguntó. No se podía «arreglar» sin mentir —está definido como
  `income − expense_total`, y los ejes de caja no tocan ni el ingreso ni el gasto—, así que pasa a
  llamarse `net_recurring_monthly` (identidad intacta, nombre honesto) y aparece
  `net_cash_monthly`, que es el que sí se mueve. Y gana `model_note`, que es donde más faltaba:
  bajar la inflación adelanta la jubilación años porque el motor capitaliza en nominal y **solo el
  objetivo FIRE** se infla — subes la rentabilidad real de todo y congelas el objetivo, gratis.
- **Sentinelas que engañaban**: `items: []` era indistinguible de «sin ítems» (ahora
  `items_included` + `item_count`); los markers no distinguían una foto de la app de un valor
  tecleado a posteriori (`source: capture|backfill`, que es lo que hacía que a «¿cuándo empecé a
  ahorrar?» las tools respondieran con un ancla de backfill); el mes en curso vacío desaparecía de
  `list_transaction_months` pero **sí** consumía slot en las series, así que la lista contradecía a
  las curvas; y `"(sin etiqueta)"` era un literal español **dentro de un campo de datos** — ahora
  es `null`.
- **La densidad híbrida escondía saltos grandes sin explicarlos** (en la auditoría, una caída de
  ~98 k€ entre dos puntos anuales). Se añade `events` con los planning flows datados que mueven la
  curva. Se descartó exponer `density` como parámetro: traería 841 puntos y seguiría sin decir
  **por qué** cayó, solo dónde.
- Paginación en `list_snapshots` y `list_transaction_imports`, que crecían sin cota ni `total_count`;
  `possible_duplicate_of` para detectar el doble import que sus propios datos delataban; y
  documentados los campos que se devolvían sin que nadie los explicara (`upcoming_*` —que no tienen
  ventana temporal y pueden mezclar un evento a 16 años vista con un runway de meses—,
  `horizon_basis`, `compound_outpaces_true_savings_month_index` —que es un **mes**, no una posición
  de array— y `value_date`, que es informativa: ningún agregado la usa).

### Transporte, CORS y kill-switch (Fase 4 — issue #85)

Nada de esto era explotable sin credencial válida. Son fallos de plataforma y de diagnóstico:
cosas que, al activarse, se leen como una avería.

- **El kill-switch no fallaba limpio en la imagen que se publica.** Con `FUTUREFIN_MCP_ENABLED=0`
  las rutas se desmontaban, así que `POST /mcp` devolvía un **405 vacío** y
  `GET /.well-known/oauth-authorization-server` devolvía **`200 text/html`** — el shell de la SPA,
  porque `ServeDir` no llama a su fallback para métodos distintos de GET/HEAD. El conector fallaba
  al parsear JSON y decía «connection failed» sin causa: **un control de seguridad que al activarse
  se diagnostica como avería**. Ahora las rutas se montan siempre y el handler responde **404 JSON
  `mcp_disabled`**, que es la doctrina que D18 ya aplicaba a `/v1/auth/sso`. El test viejo no lo veía
  porque construía el router **sin la SPA**: describía un binario de laboratorio, no el publicado.
- **OAuth quedaba irreparable bajo un proxy con subpath**: el prefijo público no entraba en el
  issuer ni en los endpoints anunciados, y la salida manual estaba cerrada porque
  `FUTUREFIN_PUBLIC_URL` hacía `panic!` con un path. Ahora lo admite. Se eligió eso y **no**
  componer el prefijo del request: el issuer es una **identidad**, no decoración, y bajo el Ingress
  de Home Assistant el prefijo lleva un **token efímero de sesión** que quedaría horneado dentro.
  Con esa decisión, además, ninguna cabecera entra ya en el path del issuer.
- **`CORS_ORIGINS` gobernaba dos superficies con una sola lista** y `allow_credentials(true)`: añadir
  un origen para hacer funcionar un cliente MCP de navegador concedía de paso acceso **con cookie**
  a `/v1/backup/user-export` y `/v1/api-tokens`. Ahora son dos capas, y la de `/mcp` va **sin
  credenciales** — no tienen sentido en una superficie autenticada por header.
- **Validación de `Origin` activada** en `/mcp` (la defensa anti-DNS-rebinding que el spec pide).
  Dato que decidía si esto rompía a Claude Desktop y a Claude Code: una request **sin** `Origin`
  pasa aunque la lista no esté vacía, así que los clientes sin navegador no se ven afectados.
- **Preflight completo**: faltaban `MCP-Protocol-Version` (obligatoria desde la revisión 2025-06-18)
  y `Last-Event-ID`, y sin exponer `WWW-Authenticate` un cliente de navegador no puede leer el
  `resource_metadata=` del 401 y **nunca descubre el authorization server**.
- **El tope de body de `/mcp` era 4 MiB, no el 1 MiB que declaraba el invariante**: `DefaultBodyLimit`
  de axum va por extractores, y `/mcp` es un `route_service` que leía con el default del SDK.
- **Metadata OAuth sin `Cache-Control: no-store`**, pese a que la documentación afirmaba que toda
  respuesta OAuth lo llevaba. Ahora es cierto, y con `Vary` cierra el vector de envenenamiento del
  issuer vía `X-Forwarded-Host`. No se exige peer de confianza para esas cabeceras: no conceden
  autoridad, solo reflejan — y lo único que hacía peligrosa esa reflexión era la cacheabilidad.
- **Sin GC de credenciales OAuth**: cada rotación de refresh insertaba dos filas que no se borraban
  jamás. Ahora se podan en `POST /oauth/token`, nunca en un GET. El refresh aguanta **30 días**
  porque la reuse-detection mira `consumed_at` antes que la expiración y necesita la fila viva.
- Las sesiones de Streamable HTTP siguen **sin ligarse a la credencial, por decisión razonada**: hoy
  no compra nada (el Bearer corre antes en cada request y el servidor no emite nada por iniciativa
  propia) y es una capa que SEP-2567 está retirando. El disparador para reabrirlo queda escrito en
  el código: la primera capacidad server→cliente.

### Escritura segura y automatización desatendida (Fase 3 — issue #84)

La fase que decide si se pueden dejar correr agentes sin nadie delante. El diagnóstico de la
auditoría era que **si algo salía mal —modelo confundido, token filtrado, inyección exitosa— el
sistema no lo limitaba, no lo detenía y no lo registraba**. Las tres cosas se cierran aquí.

- **Auditoría de escrituras** (`mcp_write_audit`). Un token podía borrar el ledger entero del hogar
  sin dejar rastro de quién, qué ni cuándo: `delete_transaction` es *hard delete* y el único
  registro era un `last_used_at` con throttle de 60 s. Ahora cada escritura deja fila con **quién,
  con qué credencial, con qué rol vivo en ese momento** (sin eso, un log de hace tres meses se
  leería con los permisos de hoy), qué tool, el desenlace y los UUIDs mutados. **Nunca los
  argumentos**, ni en claro ni hasheados: un log append-only con conceptos bancarios convierte el
  borrado del usuario en una mentira —el concepto que borró seguiría vivo un año fuera del backup
  cifrado— y un digest de fecha + importe + concepto es fuerza-brutable por baja entropía, o sea la
  misma fuga con una capa de tranquilidad falsa. El esquema es **tipado sin texto libre a
  propósito**, para que la regla no dependa de que el siguiente se acuerde de ella.
  El orden no puede mentir: la fila nace `attempted` —jamás `ok`, porque en ese instante la
  operación no ha corrido— y se cierra después; `settled_at IS NULL` significa exactamente «sigue
  en vuelo o el proceso murió», garantizado por CHECK, y el cierre es *write-once*. Todo
  best-effort: un fallo del log nunca tumba la escritura del usuario.
- **Credenciales de solo lectura.** No se podía emitir un token de lectura sin degradar a la
  persona a `viewer`, que le quita también la web: un token para «que Claude me analice los gastos»
  podía ejecutar las 31 escrituras sobre el hogar entero. `api_tokens.scope` con default que
  preserva **exactamente** los tokens existentes, y selector en Ajustes → Integraciones.
  **OAuth no se extiende**: ahí el scope lo pide la aplicación cliente —el lado del agente—, no la
  persona, así que sin pantalla de consentimiento donde estrecharlo no restringe nada, y
  anunciar `scopes_supported` sería mentir en la metadata.
- **`create_transaction` idempotente (opt-in).** Un reintento tras un timeout creaba una segunda
  fila y devolvía éxito, y en modo B/C los movimientos **son inputs del motor**: el duplicado
  inflaba el promedio y retrasaba la jubilación proyectada, en silencio. Con `idempotency_key`, el
  mismo cuerpo reproduce la fila original; un cuerpo **distinto** con la misma clave es 409 y gana
  el primero (devolver la original diría «tu segundo movimiento se creó», que es una mentira que se
  materializa como un gasto que falta). La huella se calcula sobre el cuerpo **ya validado**, así
  que `"-12.50"` y `"-12.5"` son el mismo reintento. Ámbito por usuario, no por instalación: con
  ámbito de instalación la clave de un miembro reproduciría el movimiento de otro y le devolvería
  una fila ajena.
- **El preview deja de ser saltable en lo irreparable.** `confirm: true` era un booleano que rellena
  el propio modelo: *prompting*, no un control. Ahora el preview devuelve un token de un solo uso
  ligado al hash de los efectos, **recalculados en la confirmación** — así se cierra también la
  ventana de que los efectos cambien entre las dos llamadas. Se exige en **7 de las 14**, no en
  todas: el criterio no es «destructiva» sino «confirmar sin mirar destruye algo que la conversación
  no puede reconstruir». Duplicar los viajes de cada borrado trivial haría que la ceremonia se lea
  como ruido, y una salvaguarda que se lee como ruido deja de serlo.
- **`confirm` en las tres destructivas que no lo admitían** (previews 11 → 14). `materialize_recurring`
  **poda instancias de toda la instalación** y no tenía ni struct de parámetros, así que `confirm`
  era literalmente inexpresable; `unreconcile_transfer` es irreversible y el cliente solo tenía el id
  de una pata, así que confirmaba a ciegas cuál era el par — ahora ve las dos. Para
  `materialize_recurring` el preview honesto **no era posible** (su core calcula y escribe en la
  misma transacción), así que publica los contadores como `null` **con el motivo**: un `null` sin
  motivo se lee como cero, e inventar una estimación habría sido peor que declarar el límite.
- **Bloque `impact`** en las escrituras que mueven el motor: un `create_liability` movía cuatro
  cifras de `get_summary` sin mencionar ninguna, así que el agente reportaba «pasivo creado» como si
  fuera inocuo. Sale de `summary_core` y **nunca de la proyección**: incluir la fecha de jubilación
  costaría una simulación de hasta 840 meses por escritura, justo después de que esa escritura haya
  invalidado la cache.
- **Techo de concurrencia para la proyección.** `simulate_projection` lanzaba dos `spawn_blocking`
  sin permiso; un agente en bucle podía vaciar el pool de 10 conexiones y, como `/v1/ready` usa ese
  mismo pool, dejar el contenedor *unhealthy* y reiniciándose. El semáforo envuelve la simulación y
  no el handler, así que una lectura cacheada no lo toca.
- **Dos errores que decían lo contrario de la verdad**: `delete_budget_entry` sobre la cuota de un
  pasivo devolvía **404 «no existe»** sobre un id que el cliente acababa de leer en nuestra propia
  respuesta de `GET /v1/budget` (las cuotas viajan con el UUID del pasivo). No borraba nada, pero
  mandaba a buscar un fantasma. Y el preview de `delete_liability` omitía que se lleva por delante
  la partida de presupuesto — para una hipoteca, cientos de euros al mes que el agente no contaba.

### El esquema como contrato (Fase 2 — issue #83)

El servidor defendía su corrección con **prosa**: ~27 KB de descripciones sobre un esquema casi
decorativo. El cliente lee el esquema **antes** que la prosa, y a veces la prosa se trunca.

- **51 de 52 tools aceptaban campos desconocidos en silencio.** `delete_asset {id, confirmed: true}`
  —un typo por `confirm`— devolvía un **preview** que el modelo podía leer como «borrado hecho»;
  `update_budget_entry {id, ammount: "250"}` devolvía `200` sin cambiar nada; `list_transactions`
  con un filtro mal escrito devolvía la primera página **sin filtrar**. Ninguno fallaba. Ahora las
  52 publican `additionalProperties: false` (**breaking**). Cuatro tools ni siquiera tenían struct
  de parámetros, así que el atributo solo no bastaba: se les dio uno.
- **Ningún enumerado era un `enum`.** Los ~30 parámetros enumerados eran `Option<String>` con la
  lista solo en la prosa: el modelo tenía que leerla para acertar, y fallaba una vez antes.
  Ahora el `enum` viaja en el esquema **conservando el error tipado** de runtime — la alternativa
  (tipar el parámetro con el enum de dominio) habría movido el error al fallo de deserialización
  de rmcp y **borrado seis códigos del catálogo**, degradando la SPA al mensaje genérico. La
  decisión de 4.2.0 sobre `repayment_model_invalid` ya había elegido ese camino a propósito.
- **`pattern` y cotas en el esquema**, no en la prosa: dos patrones decimales, fecha, mes y UUID;
  rangos leídos del código, no inventados. Cubre también los anidados de `simulate_projection`.
- **Los `effects` de los 11 previews tenían seis formas distintas**, y la peor escondía
  `allocation_remainder_rules_deleted` —la cifra irreversible que su propia descripción destaca en
  mayúsculas— bajo una clave llamada `unlinked`, la palabra que describe lo contrario. Ahora todos
  son `{entity, side_effects}` con claves en inglés (**breaking**), y la clave `resumen` de los
  payloads de escritura pasa a `summary`: era la última en español, la misma violación de la norma.
- **Doce errores exclusivos de MCP caían a `bad_request` genérico** por faltarles el prefijo del
  código — incluidos los tres helpers de parseo, que son los que más se disparan. `code` es lo
  único estable por lo que un cliente puede ramificar.
- **Mensajes accionables**: el error de decimal dice ahora «usa el punto como separador, sin
  símbolo de divisa ni separador de miles», que en una app española no es un detalle —el usuario
  dicta «once con ochenta y tres» y un reintento a ciegas puede colar un error de dos órdenes de
  magnitud—; `category_scope_invalid` lista sus cuatro valores; y `rule_patch_conflict` deja de
  nombrar `clear_assign_category_id`, un parámetro que **no existe**.
- **Clamp silencioso → rechazo.** Cuatro parámetros declaraban cotas y las **clampaban**, así que
  `get_projection` y `simulate_projection` respondían distinto al mismo valor fuera de rango: una
  con un error y otra con una proyección etiquetada `months_override`, que el modelo lee como «me
  hizo caso». Un clamp silencioso hace que la respuesta describa una pregunta distinta de la que se
  hizo (**breaking**; verificado con greps que la SPA no manda ningún valor fuera de rango).
- **`create_liability` validaba de una en una**: tres viajes para un alta, y tres oportunidades de
  que un agente se invente un TIN plausible para desatascarse — que aquí mueve la amortización.
  Ahora las condiciones del modelo llegan **todas juntas**; con un solo fallo se conserva el código
  específico de siempre, que es más accionable.
- **Una guardia vivía en la capa MCP y no en la core**: por HTTP se borraba la fecha de fin de
  gasto en silencio y por MCP se rechazaba — la superficie derivada más estricta que la fuente,
  justo lo contrario del contrato. El inventario completo de los ocho `clear_*` demostró que era
  **el único** caso real: los otros tres «solo-MCP» son legítimos, porque el cuerpo HTTP usa
  tri-estado JSON y es MCP quien lo aplana.
- **El congelador de contrato era ciego a todo lo anterior.** Congelaba nombres de propiedades,
  `required` y el hash de la descripción, pero no `additionalProperties`, ni `enum`, ni `pattern`,
  ni las cotas: se podía borrar mañana un `enum` sin que fallara ningún test. Ahora congela también
  las restricciones, recorriendo `properties`/`items`/`$defs`, y su fallo nombra **la tool, la ruta
  del schema y la restricción que se perdió** — porque un fallo indescifrable se «arregla»
  regenerando el fixture sin mirar, que es lo contrario de para lo que existe.

### Números que mienten (Fase 1 — issue #82)

Trece arreglos de cifras que el servidor devolvía mal o de forma no interpretable. Cinco eran
críticos y todos se verificaron llamando a las tools, no solo leyendo el código.

**`update_fire_settings` descartaba la inflación en silencio.** Su gemela `simulate_projection`
acepta el alias `annual_inflation_percent` desde 4.0.0; la tool de ESCRITURA no lo aceptaba ni
rechazaba lo desconocido. El flujo natural —simular con ese nombre, convencerse, guardar con el
mismo nombre— respondía `200` con `applied: true`, persistía el SWR y **tiraba la inflación**. El
incidente de 4.0.0 sin arreglar en la dirección de escritura, sobre el eje que más mueve la
proyección. Ahora el alias es legítimo y el struct lleva `deny_unknown_fields` (**breaking**:
rechaza campos que antes se ignoraban).

**`clear_*` ganaba en silencio sobre el campo puesto** en `PATCH /v1/transactions/{id}` y en la
tool `update_transaction`. Pasar `category_id` y `clear_category` a la vez devolvía `200` y dejaba
el movimiento **sin categoría**: el total seguía cuadrando y la atribución mentía. El camino de
lote ya tenía la guardia; la de fila, no. Ahora los **cinco** `clear_*` la tienen, en la core
compartida, así que HTTP y MCP quedan cubiertos a la vez (**breaking**, 5 códigos nuevos).

**`get_history` devolvía los activos en un campo llamado `net_worth`.** Sin snapshots de pasivo,
`net_worth` era idéntico a `assets_total` — y la descripción de la tool prometía en su primera
frase que «cuadra con `get_summary.net_worth`» antes de desmentirse a sí misma más abajo. Ahora es
**`null` en toda la serie** cuando el pasivo del scope no está fotografiado entero: es imposible
dar el número equivocado (**breaking**, nulabilidad). El flag `liabilities_snapshotted` pasa además
de `any` a **`all` por usuario**: con `any`, un hogar donde solo un miembro fotografía su deuda
publicaría `activos − deuda_de_uno`, un número que ya no coincide con `assets_total` y que **por eso
parece correcto** — el mismo bug, pero indetectable a ojo. La salida es capturar un snapshot de
pasivo, que escribe la cabecera aunque no haya ni una deuda: convierte «no debo nada» en un hecho
afirmado por el usuario en vez de una ausencia interpretada por el servidor. Mismo tratamiento en
`GET /v1/history/cashflow` → `fine.net_worth`, que tenía el defecto **y ni siquiera publicaba el
flag**; ahora lo publica.

**`jubilacion_month_index` no indexaba ninguna serie devuelta.** Es un número de MES, y `points` es
de densidad híbrida: con la densidad que la tool MCP fuerza, la serie tiene ~42 posiciones y un mes
de cruce típico se sale de todas. El objetivo FIRE nominal del mes del cruce era **inobtenible**;
un modelo que cayera en `fire_target_series[0]` presentaba el objetivo de hoy como el de dentro de
décadas — **1,48× de error** medido, creciendo con horizonte e inflación. Añadidos
`jubilacion_series_position` (último punto con `month_index <=` el del cruce) y
`jubilacion_target_net_worth_nominal` (calculado exacto, no interpolado).

**Huecos que se reportaban como ceros.** Un mes sin movimientos devolvía `actual: 0` y deltas
iguales al presupuesto entero en negativo, así que la respuesta a «¿mi gasto de este mes va bien?»
era «vas muy por debajo de tu media» cuando lo cierto es que no hay datos. Ahora
`GET /v1/transactions/summary` publica `actual_txn_count` y `has_actual_data`, y `delta_vs_budget` /
`delta_vs_avg` / `avg` llegan **`null`** sin base (**breaking**, nulabilidad). Los `actual` NO se
anulan: una suma sobre el conjunto vacío es 0 de verdad. La serie por categoría gana `has_data` por
punto y `first_month_with_data` en la raíz. La SPA pinta guion donde antes pintaba «0 €» y avisa
`Sin movimientos este mes.`

**Otros siete.** `final_net_worth_real_delta` daba **signos opuestos** para la misma magnitud al
simular la inflación (cada lado se deflacta con la suya) → `null` + `real_delta_absent_reason`.
`debt_service` valía `0` con un préstamo vivo cuando la cuota ya está dentro del gasto real → `null`
+ razón; el gate correcto es `expense_from_avg`, **no** el modo, porque el fallback del promedio es
por lado y hay casos donde el modo dice «transactions_avg» y la cuota sí se cobra. El preview de
`delete_categorization_rule` **reventaba** con una regla sin `assign_kind` (el borrado ciego
funcionaba y el previsualizado no); ahora responde, y de paso dice si esa regla **tapa** a otra
(`shadowed_transactions`), que es lo que de verdad se pierde al borrarla. Un `category_id` de otro
scope devolvía `200` con la serie vacía, indistinguible de «no gastaste nada ahí» → 400 tipado. Un
importe absurdo desbordaba `NUMERIC(18,4)` y salía como **`internal error` pelado**: el único error
que un cliente no podía clasificar, y justo el que dispara retry-on-5xx contra una entrada que
jamás será válida → SQLSTATE `22003` mapeado a 400 `amount_out_of_range` en el mismo sitio que los
23505/23503, cubriendo toda la API de una vez. `due_date: "9999-12-31"` se aceptaba y contaminaba
`upcoming_outflows_total`. Y dos reglas de categorización agnósticas idénticas se creaban las dos
(`source IS NULL`, y en SQL `NULL != NULL`) pese al 409 que promete la descripción: migración con
índice único parcial + dedup previo — el dedup es **demostrablemente inocuo** porque las filas que
borra empatan en los tres primeros componentes de la precedencia y pierden en el cuarto, así que no
podían ganar ningún matching. Los `.ffbackup` antiguos con duplicados **siguen importando**
(`ON CONFLICT DO NOTHING`): romper esa vía habría dejado sin recuperación a quien la necesitara.

## [4.3.1] - 2026-08-27

### «Entrar con Home Assistant» — HA como proveedor de identidad (solo add-on)

- **Por qué existe**: la 4.3.0 dejó a los usuarios del add-on con SSO solo dentro del panel. En el
  origen directo (túnel/puerto) sus cuentas —sin contraseña, por diseño— no podían iniciar sesión,
  y por tanto **no podían autorizar el conector MCP de claude.ai** (el consentimiento OAuth exige
  sesión en ese origen). Este parche cierra esa cojera; se publica como 4.3.1 y no como minor
  porque una 4.3.0 sin esto queda inservible en demasiados modos para quien siga el tag `:4.3`.
- **Qué hace**: con la opción `ha_sso_url` del add-on rellenada (la URL pública de tu HA), el login
  de FutureFin fuera del panel muestra **«Entrar con Home Assistant»** — también en la pantalla de
  consentimiento OAuth. El flujo es el OAuth de las apps móviles de HA (`/auth/authorize` →
  código → `/auth/token`), la identidad se lee por WebSocket (`auth/current_user`) y **es el mismo
  usuario** que el SSO del panel: el `id` de HA es el `X-Remote-User-Id` del ingress, así que ambos
  caminos caen en la misma fila de `users` (test de paridad dedicado).
- **Modelo de seguridad en tres frases**: HA no soporta PKCE ni client secret, así que la defensa
  es el mismo-origen exacto entre `client_id` y `redirect_uri` más una cookie de estado
  (`ff_ha_state`, HttpOnly, `SameSite=Lax`, un solo uso, 10 min) — la ruta de retorno viaja
  **dentro de la cookie**, nunca en el `state`, y se re-valida contra open-redirects. El refresh
  token de HA se **revoca inmediatamente** tras verificar la identidad: FutureFin no retiene
  ninguna credencial de tu domótica. La feature solo se activa en modo add-on
  (`FUTUREFIN_HA_SSO_URL` + `FUTUREFIN_HA_ADDON=1`, que solo exporta el entrypoint; la URL sin el
  flag aborta el arranque).
- **Primera dependencia de red saliente del binario**: `reqwest` (rustls, sin OpenSSL) +
  `tokio-tungstenite` para la pata WebSocket, con un único stack rustls en el árbol (gate
  `cargo tree -d`). Los tests de integración usan un doble del proveedor tras el trait `HaIdp`
  (17 casos nuevos, orden `exchange → identity → revoke` verificado) — sin red en la suite.
- **Reapertura deliberada**: la arqueología tenía «OAuth login» (FutureFin como cliente de un IdP)
  como batalla cerrada; se reabre **estrecha y conscientemente** solo para HA, con scope-note
  fechado y la decisión D19 del contrato de arquitectura (HA = fuente de identidad, nunca de
  autorización: roles y membership siguen siendo de FutureFin).
- **No es breaking y el rollback es trivial**: sin la opción todo es byte-idéntico a la 4.3.0
  (los tests del shell lo fijan); vaciar `ha_sso_url` apaga la feature — sin migraciones, sin
  datos que deshacer.

## [4.3.0] - 2026-08-27

### Home Assistant — FutureFin se instala como add-on

- **El repositorio es también una tienda de add-ons**: `repository.yaml` en la raíz y el add-on en
  `addon/futurefin/`. Se añade desde la tienda de complementos, se instala, se arranca y sale como
  **panel en la barra lateral** (`panel_title: FutureFin`, `panel_icon: mdi:currency-usd`) por el
  **ingress** del Supervisor — sin publicar ningún puerto, sin TLS que gestionar y sin escribir un
  `docker-compose.yml`. **No construye nada**: `image: maxlainz/futurefin` (Docker Hub) apunta al
  manifest multi-arch ya publicado, deliberadamente **sin `{arch}`**, para que el registry sirva
  amd64 o aarch64 según el host. `init: false` porque el entrypoint de la imagen tiene que seguir
  siendo PID 1: es quien supervisa PostgreSQL y hace el apagado ordenado, y meter s6 por delante
  rompería esa cadena.
- **Todo vive en `/data`, el único bind persistente que monta el Supervisor**: el entrypoint
  detecta el modo add-on por la presencia de `/data/options.json` y **pisa** `PGDATA` y
  `FUTUREFIN_STATE_DIR` (a `/data/pgdata` y `/data/state`) *antes* de la sección de configuración.
  Tenía que ser un override explícito y no un `${VAR:-default}`: el `Dockerfile` exporta las dos
  como `ENV`, así que el default nunca se habría aplicado y la base habría acabado fuera del
  volumen — perdiéndose al recrear el contenedor. Por lo mismo, la guarda de «no arranco sin
  volumen» pasa a preguntar por los **ancestros** del directorio (`is_persisted`) y no por el
  directorio exacto: bajo HA el mountpoint es `/data`, no `/data/pgdata`. Sigue **parando antes de
  `/`**, porque en cualquier contenedor `/` es un mountpoint y aceptarlo volvería la guarda
  decorativa.
- **`backup: cold`**: el Supervisor **para** el add-on mientras copia `/data`. Copiar en caliente el
  directorio de datos de un PostgreSQL en marcha no da una copia consistente, y una copia que no
  restaura no es una copia. El precio son 1–2 minutos de indisponibilidad por backup, y se paga.
- **Cinco opciones** (`log_level`, `sso`, `mcp`, `cors_origins`, `public_url`) que el entrypoint
  traduce a las variables de entorno de siempre, con traducciones al español y al inglés. Detalle
  que costó un intento: se leen con `has($k)` explícito y **no** con el `//` de `jq`, porque `//`
  trata `false` como vacío — un `mcp: false` se habría leído como «sin definir» y el toggle no se
  habría aplicado nunca.
- **Puerto directo `8080/tcp`, declarado pero cerrado por defecto** (`null`). Solo hace falta para
  MCP y OAuth, que **no pueden funcionar por el ingress**: el descubrimiento de OAuth 2.1 (RFC 8414
  y RFC 9728) exige servir los `/.well-known/*` en la **raíz del origen**, y bajo el ingress esa
  raíz es de Home Assistant. No hay arreglo desde este lado; hay receta, y está documentada.
  Tampoco se declara `watchdog`: el único endpoint candidato (`/v1/ready`) solo es alcanzable por
  ese puerto, así que un watchdog reiniciaría en bucle la instalación normal.
- **Documentación nueva**: [`docs/home-assistant.md`](docs/home-assistant.md) (instalación,
  usuarios, tabla de opciones, MCP con receta LAN y Cloudflare Tunnel, copias, migración desde
  Compose, actualizaciones, limitaciones y diagnóstico) más la ficha corta que se ve dentro de HA
  en `addon/futurefin/DOCS.md`.

### Acceso — Inicio de sesión con la identidad de Home Assistant

- **`POST /v1/auth/sso`**: el ingress ya autenticó a la persona antes de que la petición llegue, y
  manda su identidad en `X-Remote-User-Id`. El endpoint la canjea por una **sesión normal** —misma
  fila en `sessions`, misma cookie `ff_session`, mismo gate de instalación, mismo warm-up de la
  proyección en background—; a partir del 200 no hay nada especial en ese usuario salvo que su
  `password_hash` es `NULL`. La primera persona que entra por aquí **crea el hogar y queda como
  owner**, igual que el primer registro por contraseña; las siguientes quedan pendientes de
  aprobación. Si el canje falla por lo que sea, la SPA **cae al formulario de acceso de siempre**:
  el SSO es un atajo, nunca la única puerta.
- **El modelo de confianza es una puerta doble, y las dos hojas son opt-in**:
  `FUTUREFIN_TRUSTED_PROXY_AUTH` (sin ella, `401 sso_disabled`) **y**
  `FUTUREFIN_TRUSTED_PROXY_IPS` (peer fuera de la lista, `401 sso_untrusted_peer`). Activar la
  primera sin la segunda **aborta el arranque**: una cabecera de identidad es una afirmación sin
  prueba, y sin peer verificado la escribiría cualquiera. En el add-on el entrypoint las pone solo
  con `sso: true`, y la lista es exactamente el ingress del Supervisor (`172.30.32.2`) — de donde
  se sigue que **el puerto directo nunca honra `X-Remote-User-*`**. La ruta se monta **siempre**,
  llueva o truene: lo que decide es el estado, no la forma del router, o los tests dejarían de
  describir el binario que se despliega.
- **Las cuentas SSO no tienen contraseña, y eso se dice en voz alta.** Ni `POST /v1/auth/login` ni
  `POST /v1/auth/password` ni la exportación `.ffbackup` pueden hacer nada con un `password_hash`
  nulo, así que los tres responden un `401 sso_account_no_password` **hablado** en vez de un 401
  mudo o un «contraseña incorrecta» falso. Fijar contraseña desde ahí queda **fuera de alcance en
  esta versión** a propósito: crearía una segunda vía de acceso a una cuenta cuya autenticación
  pertenece al proveedor. El caso de la exportación es el menos obvio y el más importante: la clave
  del `.ffbackup` se **deriva de la contraseña de la cuenta**, así que sin contraseña el fichero
  sería indescifrable — mejor negarse que entregar un archivo que nadie puede abrir. El coste
  asumido es que ese 401 revela que la cuenta existe; es la misma postura que el `username_taken`
  del registro, y está en la lista de «fuera de alcance» de `SECURITY.md`.

### Servidor — Base path genérico: FutureFin ya se puede colgar de una ruta

- **El prefijo público se resuelve por petición** (`apps/api/src/prefix.rs`), con precedencia
  `X-Ingress-Path` > `X-Forwarded-Prefix` > `FUTUREFIN_BASE_PATH` > raíz. El servidor sigue
  montando **todas** sus rutas en la raíz —quien quita el prefijo es el proxy—; lo que depende de
  él es lo que resuelve el navegador: los refs absolutos del HTML, las URLs de `fetch`, los
  `pushState` y el `Path` de la cookie. Por eso lo inyecta un handler (`handlers/spa.rs`,
  `window.__FF_BASE__`) y no un `base` de Vite en build ni un placeholder reescrito al arrancar:
  **la misma imagen sirve Compose en `/` y el ingress bajo `/api/hassio_ingress/<token>` a la vez**,
  y el token cambia. La SPA lo consume desde `apps/web/src/lib/basePath.ts`, con helpers puros e
  **idempotentes** (un path que pasa dos veces por `apiUrl` no se prefija dos veces).
- **Invariante que hace esto seguro de mergear: sin prefijo y sin SSO, el `index.html` sale byte a
  byte igual** (`Cow::Borrowed`), y `BASE_PATH` degrada a `""` ante cualquier basura. El modo
  Compose no cambia ni un carácter. Cuando sí se inyecta, la respuesta lleva `Cache-Control:
  no-store` y `Vary: X-Ingress-Path, X-Forwarded-Prefix`: el shell depende de cabeceras de proxy y
  ningún caché intermedio debe servir el de un despliegue a otro.
- **Un prefijo inválido no se cuela**: debe empezar por `/`, sin `//`, sin segmentos `.`/`..`,
  charset `[A-Za-z0-9._~/-]`, ≤128 caracteres. Una **cabecera** inválida se ignora y se sigue con
  la fuente siguiente (con un `warn` deduplicado y acotado a 8 entradas, para que nadie convierta
  el log en un canal de flood); un **`FUTUREFIN_BASE_PATH`** inválido **aborta el arranque**, igual
  que `FUTUREFIN_PUBLIC_URL` — mejor un fallo ruidoso que HTML roto en silencio. La detección de
  prefijo **no** exige peer de confianza, y es deliberado: un `X-Forwarded-Prefix` falsificado solo
  deforma la respuesta del propio atacante.
- **Salda una deuda vieja**: hasta ahora servir FutureFin en `https://tu-host/futurefin/` no
  funcionaba y no había forma de arreglarlo desde la configuración. Ahora hay recetas de nginx,
  Caddy y Traefik en [`docs/instalacion.md`](docs/instalacion.md), con el aviso de que **MCP y
  OAuth siguen necesitando la raíz de un origen** y por tanto no valen en subpath.

### Seguridad — Anti-clickjacking condicional (enmienda de un invariante)

- **Hasta ahora la regla era absoluta: `X-Frame-Options: DENY` fijo sobre el router final, «nada de
  FutureFin se embebe en un iframe».** El ingress de Home Assistant pinta el add-on dentro de un
  iframe del **mismo origen** que HA, así que con `DENY` el panel salía **en blanco**. La enmienda
  no es quitar la protección: es cambiar `DENY` por `Content-Security-Policy: frame-ancestors
  'self'`, que sigue prohibiendo el embebido **cross-origin** — el vector real del clickjacking —
  y permite el same-origin que el ingress necesita.
- **La relajación tiene doble llave**: peer en `FUTUREFIN_TRUSTED_PROXY_IPS` **y** cabecera
  `X-Ingress-Path` presente. Con una sola bastaría mandar la cabecera a mano desde fuera para
  desactivar la protección. Con peer no confiable —el default— la respuesta lleva `DENY` aunque la
  cabecera venga.
- Detalle que evita un falso «arreglado»: cuando se aplica la CSP se **elimina** el
  `X-Frame-Options`, porque los navegadores que miran las dos cabeceras dan prioridad al `DENY` y
  la app habría seguido en blanco. Y la capa envuelve el router **final** (el que ya incluye el
  fallback SPA), no `api`: la pantalla de consentimiento OAuth la sirve ese fallback y es
  justamente la que más protección necesita.

### Sesiones — la cookie `ff_session` se acota al prefijo

- **El `Path` de la cookie pasa a ser el prefijo de la petición** (`/` cuando no hay prefijo, es
  decir el comportamiento de siempre en Compose), y **solo cuando el peer es de confianza**: el
  prefijo se acepta sin verificar peer porque falsificarlo solo deforma la respuesta del propio
  emisor, pero el `Path` de una cookie no es cosmético — sin esa condición cualquiera podría fijar
  el de su propia `ff_session`. Motivo concreto: bajo el ingress todos los
  add-ons comparten el origen de Home Assistant, así que una cookie con `Path=/` se enviaría a
  **todos los demás add-ons** de la instalación. Acotarla la deja donde tiene que estar. La cookie
  de borrado usa la misma plantilla de path, porque un `Set-Cookie` de borrado solo casa con la
  cookie si el `Path` coincide.

### Arranque — la guarda de downgrade explica en vez de fallar en críptico

- **Imagen vieja sobre datos nuevos ya no muere con el error crudo de sqlx.** `VersionMissing` es
  la firma exacta de ese caso, y ahora se traduce a un mensaje de operador que empieza por
  «FutureFin NO ARRANCA: esta base de datos viene de una versión MÁS NUEVA», dice **«TUS DATOS
  ESTÁN INTACTOS: no se ha tocado nada»** y da las dos salidas: volver al tag más nuevo (lo normal)
  o restaurar el `pre-migration-*.sql.gz` (`/var/lib/futurefin/backups`, o `/data/state/backups` en
  el add-on). Importa más de lo que parece con actualización automática de por medio: el rollback
  del add-on es restaurar la copia de HA, y quien lo intente reinstalando una versión anterior se
  topa con esto y necesita entender qué le está diciendo.
- **No añade ninguna comprobación propia**: sqlx ya fallaba ahí. Cualquier otro error de migración
  —un desajuste de checksum, señaladamente— pasa **tal cual**, conserva su mensaje y sigue sin
  auto-repararse.

### Release — el add-on se versiona solo

- **`publish-image.yml` sube el `version:` de `addon/futurefin/config.yaml` en `main`** al final del
  run que publica, cuando la imagen ya está verificada en el registry y el GitHub Release creado:
  la tienda de add-ons **nunca anuncia una versión que no existe**. Sin ese paso, el número se
  quedaría clavado para siempre en la versión anterior, porque el Supervisor usa ese `version:`
  como tag de la imagen. Va por la Contents API y no por `git push` porque los checkouts del
  workflow usan `persist-credentials: false`. No hay bucle: un push con `GITHUB_TOKEN` no dispara
  workflows.
- **Requisito manual que no vive en git**: la app «GitHub Actions» tiene que estar como *bypass
  actor* del ruleset «Proteger main», o el paso falla con un 403. Si falla, la imagen y el Release
  ya están fuera y el add-on se queda una versión por detrás — se arregla con un PR normal.
  Anotado en `CONTRIBUTING.md` para que no se pierda.
- **`./scripts/audit-releases.sh --addon`** comprueba que el add-on y `apps/api/Cargo.toml`
  declaran la misma versión.

### Endurecimientos de la revisión previa al merge

Ocho correcciones salidas de la revisión de la rama, ninguna visible en Compose y todas capaces de
romper el add-on:

- **`sso: false` ya no deja el panel en blanco.** Con el SSO apagado el entrypoint no ponía
  `FUTUREFIN_TRUSTED_PROXY_IPS`, y sin peer de confianza la respuesta salía con `X-Frame-Options:
  DENY` — es decir, el iframe del ingress se quedaba vacío justo en la configuración pensada para
  quien prefiere el login clásico. La lista del ingress se pone **siempre** en modo add-on; lo que
  gobierna `sso` es el canje de identidad (`FUTUREFIN_TRUSTED_PROXY_AUTH`), que es lo que la opción
  dice que hace.
- **`/index.html` explícito pasa por el inyector.** El shell solo se inyectaba en el fallback SPA;
  pedir la URL literal servía el fichero crudo, sin `__FF_BASE__`, y la app arrancaba creyéndose en
  la raíz. Un caso raro de teclear a mano y garantizado en cuanto algo enlace esa URL.
- **Cerrar sesión gana al SSO.** El flag de «ya lo he intentado» vivía a nivel de módulo, y bajo el
  ingress el iframe se **remonta** al cambiar de panel: tras «Cerrar sesión», volver al panel te
  volvía a entrar solo. Ahora la marca vive en `sessionStorage` (sobrevive al remontaje, se limpia
  en una pestaña nueva). Corolario necesario: el formulario de acceso enseña **«Entrar con Home
  Assistant»** cuando hay SSO disponible — sin ese botón, una cuenta SSO (que no tiene contraseña)
  se quedaba encerrada fuera después de cerrar sesión.
- **Los `href` de la navegación llevan prefijo.** Las pestañas de la barra superior, el drawer móvil
  y el enlace del aviso de inflación pintaban la ruta cruda: el clic normal iba bien (lo intercepta
  el router), pero Cmd/rueda —y la vista previa del enlace— salían del subpath al 404 de Home
  Assistant. Van por `appUrl`, que además pasa a ser **literalmente la misma función** que `apiUrl`
  para que las dos no puedan divergir. Una regla de ESLint prohíbe desde hoy pasar una ruta absoluta
  a `fetch`, y CI comprueba que el `index.html` construido solo trae referencias absolutas de las
  que el reescritor del servidor sabe tratar.
- **Un `X-Remote-User-Id` repetido se rechaza.** Se leía la primera aparición; con dos cabeceras, un
  proxy y un atacante podían discrepar sobre quién eres y ganaba la que llegara antes. Ante más de
  una, la petición se rechaza en vez de elegir.
- **`FUTUREFIN_TRUSTED_PROXY_IPS=any` es incompatible con el SSO activado** y aborta el arranque. El
  comodín existe para redes privadas y tests; combinado con «acepto la identidad que me declaren»
  significa no autenticar.
- **El `%` sale del charset del prefijo.** Estaba permitido para dejar pasar rutas ya escapadas, pero
  el prefijo se **interpola en el HTML** y en el `Path` de la cookie: aceptar escapes obligaba a
  razonar sobre doble decodificación en dos contextos distintos. El token del ingress no lo necesita.
- **El `Path` de la cookie solo se acota con peer de confianza** (ver arriba).

### Migración / compatibilidad

- **Migración `20260827120000_users_trusted_header_identity.sql`**: `users.password_hash` deja de
  ser `NOT NULL` y aparece `users.external_user_id` (UUID) con un índice **UNIQUE parcial**
  (`WHERE external_user_id IS NOT NULL`, para que las cuentas de contraseña —que la dejan a NULL—
  no compitan por él y el índice quede pequeño). Las dos cosas son **aditivas**: ninguna fila
  existente cambia. Una cuenta creada por el proxy no tiene contraseña que guardar y **la ausencia
  se modela como ausencia**; inventarle un hash aleatorio habría sido peor — una credencial que
  nadie conoce y que el cambio de contraseña podría rotar.
- **Datos**: sin pérdida. **Backups `.ffbackup`**: sin cambio, `schema_version` sigue en **10**.
- **Primer arranque tras actualizar**: nada que hacer. Sin `FUTUREFIN_BASE_PATH`, sin cabeceras de
  proxy y sin `FUTUREFIN_TRUSTED_PROXY_*`, el `index.html` se sirve idéntico, la cookie sigue con
  `Path=/` y la respuesta sigue llevando `X-Frame-Options: DENY`. Actualizar no mueve ningún número.
- **Rollback**: volver a 4.2.1 con la migración ya aplicada **no arranca** — es exactamente el caso
  que la guarda de downgrade de esta versión aprende a explicar, solo que el binario de 4.2.1 aún
  lo cuenta con el error crudo de sqlx. Se para sin tocar los datos igualmente; para bajar de
  verdad hay que restaurar el `pre-migration-*.sql.gz`. Y lo que además se
  pierde al bajar es el add-on: una imagen anterior no conoce `/data/options.json`, arrancaría en
  modo Compose con `PGDATA` fuera de `/data` y **la guarda de volumen la pararía**.

## [4.2.1] - 2026-08-25

### Corregido

- **El catálogo MCP no contaba entero el modelo de amortización nuevo**: las tools
  `create_liability`/`update_liability` y `get_summary` salieron en 4.2.0 con los matices del
  `repayment_model`, pero `list_liabilities` seguía enumerando los campos del pasivo sin el
  modelo, y `get_projection` no decía cómo simula ahora la deuda. Un cliente MCP que solo
  leyera esas dos descripciones podía razonar con el modelo antiguo (amortización 1:1 siempre).
  Ambas descripciones nombran ya el `repayment_model` y su efecto: solo `french`/`revolving`
  devengan intereses, y solo con plan de pago activo. Sin cambios de esquema ni de datos —
  las descripciones de tools son contrato y por eso esto es un patch, no un arreglo de docs.

## [4.2.0] - 2026-08-25

### Proyección — Modelo de amortización por pasivo: la deuda ya devenga intereses

- **Cada pasivo declara CÓMO se paga (`repayment_model`), y la proyección le cobra los
  intereses**: `fixed_payments` (default, el modelo histórico) · `french` (sistema francés) ·
  `interest_only` (solo intereses) · `revolving`. Hasta ahora el motor amortizaba **1:1**: cada
  cuota reducía el principal en su importe exacto, como si ningún préstamo cobrara nada. Sobre
  una instalación de ejemplo, una deuda de **100.000 €** con cuota de **500 €/mes** al **3 %**:

  | | Antes (hasta 4.1.0) | Ahora, declarándola `french` |
  |---|---|---|
  | Mes de extinción | **200** (100.000 ÷ 500) | **278** |
  | Meses de cuota que faltaban | — | **+78** (6 años y medio) |
  | Intereses nunca cobrados | 0 € | ≈ **38.800 €** (277 × 500 + ~303 − 100.000) |

  Los 78 meses son lo grave: el patrimonio proyectado se liberaba de una cuota de 500 € seis años
  y medio antes de tiempo, y todo lo que venía después —el cruce FIRE incluido— se apoyaba en esa
  liberación falsa. El caso extremo que el modelo viejo **ni siquiera podía representar**: con una
  cuota **por debajo del interés** la deuda **crece** y crece sin techo — 100.000 € al 12 % con
  500 €/mes cierran el mes 1 en 100.500 y el mes 2 en 101.005. Antes esa misma deuda «se
  amortizaba» a 500 €/mes.
- **Es opt-in: al actualizar no se mueve un solo número.** La columna nace con
  `DEFAULT 'fixed_payments'` y ese modelo reproduce la recurrencia anterior **bit a bit** —
  pineado por un test escrito ANTES de la reforma (`liability_payment_plan_series_pin_pre_4_2_0`, que
  congela la serie en los meses 0/1/2/12/200/201/300) y por un test que le añade un TIN del 3 % al
  mismo input y exige serie idéntica: **el TIN es un dato inerte hasta que eliges modelo**.
  Alguien puede rellenar el interés de su hipoteca sin cambiar nada más y sus cifras no se mueven.
  Los intereses empiezan a contar solo cuando el usuario elige `french`/`revolving` y configura
  la TAE.
- **Qué hace cada modelo**: los que devengan aplican `P' = P·(1+i) − M` con `i = TAE/1200`,
  interés sobre el **saldo de apertura** y cuota a **fin de mes** — la misma recurrencia con la que
  el histórico ya interpolaba el pasado entre snapshots, así que pasado y futuro por fin hablan el
  mismo idioma. `interest_only` deja el principal **constante** y la caja es la cuota (la cuota
  declarada YA es el interés; devengarlo otra vez lo cobraría dos veces). `revolving` **comparte
  recurrencia con `french` en 4.2.0**, a propósito y con test que lo fija: existe como concepto
  aparte porque su evolución (disposiciones, cuota mínima como % del saldo) sí divergirá, y ese
  día el test se cambia a conciencia. Dos detalles con consecuencias: sin **plan de pago activo**
  no hay devengo (un `payment_end_date` cumplido con principal vivo **congela** el residual, no lo
  convierte en bola de nieve; y los modos de ahorro B/C, que ponen la cuota a cero, siguen viendo
  su deuda plana — las cuotas ya viven dentro de su gasto real promediado), y el tope de la cuota
  pasa a ser el **payoff** en vez del principal: cancelar cuesta el saldo *con* el interés del mes
  (400 € al 3 % ⇒ 401,00 € de caja, no 400).
- **El principal derivado del plan ya no es una sola fórmula (cambio de comportamiento de
  `POST`/`PATCH /v1/liabilities` con `derive_principal_from_plan`)**: en `fixed_payments` sigue
  siendo `Σ cuotas` exacta, como siempre; en `french` es el **valor actual** de esa renta al TIN,
  que es el capital pendiente de verdad. 200 cuotas de 500 € al 3 % son 100.000 € de caja pero
  **78.618,1542 €** de deuda hoy — 21.382 € de diferencia que antes entraban en el ledger como
  deuda fantasma y se restaban del patrimonio en todo el horizonte. Al 6 %, la misma renta vale
  63.120,2771 €. Cambiar el modelo **o** la TAE con el derive activo **re-deriva** el principal.
  (Nota de método: el plan de la reforma predijo «≈ 78.621 €» y el cálculo devolvió 78.618,15 —
  la desviación era de la predicción, no del código; verificado a 40 dígitos con aritmética
  decimal independiente. La misma disciplina destapó un «≈ 430 meses» que llevaba desde julio en
  la tabla de la campaña de realismo: 430 meses corresponden a un TIN de ≈ 5 %, no al 3 % que la
  fila decía. Corregido a 278.)
- **Validación por modelo** (sobre el estado **resultante**, no sobre el body — en `PATCH` se
  valida el pasivo que va a quedar guardado). Cinco códigos de error estables nuevos, todos con
  su frase en español en la UI: `repayment_model_invalid` (literal desconocido por MCP; por HTTP
  lo corta serde con un 422), `payment_plan_required_for_model` (sin cuota no hay ni interés ni
  amortización: un `french` sin plan sería un `fixed_payments` disfrazado que no mueve un número),
  `apr_required_for_model` (`french`/`revolving` exigen TAE > 0 — con TAE 0 el motor degenera al
  modelo histórico y el usuario tendría un «francés» que no cobra un céntimo),
  `weekly_not_supported_for_model` (la recurrencia es mensual; con `weekly` la cuota se convierte
  ×52/12, exacto sin intereses pero falso con ellos) y `derive_not_supported_for_model` (derivar
  solo tiene inversa cerrada en `fixed_payments` y `french`).
- **Interfaz**: selector «Modelo» en el formulario de pasivos, con pista por modelo, TAE obligatoria
  donde hace falta, `weekly` y el cálculo del capital pendiente **deshabilitados** donde el
  servidor los rechazaría, y una vista previa del principal derivado que **calla** exactamente en
  los estados que darían 400 (enseñar un número donde el servidor da error enseña a desconfiar de
  la vista previa). En el listado, el modelo se ve como chip solo cuando **no** es cuota fija.
  Cero CSS nuevo.
- **MCP**: `create_liability` y `update_liability` aceptan `repayment_model` y sus descripciones
  enumeran los cuatro modelos y la diferencia entre `Σ cuotas` y valor actual al derivar. El
  catálogo **sigue en 52 tools** (es un campo de un recurso ya cubierto, no un recurso nuevo).
  `simulate_projection` queda fuera: no acepta pasivos hipotéticos. Corregida también la
  descripción de `get_summary`, que afirmaba en seco que «la proyección NO descuenta el interés de
  la deuda».
- **Engine breaking**: `ProjectionLiabilityInput` gana dos campos (`repayment_model`,
  `apr_percent`), así que cualquier construcción literal del struct fuera del repo deja de
  compilar. El enum `RepaymentModel` y `present_value_of_payments` se exportan desde
  `futurefin_engine`. Un TIN ausente o ≤ 0 hace degenerar **cualquier** modelo en el histórico, a
  propósito: el motor nunca panica ni falla por un dato incoherente (un TIN absurdo satura en vez
  de desbordar), porque un `.ffbackup` restaurado puede traer combinaciones que hoy no serían
  válidas.
- **Cierra el hueco #11** del inventario de simplificaciones de la campaña de realismo de la
  proyección («los pasivos no llevan interés», clasificado [GAP] desde julio de 2026) — cerrado
  para quien elija modelo, vivo por defecto. Dos simplificaciones nuevas quedan registradas en su
  sitio: un pasivo sin plan activo no devenga, y `revolving` todavía usa la matemática del francés.

### Migración / compatibilidad

- **Migración `20260825120000_liabilities_repayment_model.sql`**: añade
  `liabilities.repayment_model TEXT NOT NULL DEFAULT 'fixed_payments'` + CHECK con los cuatro
  literales (CHECK y no un ENUM de Postgres: añadir un modelo será un `ALTER` de la restricción,
  sin las servidumbres de tipo que un ENUM arrastra a las migraciones y al backup por usuario).
- **Datos**: sin pérdida de datos. Todos los pasivos existentes quedan en `fixed_payments` y sus
  números —proyección, presupuesto, resumen— son idénticos a los de 4.1.0.
- **Backups `.ffbackup`**: `schema_version` sube a **10**. Un backup **v1..v9 importa igual** y sus
  pasivos entran como `fixed_payments`, que es exactamente el modelo con el que se calcularon los
  números que el usuario vio al exportar. **Aviso en la otra dirección: un `.ffbackup` v10 NO
  importa en 4.1.0 ni anterior** — la versión vieja rechaza con `409` cualquier `schema_version`
  por encima de la suya (falla ruidosamente en vez de tragarse lo que no entiende). Si quieres un
  backup legible por la imagen anterior, expórtalo **antes** de actualizar.
- **Primer arranque tras actualizar**: nada que hacer. El selector «Modelo» aparece en el
  formulario de pasivos con «Cuota fija» ya elegido.
- **Rollback**: volver a la imagen 4.1.0 con la migración aplicada funciona — la columna sobra
  pero no estorba (el binario viejo no la selecciona) y ningún pasivo cambia de comportamiento,
  porque el binario viejo solo sabe amortizar 1:1, que es lo que hace `fixed_payments`. Lo que
  **no** sobrevive al rollback es la elección de modelo: los pasivos declarados `french` se
  proyectarán otra vez sin intereses hasta que vuelvas a subir.

## [4.1.0] - 2026-08-25

### Añadido

- **KPI «Rendimiento neto» en el Resumen** (fila «Salud financiera»): el rendimiento anual
  **esperado** del patrimonio neto. El Resumen contaba cuánto tienes, cuánto ahorras y cuánto
  aguantas, pero no lo que tu patrimonio hace por sí solo — y la respuesta no estaba en ninguna
  tarjeta porque hay que cruzar dos lados: los activos rinden y las deudas cuestan. La base, que
  es lo que la métrica promete: se suma `valor × rentabilidad esperada` de **todos** los activos
  del scope, se resta `principal × TAE` de los pasivos **no vencidos** (mismo filtro que el resto
  del Resumen) y se divide entre el patrimonio neto. Un activo sin rentabilidad configurada o un
  pasivo sin TAE cuentan como 0 % pero **siguen pesando en el denominador**: la cifra baja, que es
  la lectura honesta de «no lo has configurado». Ejemplo con cifras inventadas: 100.000 € al 5 %
  más 50.000 € sin tasa, con una hipoteca de 60.000 € al 3 %, dan 3.200 €/año sobre 90.000 € de
  patrimonio = **3,5556 % nominal**; con la inflación en el 2 %, **1,5251 % real**. La cifra
  grande es la real y el paréntesis la nominal.
  - El **real se obtiene dividiendo factores** —`(1+n)/(1+i) − 1`—, no restando puntos: la resta
    (3,5556 − 2 = 1,5556) se desvía justo cuando las tasas suben, y aquí ya se lleva tres
    centésimas.
  - **Sin patrimonio neto positivo no hay métrica**: la tarjeta desaparece y los dos campos no
    viajan en el JSON. Un cociente con denominador negativo se leería con el signo cambiado —
    «rindes un 10 %» sobre un patrimonio que en realidad debes.
  - El cálculo vive en el motor (`crates/engine/src/net_return.rs`, `net_return_percentages`,
    solo `Decimal`, 9 tests unitarios). API: dos campos **aditivos** en
    `financial_health` de `GET /v1/summary` —`net_return_nominal_annual_pct` y
    `net_return_real_annual_pct`, porcentajes (no fracciones) a 4 decimales—, que la tool MCP
    `get_summary` hereda por compartir core.
  - Consecuencia que el texto de ayuda dice en voz alta: la proyección hace crecer los activos
    pero **todavía no le cobra intereses a la deuda**, así que este número es más conservador que
    la simulación. Es una discrepancia real de modelo, y esconderla habría sido peor.

### Infraestructura del repositorio (no toca la imagen; viaja en este release)

- **El espejo de alertas Dependabot ya no queda abierto para decir que no hay alertas**: con
  0 alertas, `dependabot-alerts-mirror.yml` cierra el issue espejo (#55) y lo reabre solo
  cuando vuelve a haber; su estado pasa a ser parte del dato (abierto ⟺ hay alertas), y
  «cero» sigue distinguiéndose de «espejo roto» por presencia + frescura de `GENERADO`. La
  rutina (prompt v8.2) lo busca ahora entre abiertos y cerrados y no toca su estado.
- **El candado de la rutina de dependencias ya no deja rama huérfana** (issue #68): la
  credencial de la rutina no puede borrar refs (HTTP 403, igual que empujar tags), así que
  `ops/routine-lock` se quedaba viva tras cada pasada. Protocolo v8.1: liberar = dejar un
  commit `lock: LIBERADO` en la punta (que cualquier sesión futura lee como candado libre), y
  un workflow nuevo, `routine-lock-janitor.yml`, borra la rama al ver la marca — con
  `workflow_dispatch` como escoba para candados caducados. El repo vuelve a tener una sola
  rama estable: `main`.
- **Toda publicación vuelve a subir su tag de versión** (`fix/publish-version-tags`). Cada
  versión publicada por `workflow_dispatch` (4.0.1, 4.0.4, 4.0.5) y la 4.0.6 (auto-tag) salió a
  los registries **solo como `:latest`**: `metadata-action` deriva los tags semver de
  `github.ref`, que fuera de un push de tag es `main`, y los tres semver quedaban vacíos en
  silencio. Los móviles `:4`/`:4.0` llevaban clavados en 4.0.0 — quien desplegaba con
  `FUTUREFIN_TAG=4` estaba congelado sin ningún error. Arreglo: `value=` explícito en los tres
  semver + un guard nuevo que pregunta al registry por el manifest del tag exacto antes de crear
  el Release (el silencio pasa a ser un run rojo). Además, los tags móviles pasan a moverse
  **por rango**: `:X.Y` con la más alta de su minor y `:X` con la más alta de su major;
  `:latest` sigue reservado al más alto global.
- **Backfill en GHCR por digest, sin reconstruir**: los manifests originales seguían en el
  registry (cada uno fue `:latest` en su día) y los digests exactos constan en los logs de sus
  runs de publicación, así que `:4.0.1`, `:4.0.4`, `:4.0.5`, `:4.0.6`, `:4.0` y `:4` se
  restauraron con `imagetools create` apuntando al manifest original — `:4.0.6` ≡ `:latest`
  byte a byte. En **Docker Hub** el backfill histórico se descartó a propósito (habría exigido
  reconstruir con digests divergentes o credenciales fuera de sesión): allí solo `:latest` y
  las versiones ≤ 4.0.0 responden; las futuras publican completo en ambos registries. Nota de
  la investigación: `:1`→1.8.0, `:2`→2.3.0 y `:3`→3.9.0 siempre estuvieron bien (3.10.0 y
  3.5.0 nunca se taguearon — viajaron dentro de la siguiente versión), y el hueco de `1.0.6`
  (anterior a la imagen autocontenida) se deja documentado sin reconstruir.

## [4.0.6] - 2026-08-24

**Qué cambia para ti**: el gráfico de Proyección deja de encogerse cuando el hogar acumula
activos. Hasta ahora la leyenda vivía dentro del propio gráfico y cada activo le robaba una
línea: con un hogar real (varios miembros, histórico con snapshots) llegaba a ocupar 8 filas
y el gráfico quedaba aplastado — en móvil directamente desaparecía. Ahora la leyenda vive
debajo, colapsada, y el gráfico ocupa siempre todo el ancho y el alto disponibles.

### Interfaz

- **Leyenda escalable en Proyección** (issue del apilamiento): las series estructurales
  (Patrimonio neto, Capital aportado, Objetivo FIRE, Histórico) se ven siempre; los activos
  se ordenan de mayor a menor peso y se truncan con un chip «+N más» expandible. Con 30
  activos o con 3, la leyenda colapsada cuesta lo mismo. El mismo componente unifica las
  leyendas del Resumen y de Jubilación (antes había tres implementaciones distintas).
- **Nombres duplicados desambiguados por persona** en la vista «Todo el hogar»: dos «Cuenta
  corriente» de miembros distintos ahora se leen «Cuenta corriente · ana» / «Cuenta
  corriente · luis». Las series que solo existen en el histórico (snapshots de activos ya
  eliminados) quedan sin sufijo, sin impedir el de las actuales.
- **Tooltip acotado**: lista los 5 activos con más valor en el mes apuntado y agrega el
  resto en «Otros (k) — suma», en lugar de una línea por activo.
- **El gráfico es por fin ancho completo**: el lienzo interno crecía 38px más alto que su
  caja cuando las etiquetas del eje X iban rotadas, y el navegador encogía el dibujo entero
  centrándolo con bandas laterales (defecto que la leyenda interna disimulaba). El alto
  extra ahora sale del propio plot y el dibujo casa exacto con su caja.
- **Etiquetas del eje X legibles**: los años se diezman según el ancho real del plot (fin de
  la ventana siempre etiquetado, huecos uniformes) — antes se pintaban los 90 años del
  horizonte aunque no cupieran. La etiqueta «Hoy» baja a la fila del eje X, alineada con los
  años, en vez de flotar pegada al subtítulo.
- **Móvil, estilo Resumen pero navegable**: «Vista cercana» activada por defecto (override
  efímero — el toggle funciona y la preferencia guardada de escritorio no se toca), sin
  etiquetas del eje Y (el valor exacto vive en el tooltip; el plot gana todo ese margen), y
  el subtítulo se parte en dos líneas para no rozar el borde. La vista cercana deja además
  margen tras el último hito para que la etiqueta «Jubilación» no quede pisada.
- **Ajustes guarda todo solo**: los dos últimos formularios con botón de guardar (zona
  horaria y proyección/modo de edad) pasan a guardado automático con el mismo contrato que
  el resto de la pestaña. La zona horaria solo se envía cuando es una IANA válida (mientras
  tecleas, el pie avisa «no reconocida — sin guardar» en vez de disparar errores), y la
  inflación solo dentro de 0–50.
- **La tabla de tramos IRPF se integra con el resto de la interfaz**: fuera el cajón gris
  del `fieldset` y los inputs desnudos del navegador — ahora usan el estilo estándar de la
  app, alineados a la derecha con numerales tabulares, y «Restaurar España» es un botón
  discreto bajo la tabla.
- **Ajustes armonizado**: todos los paneles siguen ahora la misma plantilla — título sin
  iconografía (los tres iconos de Integraciones se retiran), una descripción corta bajo el
  título, controles separados con el mismo filete, estados canónicos («Cargando…», «Sin
  datos.», «Solo lectura.») y el «Guardado automático.» siempre al pie. En «Fuente del
  ahorro» la pantalla muestra solo la descripción del modo elegido (la comparativa completa
  de los tres modos vive en el icono de ayuda ⓘ, que además absorbe los matices de cuotas de
  préstamos y del fallback al presupuesto); Snapshots estrena descripción, y las cabeceras de
  la tabla de tramos IRPF y las fichas de Instalación/Estado del sistema quedan alineadas con
  el resto de tablas y fichas.

### Infraestructura del repositorio (no toca la imagen; viaja en este release)

- **Auto-tag on merge** (PR #63): mergear un bump de versión a `main` publica solo —
  `publish-image.yml` corre en cada push a `main`, y si `Cargo.toml` lleva una versión sin tag,
  el mismo run espera la CI verde del commit, comprueba el orden estricto, crea el tag y
  construye. Un merge sin bump es un no-op de segundos. El `workflow_dispatch` con «Crear el
  tag» pasa a ser idempotente (tag ya existente → verde sin construir), así que el dispatch de
  la rutina de dependencias no puede chocar con el auto-tag. Esta 4.0.6 es el estreno en vivo.
- **La gestión de dependencias pasa a ser autónoma**: los PRs de Dependabot los procesa una
  rutina cloud disparada por webhook (con barrido de seguridad los martes). Los majors ya no
  quedan bloqueados para siempre: se mergean con una barra de evidencia (notas leídas, cada
  rotura anunciada buscada en el código con la salida como prueba, checks en verde). Cada fix
  que llega a la imagen produce su propio release patch; los issues-informe resueltos se
  cierran solos.
- **Espejo de alertas** (`dependabot-alerts-mirror.yml`): las alertas Dependabot abiertas se
  publican en un issue fijo (label `dependabot-mirror`) porque el sandbox de la rutina no
  puede leer la API de alertas. Requiere el secret `DEPENDABOT_ALERTS_TOKEN`.
- **actionlint en CI**: los propios workflows pasan a tener gate — hasta ahora un error de
  sintaxis o un input inexistente solo se descubría en rojo tras el push.
- **`docker-stack` deja de depender de un pull de `alpine`** para el check de volumen vacío:
  un 500 de Docker Hub tumbó el job el 2026-08-24 sin que nada del repo estuviera roto; ahora
  reutiliza la imagen que el propio job acaba de construir.

## [4.0.5] - 2026-08-24

**Qué cambia para ti**: nada visible en la aplicación, pero con esta versión **la instalación deja
de tener alertas de seguridad abiertas**: las 7 que arrastraba la cadena de build de la interfaz
(2 críticas, 1 alta, 3 moderadas y 1 baja) quedan cerradas. Ninguna era explotable contra un
FutureFin desplegado —todas eran de herramientas de desarrollo, no de código que se ejecute en tu
servidor—, pero salían en el panel del repositorio y ya no salen. No se toca motor, API, interfaz ni
esquema: actualizar es seguro.

### Dependencias que viajan en la imagen

- **`@babel/core`** 7.29.0 → 7.29.7 (transitiva, vía `@vitejs/plugin-react`). Cierra
  **GHSA-4x5r-pxfx-6jf8** (lectura arbitraria de ficheros a través del comentario
  `sourceMappingURL`), la última alerta que quedaba abierta.

### Contexto: de dónde venían las otras seis

Se cerraron en el commit anterior (`vitest` 2 → 4), que no cambia la imagen y por eso no llevó
versión propia. Merece quedar escrito porque el diagnóstico no era el que parecía: **el `vite` y el
`esbuild` vulnerables nunca fueron los del repositorio**, que ya estaban parcheados (6.4.3 y
0.25.12). Eran los que `vitest@2.1.9` anidaba por debajo — `vite@5.4.21` y `esbuild@0.21.5`. Al
subir `vitest` a 4, que admite `vite ^6 || ^7 || ^8`, esas copias anidadas desaparecen y con ellas
las seis alertas. No hizo falta subir `vite` de major ni cambiar `@vitejs/plugin-react`.

## [4.0.4] - 2026-08-24

**Qué cambia para ti**: nada visible en la aplicación — la interfaz se reconstruye sobre **React
19.2.8**, que corrige una regresión de React 19.2.6 en las entradas de `FormData` de las Server
Actions y mejora el rendimiento de decodificación. No se toca motor, API, interfaz ni esquema:
actualizar es seguro.

### Dependencias que viajan en la imagen

- **`react`** y **`react-dom`** 19.2.6 → 19.2.8 (PR #41). Ambas están en `dependencies`, así que
  entran en `apps/web/dist/` vía el `npm ci && npm run build:web` del Dockerfile.

### Dependencias de desarrollo (mismo PR #41, no viajan en la imagen)

- **`eslint-plugin-react-refresh`** 0.4.26 → 0.5.4. Cruza minor en `0.x`, o sea que es un cambio
  con roturas: pasa la barra de evidencia completa. De las cuatro roturas anunciadas en 0.5.0,
  tres no aplican (`customHOCs` no se usa; el repo ya iba con flat config, ESLint 9 y Node 24; el
  default export sigue existiendo y se verificó ejecutándolo). La cuarta —validación más estricta—
  hace aparecer **un aviso nuevo** en `apps/web/src/main.tsx:11`
  (`react-refresh/only-export-components`). No rompe nada: la regla es `warn` y `lint` corre sin
  `--max-warnings`, así que el job `web` sigue en verde. Anotado por si algún día se endurece.
- **`typescript-eslint`** 8.59.3 → 8.67.0, **`@types/react`** 19.2.14 → 19.2.18 y
  **`@types/react-dom`** 19.2.3 → 19.2.4.

## [4.0.3] - 2026-08-24

**Qué cambia para ti**: nada visible en la aplicación — el servidor MCP embebido incorpora las
correcciones de la versión 3.1.4 del SDK `rmcp` (endurecimiento del manejo de claves de firma,
errores de metadatos previos a la inicialización y conservación del dialecto `$schema` en las
elicitaciones). No se toca motor, API, interfaz ni esquema: actualizar es seguro.

### Dependencias que viajan en la imagen

- **`rmcp`** 3.1.3 → 3.1.4, y `rmcp-macros` con él (PR #46). Solo parche, sin cambios de API.

## [4.0.2] - 2026-08-24

**Qué cambia para ti**: nada visible en la aplicación — la imagen se reconstruye sobre una snapshot
más reciente del toolchain de Rust, que trae las actualizaciones de sistema de la base
`rust:bookworm`. No se toca motor, API, interfaz ni esquema: actualizar es seguro.

### Dependencias que viajan en la imagen

- **`rust:bookworm`** (imagen base del `rust-builder` en `apps/api/Dockerfile`): digest
  `adab794` → `e70e2ee` (PR #45).

## [4.0.1] - 2026-08-24

**Qué cambia para ti**: se actualizan las dependencias que van dentro de la imagen, varias de ellas
parches de seguridad de la cadena de terceros. No se toca motor, API, interfaz ni esquema, así que
actualizar es seguro y recomendable.

Todo lo demás que hay aquí abajo es **cómo se desarrolla el proyecto**, no cómo funciona la app:
interesa a quien contribuya, no a quien lo instala. Va en esta versión porque entró en `main` entre
la 4.0.0 y esta imagen.

> **Nota sobre la numeración.** Durante dos días el repositorio llegó a tener secciones de CHANGELOG
> para una 4.0.1, una 4.0.2 y una 4.0.3 que **nunca se publicaron como imagen**: se bumpaba la
> versión por cambios de documentación y de CI que no viajan en el artefacto. Se ha corregido
> colapsándolas en esta única 4.0.1, que sí tiene imagen. La regla, desde ahora: **una versión, una
> imagen**. Si un cambio no altera la imagen, no cambia la versión.

### Dependencias que viajan en la imagen

Son la razón por la que esta versión existe.

Del grupo `cargo-menores` (PR #42) — todas parche o menor, sin cambios de API:

- `chrono` 0.4.44 → 0.4.45
- `cookie` 0.18.1 → 0.18.2
- `http` 1.4.0 → 1.5.0
- `http-body-util` 0.1.3 → 0.1.5
- `rmcp` 3.1.2 → 3.1.3 y `rmcp-macros` 3.1.2 → 3.1.4
- `rust_decimal` 1.42.0 → 1.42.1
- `serde` 1.0.228 → 1.0.229 y `serde_json` 1.0.149 → 1.0.151
- `thiserror` 2.0.18 → 2.0.20
- `tokio` 1.52.3 → 1.53.1
- `uuid` 1.23.1 → 1.24.1

De la cadena de build del frontend, que genera `apps/web/dist/`:

- `vite` 6.4.2 → 6.4.3 (PR #32)
- `postcss` 8.5.14 → 8.5.26, arrastrando `nanoid` 3.3.12 → 3.3.18 (PR #31)

### Dependencias que NO viajan en la imagen

Entraron en `main` en la misma tanda, pero son utillaje de test y lint, así que no cambian el
binario ni los assets servidos: `brace-expansion` (PR #29) y `js-yaml` 4.1.1 → 4.3.1 (PR #30).

### Una sola rama

Desaparece `dev`. El repositorio pasa a **GitHub Flow**: `main` es la única rama de larga vida, el
trabajo va en ramas cortas que vuelven por Pull Request, y **los releases son tags sobre `main`**.

El modelo anterior —`dev` de larga vida volcándose en `main` en cada release— venía de que `main`
no publicaba `CLAUDE.md` ni `.claude/`. Sostener esa frontera costaba unas **244 líneas** cuya
única función era gestionarla:

- `scripts/release-to-main.sh` (126 líneas), que existía para resolver los conflictos
  «modificado/borrado» que salían en CADA release, con comentarios documentando dos bugs que ya
  habían mordido. Un proceso que necesita 126 líneas de defensa contra sí mismo está diciendo algo.
- El job `main-guard` de CI (31 líneas), que vigilaba una frontera que ya no existe.
- Las secciones de `CLAUDE.md` y de la skill `futurefin-change-control` que explicaban por qué las
  dos ramas **no** eran espejo — y que el 2026-08-22 se descubrió que decían justo lo contrario:
  ambas afirmaban que `main` era «un espejo completo de `dev`», cuando actuar en consecuencia
  (`git merge main` desde `dev`) habría borrado la documentación interna entera.

Y lo que más pesaba: mientras el script empujara a `main` directamente, **no se podían exigir
checks obligatorios** en la rama publicada. El issue #28 lo pedía y hubo que dejarlo a medias.
Ahora `main` está protegida de verdad: pull request obligatorio y CI en verde para poder mergear.

La contrapartida, explícita: `CLAUDE.md` y `.claude/` vuelven a estar en la rama por defecto y se
ven en la portada del repositorio. Se comprobó antes de decidir que el coste era de presentación y
no de confidencialidad — con dos ramas públicas, `raw.githubusercontent.com/…/dev/CLAUDE.md` ya
respondía `200` a cualquiera.

De paso, `.github/dependabot.yml` pierde las cuatro líneas de `target-branch` que se le habían
añadido horas antes: con una sola rama, la de por defecto ya es el destino correcto.

### El problema

Abrir el repositorio en público dejó al descubierto cuatro cosas que funcionaban «hacia dentro»
pero no hacia fuera:

- **Los PRs de Dependabot iban contra `main`.** `.github/dependabot.yml` no declaraba
  `target-branch`, así que Dependabot usaba la rama por defecto — que aquí es la de publicación,
  no la de desarrollo. Los 22 PRs de agosto salieron todos con `base=main`: mergear uno dejaba el
  bump fuera de `dev`, con CI compilando contra la versión vieja, y el siguiente que regenerase un
  lockfile lo revertía en silencio. Ahora las cuatro entradas (cargo, npm, github-actions, docker)
  apuntan a `dev`.

  Dos límites que el fichero deja escritos porque no son evidentes: `target-branch` solo redirige
  las *version updates* —las **security updates** van siempre contra la rama por defecto— y
  Dependabot lee la configuración desde la rama por defecto, así que un cambio ahí no entra en
  vigor hasta la release siguiente. Esta.

- **Nadie analizaba el código propio.** `secrets-scan` mira datos personales en el árbol y
  Dependabot mira dependencias de terceros; `code-scanning/alerts` respondía «no analysis found».
  Nuevo workflow `codeql.yml` sobre `rust`, `javascript-typescript` y `actions` — los workflows
  también son superficie de ataque, y este repositorio guarda `DOCKERHUB_TOKEN`. Va aparte de
  `ci.yml` y **no** como check obligatorio a propósito: exigirlo bloquearía el push directo a
  `main` que hace `scripts/release-to-main.sh`.

- **La documentación decía que `main` es un espejo de `dev`**, y actuar en consecuencia destruye
  trabajo. Desde la 4.0.0 `main` no publica `CLAUDE.md` ni `.claude/`, así que arrastra commits que
  solo borran esas rutas y que `dev` no debe recibir jamás: un `git merge main` desde `dev`
  borraría la documentación interna entera. `CLAUDE.md` y la skill `futurefin-change-control`
  —que repetía el error palabra por palabra, en la sección que se consulta *antes* de mergear—
  documentan ahora que el flujo es de una sola dirección, y el comando que distingue «`dev` está
  atrasada» de «`main` solo lleva sus borrados de release».

- **El sync de la descripción de Docker Hub** moría con un «Forbidden» sin explicar por qué y podía
  dispararse desde cualquier rama, saltándose el `main-guard`. Y el borrado de documentación
  interna de `scripts/release-to-main.sh` era un **no-op silencioso**: capturaba la lista de
  ficheros *después* del `git rm --cached`, cuando el índice ya no los tenía, así que el commit
  salía bien y los ficheros se quedaban sueltos en el árbol de `main`.

### Ajustes de GitHub (no viven en git)

Sin commit que los pruebe; se verifican con `gh api repos/<owner>/<repo> --jq
'.security_and_analysis'` y `gh api repos/<owner>/<repo>/rulesets`:

- Dependabot alerts, Dependabot security updates, secret scanning y **push protection**: activados.
  Las alertas salieron al momento: 15 abiertas, las 15 con `scope: development`. Cero en runtime —
  las únicas dependencias de producción del frontend son `react` y `react-dom`, así que **ninguna
  llega a la imagen**. Secret scanning: 0 alertas.
- `main` protegida con un ruleset de `deletion` + `non_fast_forward`. Deliberadamente **sin checks
  obligatorios**: bloquearían el push directo del script de release. Protege lo irreversible, no
  el mergear en rojo.
- Actions: aprobación requerida para **todos** los colaboradores externos (estaba en «solo los
  que contribuyen por primera vez»).

## [4.0.0] - 2026-08-22

**Qué cambia para ti** — FutureFin **se abre en público** y esta versión es la que se puede
enseñar. La app ya se puede usar recién instalada: un hogar nuevo nace con categorías, te recibe
un asistente que pregunta lo imprescindible, y cada pantalla vacía explica qué va ahí. Los errores
te hablan en español. La divisa se puede cambiar. Borrar algo pregunta antes. Ajustes está
reorganizado. **Ya puedes cambiar tu contraseña y retirarle el acceso a alguien del hogar**: dos
cosas que la documentación daba por hechas y que no existían.

Y antes de publicar se auditó todo —seguridad, matemática, contrato de la API, interfaz, CI—, de
donde salieron 28 arreglos. El más caro para ti: **teclear `250.000` guardaba 250 €**, sin avisar.
Están todos contados más abajo.

**Lo único que puede requerir acción por tu parte**: si tu instalación usa una **base de datos
externa** (`DATABASE_URL` apuntando fuera del contenedor), 4.0.0 ya no la soporta. Tus datos no
corren peligro y el contenedor te lo dirá antes de tocar nada — pero tienes que pasar una vez por
la 3.9.0 para migrarlos. Todo lo demás se actualiza como siempre.

### Breaking — se retira el soporte de bases de datos externas

Se anunció en la 3.0.0, en el README, en `.env.example` y en el propio aviso de deprecación que
salía en los logs: la base de datos externa desaparece en 4.0.0. Aquí está. PostgreSQL va **siempre**
dentro de la imagen.

Del entrypoint desaparecen `exec_api_external` (hablar con una base externa), su aviso de
deprecación y la migración one-shot `automigrate_prepare`/`automigrate_restore`. Con ellos se van
`FUTUREFIN_DB_MODE=external` —el valor se sigue aceptando solo para poder dar un mensaje útil en
vez de un error críptico— y `FUTUREFIN_EXTERNAL_WAIT_SECS`.

Lo que queda es una puerta, y es lo importante. Si `DATABASE_URL` apunta fuera del socket local:

- **con un cluster embebido ya en el volumen** → se ignora con un aviso. Quien migró en la 3.x
  tiene sus datos aquí y solo le sobra una variable en el compose.
- **sin cluster** → el contenedor **se para**. No arranca con una base vacía, porque eso se leería
  como pérdida de datos aunque los datos estén intactos al otro lado. El mensaje dice exactamente
  qué hacer: arrancar una vez la 3.9.0 con esa misma `DATABASE_URL` y ese mismo volumen, quitar la
  variable, y volver a 4.0.0.

Esto alcanza también a quien **auto-actualiza con watchtower sobre un compose 2.x sin tocar**: ese
caso venía funcionando en modo compatibilidad desde la 3.0.0 y ahora se para. Es deliberado, está
cubierto por un test de CI, y la ruta de salida es la misma.

`DATABASE_URL` **sigue existiendo y sigue haciendo falta en desarrollo** (`cargo run` contra
`docker-compose.dev.yml`). Lo que se retira es el modo externo del contenedor de producción.

#### Migración

| Tu situación | Qué hacer |
|---|---|
| Compose 3.x normal, sin `DATABASE_URL` | Nada. `docker compose pull && docker compose up -d`. |
| `DATABASE_URL` puesta pero ya migraste en 3.x | Quítala del compose. Si no lo haces, se ignora con un aviso. |
| Base externa de verdad, sin migrar | Arranca **una vez** `maxlainz/futurefin:3.9.0` con la misma `DATABASE_URL` y el mismo volumen, espera a `automigration completed` en los logs, quita `DATABASE_URL` y actualiza a 4.0.0. |
| Compose 2.x de dos contenedores | Igual: pasa por 3.9.0 y después sustituye el compose por el de 4.x. |

#### Tests

El escenario 3 de CI dejaba de tener sentido —probaba la automigración— y pasa a fijar la conducta
nueva: con `DATABASE_URL` heredada y volumen vacío el contenedor **aborta sin inicializar nada** y
el volumen se queda intacto. El escenario 2 mantiene la ruta 2.x → volumen reutilizado, pero su
paso intermedio ahora comprueba el rechazo en vez del modo compatibilidad.

### Auditoría completa previa a la publicación: 28 hallazgos, arreglados

Antes de taguear 4.0.0 se auditó el repositorio entero —seguridad, matemática del motor,
contrato de la API y del MCP, frontend, CI y tests— con la app ya pública. Salieron 28 cosas
que había que arreglar antes de publicar la imagen. Ninguna rompía un test: casi todas
producían números plausibles o mensajes creíbles.

#### Tus datos financieros estaban publicados en los issues

Cinco issues cerrados eran auditorías del servidor MCP hechas contra una instalación real, y
publicaban patrimonio neto, ingreso mensual, tasa de ahorro, deuda viva, el nombre de un
prestamista y comercios concretos. Cerrado no es privado. Se borraron, y con ellos las
referencias del código y del propio CHANGELOG. Se comprobaron además los 2.029 objetos del
historial de git: sin IBAN, sin tarjetas, sin correos.

#### Un importe con separador de miles se guardaba mil veces más pequeño

Teclear `250.000` en el valor de un activo —escritura española normal— lo guardaba como
**250 €**. Sin error: el formulario se cerraba y el patrimonio, la proyección, el número FIRE
y el runway quedaban mal en silencio. El conversor solo cambiaba la primera coma por un punto
y dejaba los puntos intactos, y `250.000` es un decimal válido para el servidor. El asistente
de primera vez llegaba a sugerirlo: su ejemplo era literalmente `1.500`.

Ahora la app entiende la escritura española completa (`1.234,56`, `250.000`) y **rechaza lo
ambiguo en vez de adivinar**, que es exactamente lo que causó el fallo.

#### La proyección de un miembro se le servía a otro

La memoria intermedia que evita recalcular la proyección guardaba una sola copia por hogar,
pero la respuesta lleva datos de **quien la pide**: su fecha de nacimiento, su horizonte y su
edad de jubilación. El primer miembro que abría la proyección dejaba la suya cacheada para
todos. En un hogar de dos personas con edades distintas, la segunda veía el horizonte de la
primera — y si el suyo era más largo, la app podía decirle «no llegas a jubilarte» sobre un
plan que sí llega.

#### Si ya has llegado a tu número FIRE, la app decía que seguías aportando

La pantalla de Activos publicaba una aportación mensual («aportas 2.000 €») para un hogar que
la simulación, en ese mismo mes, está **vendiendo** activos para vivir. Signo contrario, y
sostenido en todo el horizonte. La función que calcula la aportación del primer mes no miraba
el objetivo FIRE; el motor sí. No es un caso raro: es el estado final del público de la app.

#### `?months` no múltiplo de 12 perdía el final de la gráfica

Con densidad `hybrid` la serie solo emitía múltiplos de 12, así que pedir 100 meses devolvía
96: los cuatro últimos no existían, y con ellos desaparecía el punto que cualquiera lee como
«patrimonio al final». Invisible desde la web, pero la herramienta `get_projection` del MCP
usa siempre esa densidad.

#### Faltaban dos palancas que la documentación daba por hechas

**No se podía cambiar la contraseña.** Una cookie robada, una sesión abierta en un ordenador
compartido o una filtración en otro servicio daban treinta días de acceso sin que pudieras
hacer nada. Ahora `Ajustes` permite cambiarla, y hacerlo **cierra las demás sesiones y revoca
los tokens de API y las conexiones OAuth**: si cambias la contraseña por miedo, dejar viva una
credencial que no caduca haría el cambio decorativo. *Aviso*: un `.ffbackup` exportado antes
sigue necesitando la contraseña con la que se generó.

**No se podía retirar el acceso a nadie.** Aprobar al usuario equivocado concedía acceso
permanente a todas las finanzas del hogar; el único remedio era entrar en la base de datos a
mano. Ahora el propietario puede ver los miembros, cambiarles el rol y revocarlos, con la
garantía de que el hogar nunca se queda sin propietario. Revocar **no borra los datos** de esa
persona: si se la vuelve a aprobar, los recupera.

#### Un fichero de copia de seguridad podía tumbar el servidor

El manifiesto de un `.ffbackup` viaja sin firmar, y de él salían los parámetros de la función
que deriva la clave. Un fichero de 200 bytes podía pedir 8 GB de memoria y llevarse por
delante el contenedor entero —con la base de datos dentro— desde el endpoint de
previsualización, que ni siquiera escribe. Se acotan esos parámetros y el tamaño de lo
descomprimido. En la misma línea, el cifrado de contraseñas ya no bloquea el servidor: cuatro
peticiones simultáneas de registro bastaban para dejar la aplicación sin responder.

#### El asistente conversacional creía cosas falsas

El servidor MCP describe cada herramienta al modelo, y varias descripciones mentían.
`get_summary` afirmaba una igualdad entre dos cifras que no se cumple con ninguna hipoteca.
`materialize_recurring` se presentaba como inocua y **borra movimientos**, del hogar entero,
no solo tuyos — con la etiqueta que los clientes usan para decidir si te piden permiso puesta
en «no destructiva». `unreconcile_transfer` es irreversible desde el chat y decía no serlo. Y
`create_liability` prometía amortización francesa cuando suma cuotas sin descontar intereses:
una hipoteca de 850 €/mes hasta 2049 entraba como 234.600 € en vez de unos 185.000.

**Breaking de contrato**: el campo `months_with_data` de `savings_income_basis` /
`savings_expense_basis` pasa a llamarse **`avg_months`** en `/v1/summary`,
`/v1/projection/series` y `simulate_projection` — significaba lo contrario que el campo del
mismo nombre de `/v1/transactions/summary`. La previsualización de `delete_asset` añade
`allocation_rules_deleted` y `allocation_remainder_rules_deleted`: borrar un activo **borra**
las reglas de reparto que apuntan a él, y era el único efecto irreversible que no se contaba.
`simulate_projection` acepta `annual_inflation_assumption_percent` como alias y rechaza los
campos desconocidos en vez de ignorarlos.

#### Un tag mal puesto podía degradar instalaciones ajenas

`:latest` se movía **siempre**, también al reconstruir una versión antigua. Con
`FUTUREFIN_TAG:-latest` en el compose, quien actualiza automáticamente habría recibido una
versión anterior sobre un volumen ya migrado. Ahora `:latest`, `:X` y `:X.Y` solo se mueven si
el tag es el más alto del repositorio, y antes de construir se comprueba que el tag coincide
con la versión del binario y que existe su sección en este archivo. Además, un `pg_upgrade`
interrumpido en el peor momento ya se puede reanudar: el código que lo hacía era inalcanzable
justo en el único caso para el que existía.

#### Y en la interfaz

Un fallo del servidor al cargar la proyección no se veía (la pestaña se quedaba en blanco para
siempre) o se veía en inglés; con la API caída salía «Failed to fetch»; una sesión caducada a
media navegación no te devolvía al acceso; una contraseña incorrecta decía «tu sesión ha
caducado» en la pantalla donde por definición no hay sesión; el guardado automático de Ajustes
daba por guardado lo que había fallado; guardar cualquier ajuste borraba lo que estuvieras
tecleando en inflación; mover un movimiento a otro mes lo dejaba en la tabla del mes viejo; y
con la API caída cuatro pantallas te acusaban de haber borrado tus categorías.

#### Lo que queda escrito para que no vuelva a pasar

Tests de regresión nuevos para todos los hallazgos con consecuencia numérica, cada uno
verificado en rojo antes de arreglar. Una gate nueva (`tests/openapi_contract.rs`) que valida
el propio documento OpenAPI: no había ninguna, y por eso la especificación pública podía
declarar la API entera como si no necesitara autenticación —81 operaciones— sin que nada
protestara.

### Auditoría del servidor MCP: once hallazgos, arreglados

Una auditoría caja-negra del servidor MCP contra una instalación de ejemplo ejercitó las 50 herramientas
y encontró once cosas: cifras que no cuadraban entre sí, escrituras que se aceptaban sin validar,
y campos cuyo nombre invitaba a leerlos al revés. Nada de esto afecta a quien usa la app por la
web; afecta a quien le pregunta por sus finanzas a Claude. Se arreglan todos antes de publicar,
porque 4.0.0 es la única versión en la que se puede cambiar el contrato sin romperle nada a nadie.

#### Un gasto se podía apuntar en positivo, y eso adelantaba la fecha de jubilación

Los importes van firmados: los ingresos en positivo, los gastos y los traspasos a ahorro en
negativo. Esa regla la aplicaba **la pantalla**, no el servidor — así que apuntar un gasto por la
API o por Claude con el importe en positivo se aceptaba sin rechistar. Y como el total de gastos se
calcula cambiándole el signo a la suma, un solo gasto positivo dejaba el **gasto total del mes en
negativo**. Si tu ahorro sale de los movimientos reales (modos B y C), ese mes entraba en el
promedio que alimenta la proyección: la tasa de ahorro subía, la fecha de jubilación se adelantaba,
y nada lo señalaba.

Ahora el servidor lo rechaza al apuntar un movimiento y al cambiarle el importe. **Reclasificar
sigue siendo libre**, y es deliberado: una devolución llega del banco en positivo y pasarla a
«gasto» es lo correcto —netea contra el gasto del mes—, así que ni la edición del tipo, ni la
recategorización en lote, ni las reglas lo impiden. El importador de CSV y la restauración de una
copia `.ffbackup` tampoco validan nada: traen el signo real del banco, y una copia que se niega a
restaurar es peor que una fila rara.

#### No se podía corregir una regla de categorización desde el chat

Desde la 3.8.0 podías pedirle a Claude que creara una regla («todo lo que ponga MERCADONA es
Supermercado») y que la aplicara a cientos de movimientos de golpe. Lo que **no** podía era
corregirla ni retirarla — así que la única salida era crear otra encima. En una instalación de
ejemplo el resultado se ve enseguida: tres reglas contradictorias para el mismo comercio, y un
mismo cargo repartido entre Suscripciones, Hogar y Otros.

Ahora existen las dos herramientas que faltaban. Borrar una regla pide confirmación y antes enseña
**cuántos movimientos gobierna hoy** — y deja claro que borrarla **no descategoriza nada**: lo que
ya está categorizado se queda como está, la regla simplemente deja de aplicarse a los imports
futuros.

De paso, editar una regla dejó de aceptar dos cosas que antes pasaban en silencio: mandar un cambio
vacío (ahora avisa de que no has cambiado nada) y poner y quitar el mismo dato a la vez (antes ganaba
el «quitar» sin decírtelo).

#### El pie del gráfico decía «prom. 0 meses»

Encontrado de camino, no estaba en los issues. Desde la 3.9.0 el gráfico de proyección leía un dato
que el servidor había dejado de enviar al hacerse configurables las ventanas del promedio, así que
en los modos que usan tus movimientos reales el pie ponía siempre **«prom. 0 meses»**. Ahora dice
los meses de verdad, y si el ingreso y el gasto promedian ventanas distintas, dice las dos.

#### Un mes excelente se leía como una pérdida

En la pestaña Movimientos, el tooltip de la gráfica mensual decía «Neto». Ese neto **incluía el
dinero que moviste a ahorro o inversión**, así que un mes en el que ingresaste 2.400 €, gastaste
1.800 € y aportaste 1.500 € a tu cartera salía como **−900 €**. Es aritméticamente correcto —esa
es la caja que se movió— pero se lee justo al revés de lo que pasó.

Peor: la comparativa mensual tenía otra cifra llamada también «neto» que **no** incluía el ahorro.
Dos números distintos con el mismo nombre.

Ahora hay dos cifras y cada nombre dice su fórmula: **«Ingresos − gastos»** (lo que quedó tras
consumir, que es lo que responde a «¿fue buen mes?») y **«Variación de caja»** (incluye los
traspasos). El tooltip enseña las dos, la primera delante, y la palabra «Neto» a secas desaparece de
la interfaz. La primera coincide al céntimo con la de la comparativa.

**API breaking**: en `GET /v1/history/cashflow` y en la tool `get_history_cashflow`, el campo
`net` de cada mes pasa a llamarse **`cash_delta`** —que es lo que siempre fue: la caja que se
movió, traspasos incluidos— y se añade **`income_minus_expense`**, que sí responde a «¿cuánto
me quedó?» y coincide con `totals.net_actual` de `get_transactions_summary`. El nombre viejo
no se conserva a propósito: un campo llamado `net` que significa dos cosas distintas en dos
respuestas es exactamente lo que hacía que un mes excelente se leyera como una pérdida, y
mantener el alias habría dejado vivo el malentendido.

#### Las reglas de categorización se enviaban todas de golpe

Es la única lista que **crece con el uso**: cada import aprende una regla por concepto nuevo, así
que una instalación con dos años de extractos tenía ya un centenar. Preguntarle a Claude por ellas
le gastaba una parte notable de su memoria de trabajo sin que nadie lo pidiera. Ahora vienen por
páginas, con el total y un aviso de si quedan más. La API web sigue devolviéndolas todas: ahí no
molestan y cambiar el formato habría roto la pantalla.

#### «No llegas a jubilarte» y «no te lo puedo decir» se veían igual

Cuando el horizonte de la proyección no alcanzaba el objetivo, los campos de la jubilación
**desaparecían** de la respuesta en vez de venir vacíos. Para quien la lee eso es ambiguo: no
distingue «no se alcanza» de «esta versión no publica el dato». Ahora vienen siempre, vacíos cuando
no hay cruce — que es lo que ya hacía el simulador, así que las dos superficies dejan de
contradecirse. Y el objetivo FIRE dice en su descripción que está **en euros de hoy**: el objetivo
del año en que te jubiles es bastante mayor, y el nombre solo no lo dejaba claro.

De paso se ata algo que se cumplía por casualidad: la serie del objetivo FIRE se alinea con la del
patrimonio **por posición**, y las dos se construían por caminos distintos que coincidían de milagro.
Ahora la segunda se deriva de la primera, así que no pueden desalinearse.

#### Las herramientas de escritura contestaban en inglés

Crear o editar un flujo planificado devolvía `Coche · 123.45 (Outflow)` —el nombre interno del
código— mientras leerlo devolvía `outflow`. Dos formas del mismo valor en el mismo sitio. Ahora hay
una.

#### Cifras con veintidós decimales

Preguntarle a Claude por tu patrimonio a treinta años devolvía
`69946992.976753373554690255548 €`. No era un error de cálculo —el número es correcto— sino de
presentación: la proyección compone un interés mensual que sale de una raíz duodécima, y nadie
recortaba el resultado antes de mandarlo. Además de ruido, empujaba a presentar cifras con una
precisión que no existe.

Los importes salen ahora con **cuatro decimales**, la misma escala que usa la base de datos. El
recorte se aplica solo a la cifra que se envía, nunca a la que entra en el cálculo: el objetivo FIRE
es también un número interno del motor y redondearlo movería la fecha de jubilación. Con el mismo
arreglo se van dos rarezas: una categoría sin movimientos publicaba su importe como `-0`, y la lista
de hitos mezclaba `25000.0` con `50000` y `100000`.

#### Poner un tope a una regla de reparto podía no hacer nada, y decir que sí

Pedirle a Claude «ponle un tope de 99.999 € a la cartera» devolvía **éxito** y no cambiaba nada: el
tope se manda en dos mitades —el tipo y el valor— y si solo llegaba el valor, se descartaba por el
camino sin que nada lo notara. El caso simétrico (solo el tipo) sí daba error, así que la mitad de
las veces funcionaba y la otra mitad mentía.

Ahora cualquiera de las dos mitades a solas da el mismo error, y poner y quitar el tope a la vez
también. La causa de fondo se arregla en su sitio: la comprobación de «no me has pedido cambiar
nada» estaba escrita a mano en la capa de Claude en vez de vivir en el código compartido con la
API, y ahí es donde se olvidó el campo. Ahora vive donde el resto, y el compilador se niega a
construir el proyecto si alguien añade un campo nuevo y no lo tiene en cuenta.

#### Se podían apuntar movimientos con fecha futura

`2099-12-31` se aceptaba, y el listado de meses lo publicaba como `2099-12`, **mes cerrado y con
datos**. Un movimiento con fecha futura no es un gasto, es un plan: para eso está «Próximos». Ahora
la fecha no puede pasar de hoy, ni al apuntar ni al editar, y el selector de fecha del formulario de
edición tiene el mismo tope que ya tenía el de alta.

#### La curva del pasado no llegaba a tus propias fotos

Si guardabas una foto de tu patrimonio este mes, la curva histórica **no llegaba hasta ella**: podía
quedarse más de mil euros por debajo de un dato que tú mismo habías metido hoy. Y un activo que
aparecía por primera vez en la foto más reciente salía valiendo **cero en toda la gráfica**.

Los dos síntomas eran la misma causa: el último punto de la curva se calculaba a **día 1 del mes**,
no a día de hoy. El mes en curso está a medias, así que se evalúa en la fecha de hoy — igual que ya
hacía el detalle fino del cash-flow. La curva ahora termina exactamente donde dice tu última foto, y
coincide con el patrimonio que ves en el Resumen. En la web no cambia nada visible: la gráfica ya
tomaba el punto del mes actual de la proyección.

De paso, algo que solo veía quien pregunta por Claude: si nunca has fotografiado tus deudas, el
patrimonio histórico no las resta, y un cero era indistinguible de «no debo nada». La cifra sigue
siendo la misma —el histórico es lo que tú fotografiaste— pero ahora la respuesta dice cuál de las
dos cosas es.

#### Dos cifras de ahorro sin decir cuál es cuál

El resumen que Claude recibe trae **dos** ahorros mensuales, y no son intercambiables: uno es el
ahorro real del modo que tengas activo —el que usa la proyección— y el otro es siempre el que sale
de tu presupuesto, que existe solo para poder decirte «vas por encima del plan». En el modo por
defecto valen lo mismo; en los modos que usan tus movimientos reales pueden diferir un 14 %. Nada lo
explicaba, así que elegir el equivocado desplazaba la respuesta.

No cambia ningún cálculo: cambia lo que la herramienta dice de sí misma. Ahora nombra las dos, dice
cuál usa el motor, y cuál es solo el contraste con el plan. En la misma línea, dos aclaraciones más:
el objetivo FIRE se devuelve **en euros de hoy** (el del año en que te jubiles es bastante mayor), y
poner la tasa de retirada a cero no es un escenario conservador — es «jamás», y anula el objetivo
entero.

#### Un filtro de vista mal escrito devolvía los datos de todo el hogar

`?view=` aceptaba **cualquier** valor y, si no era exactamente `mine`, servía el hogar completo sin
decir nada. La app nunca lo notó —manda siempre `mine` o nada—, pero un asistente que escribiera
`"MINE"` en mayúsculas recibía los movimientos de todos los miembros creyendo haber pedido solo los
suyos, y respondía sobre ellos. No era un agujero de permisos (cualquier miembro puede pedir el
hogar entero a la cara, y siempre ha podido), pero sí una respuesta sobre gente distinta de la que
se preguntó, sin ninguna señal.

Ahora `view` admite `mine`, `household` o nada, y **rechaza el resto**. Dos parámetros más tenían el
mismo defecto y van con él: `resolution` del cash-flow (pedir `hourly` devolvía un gráfico semanal
diciendo «semanal») y `density` de la proyección (pedir una densidad inexistente devolvía la serie
completa, diez veces más grande que la pedida).

La causa de fondo era la duplicación: `/v1/projection/series` tenía **su propia copia** del parseo
en vez de usar el compartido, y por eso el arreglo se le habría escapado. Esa copia se ha borrado.
Regresión sobre las 14 rutas con `?view=`: `apps/api/tests/query_param_validation.rs`.

### Simular escenarios: la herramienta solo sabía empeorar el plan

`simulate_projection` es con lo que se contesta «¿y si…?» sobre tu plan. Hasta ahora solo respondía
bien a «¿y si gasto más?»: los tres ajustes mensuales rechazaban cualquier valor negativo, no había
forma de tocar una categoría concreta, ni de cambiar la fuente del ahorro, ni de ponerle fecha a un
cambio, ni de tocar el ingreso, ni de comparar dos escenarios de una vez. Y la cifra final llegaba
en euros nominales a décadas vista, que no dicen nada.

De esa lista, esta versión cierra **los deltas negativos y los ejes de `fire_settings`**. Siguen
pendientes, dichos a las claras: recortar una categoría concreta, ponerle fecha de inicio o fin a
un cambio, tocar el ingreso, y comparar varios escenarios en una sola llamada.

**La descripción de la herramienta era incorrecta, no solo incompleta.** Decía que el gasto extra
«mueve también el target FIRE», y eso solo es cierto con el número FIRE calculado por gasto anual:
si lo calculas por ingreso actual o pones un importe fijo, el objetivo no mira el gasto y el delta
sale 0. Quien lo leía veía un cero y pensaba en un fallo. Ahora la descripción condiciona esa frase
al modo, y cada lado de la respuesta dice con qué modo se calculó.

**Los dos mandos que eran el mismo.** «Ahorro extra» y «ajuste de caja» escriben la misma variable
con el signo cambiado, así que pedir 40 € de ahorro extra es exactamente lo mismo que un ajuste de
caja de −40 €. Eso ya funcionaba, pero no estaba dicho en ninguna parte — y tampoco lo estaba su
consecuencia incómoda: con cualquiera de los dos, los deltas de gasto, neto, tasa de ahorro y
runway salen **cero exacto**, porque un ajuste de caja entra en la caja del mes y no en la base de
gasto. Media respuesta a cero sin explicación parecía un error; ahora se dice que es el contrato, y
se señala cuál es el eje que sí mueve esas cifras.

Además, la cota «cero o más» de esos dos ejes vivía solo en la prosa de la descripción. Ahora viaja
también en el esquema de la herramienta, donde un cliente la lee como restricción y no como texto.

**Cada lado dice ahora con qué se calculó.** La simulación devolvía cifras sin decir de dónde
salían, y eso convertía respuestas correctas en aparentes fallos. El caso claro: si calculas tu
número FIRE con un importe fijo, ningún cambio de gasto puede moverlo — el delta del objetivo sale
0 y es exacto, pero sin saber el modo parece que la herramienta ignoró lo que le pediste. Ahora
cada lado devuelve el modo del número FIRE, la fuente del ahorro que acabó usando, sobre cuántos
meses reales promedió cada mitad, el SWR y la inflación efectivos, y las tres bases de gasto e
ingreso con las que trabajó. Cuando no hay objetivo FIRE, dice **por qué** no lo hay —importe
manual sin poner, la pensión ya cubre el gasto, o SWR a cero— en vez de devolver tres huecos sin
causa. Seis de esos valores ya se calculaban por dentro y se tiraban.

**La cifra final ya se puede leer.** El patrimonio al final de la simulación llegaba en euros
nominales de dentro de cuarenta o cincuenta años, que es una cifra grande y vacía. Ahora viene
acompañado del mismo importe **en euros de hoy**, descontada la inflación que se haya asumido en
ese lado. Si no asumes inflación, las dos cifras son idénticas.

**Ahora se puede preguntar «¿y si cambio de forma de calcular?» sin cambiarla.** De toda tu
configuración FIRE, lo único que la simulación dejaba tocar era la tasa de retirada segura. Todo lo
demás —de dónde sale el ahorro (tu presupuesto o tus movimientos reales), cómo se fija el número
FIRE, si cuentan los impuestos, sobre cuántos meses se promedia cada lado— había que **guardarlo**
para poder verlo, y luego deshacerlo. Ahora se simula sin tocar nada.

Simular un cambio de estos usa exactamente el mismo código que hacerlo de verdad, para que lo que
te enseña la simulación sea lo que pasará si lo guardas. Y si pides promediar tus movimientos reales
pero no hay meses con datos suficientes, la respuesta te dice que acabó usando el presupuesto en
lugar de devolverte en silencio el mismo escenario de partida.

De paso: dos mensajes de error de los ajustes del promedio se devolvían sin traducir. Ya están en
español.

**Ya se puede simular un recorte.** Era el problema de fondo: los tres ajustes mensuales
rechazaban cualquier valor negativo, así que la pregunta más frecuente que existe —«¿cuánto
adelantaría mi jubilación si gasto 200 menos al mes?»— no se podía hacer. Ahora el gasto mensual
extra admite signo, y un recorte mueve todo lo que movía un aumento: gasto total, ahorro neto, tasa
de ahorro, runway, objetivo FIRE y fecha de jubilación.

Si pides un recorte mayor que tu gasto, no se rechaza: la base se queda en cero y la respuesta dice
en qué cifra quedó, para que veas cuánto se aplicó de verdad. Con gasto cero y el número FIRE
calculado por gasto anual no hay objetivo que alcanzar — y también eso se dice, en lugar de
devolver huecos.

### La app no se podía usar recién instalada

Un hogar nuevo nacía con **cero categorías** —la migración original lo decía con todas las letras:
«No server-side seeding; clients create categories as needed»— y la vista de Activos **escondía el
botón de añadir** cuando no había ninguna. El primer usuario aterrizaba en un Resumen en blanco,
iba a Activos y se encontraba una pantalla sin salida cuya única pista era una miga de pan de dos
palabras («Activos · Ajustes → Categorías») que ni siquiera era un enlace.

- **El hogar nace con categorías** (`seed_default_categories`): cuatro de activo, tres de pasivo,
  dos de ingreso y siete de gasto, dentro de la misma transacción que crea la instalación. Son un
  punto de partida, no un dogma: se renombran y se borran como cualquier otra.
- **El botón «+» ya no se esconde nunca.** Si de verdad falta una categoría, se queda deshabilitado
  y el estado vacío explica por qué y a dónde ir.
- **Asistente de primera vez** (`OnboardingWizard`): divisa y zona horaria → inflación y tasa de
  retirada → primer activo → un resumen de para qué sirve cada pestaña. Saltable, y reabrible
  desde Ajustes → General. La zona horaria se propone desde el navegador: el servidor ponía `UTC`,
  y con eso «el gasto de hoy» podía caer en el día equivocado.
- **Estados vacíos con acción** en Resumen, Activos, Pasivos, Presupuesto y Próximos, siguiendo el
  patrón que ya funcionaba en Movimientos. Con ellos se unifica la política de ceros, que estaba
  partida: el Resumen ocultaba las KPI a cero mientras el resto pintaba `0 €`. Ahora la unidad es
  el **bloque**: con datos se pintan todas las cifras (un cero real es información), y sin datos el
  bloque entero deja paso a una explicación.

### La divisa base estaba clavada a EUR

`bootstrap_installation_as_owner_if_empty` insertaba `VALUES ('EUR', 'dates')` y `base_currency` no
estaba en el PATCH de la instalación. El único selector de divisa del código vivía en
`BootstrapInstallationPanel`, **inalcanzable**: el registro crea la instalación, así que la pantalla
que lo contenía no llegaba a mostrarse nunca. Un usuario fuera de la eurozona se quedaba en euros
para siempre, con «Moneda base: EUR» en Ajustes y ningún control al lado.

Ahora `base_currency` se cambia en **Ajustes → General** (owner-only, EUR/USD/GBP) y en el paso 1
del asistente. **Una sola divisa por instalación**: FutureFin no convierte ni mezcla, y cambiarla
no reconvierte los importes ya guardados — el aviso lo dice antes, no después.

De paso, el import de CSV deja de exigir euros a fuego: valida contra la divisa del hogar. El
código de error `currency_not_eur` pasa a llamarse **`currency_mismatch`**, que es lo que de verdad
comprueba.

### La pantalla de «acceso pendiente» era una trampa

Quien se registraba en segundo lugar veía esto, y nada más: «Acceso pendiente» + «Ajustes →
Usuarios» — una instrucción **para el propietario**, enseñada a quien espera. No podía cerrar
sesión (el botón vive dentro de Ajustes, inalcanzable en ese gate), y las nueve pestañas de
navegación se pintaban igual aunque ninguna hiciera nada al pulsarla.

Ahora explica qué pasa y con qué usuario se registró, ofrece **cerrar sesión** y **comprobar
ahora**, se refresca sola cada 15 segundos —para entrar en cuanto la aprueben— y la navegación
muerta desaparece (`TopBar` gana `showNav`).

### Cuatro borrados permanentes iban a un clic

Activo, pasivo, línea de presupuesto y movimiento previsto se borraban sin modal, sin deshacer y
sin aviso, mientras categorías, snapshots, movimientos y tokens **sí** confirmaban: la misma app
con dos criterios opuestos, y el peligroso era el que no preguntaba. Ahora los cuatro pasan por una
confirmación que nombra lo que se va a borrar. Se intercepta en el borde de `App.tsx`, así que las
vistas no se enteran.

### Ajustes: ocho apartados partidos por dónde vive el dato

La sub-pestaña «Jubilación» contenía **solo** los tramos de IRPF, mientras el SWR y el objetivo FIRE
vivían en la **pestaña** «Jubilación»: dos cosas con el mismo nombre y mitades del mismo concepto.
«Proyección» mezclaba un supuesto económico (inflación), una preferencia de visualización (modo
edad) y el modo del motor bajo una sola cabecera. Y el propietario aterrizaba en «Usuarios» →
«Nadie pendiente», mientras el resto aterrizaba en «MCP», la página más técnica de la app.

Ahora son siete, ordenadas de lo que casi todo el mundo toca a lo que toca casi nadie:
**General** (apariencia, divisa, zona horaria, asistente, datos de la instalación y estado del
sistema) · **Plan** (todo el plan junto) · **Categorías** · **Histórico** · **Usuarios** ·
**Integraciones** (MCP, tokens, conexiones) · **Copias de seguridad**.

Los slugs antiguos siguen resolviendo (`/ajustes/mcp` → Integraciones, `/ajustes/jubilacion` →
Plan): un enlace guardado que no se reconoce acaba en la primera sub-pestaña **sin decir nada**,
que es peor que un 404.

### Fixed — el aviso de la inflación llevaba a la pantalla equivocada

El banner de Jubilación navegaba a `/ajustes` a secas y el canonicalizador lo reescribía a la
primera sub-pestaña: hablaba de la inflación y te dejaba en la pantalla de aprobar usuarios. Ahora
va a **Ajustes → Plan**, donde está el ajuste del que habla.

### Fixed — el guardado automático del plan fallaba en silencio

`runFireSave` salía **sin guardar y sin avisar** cuando el SWR estaba fuera de rango o faltaba el
objetivo manual, mientras el pie del panel seguía prometiendo «Guardado automático». El usuario
movía el control, leía que se había guardado, y se iba con el cambio perdido. Ahora sale un aviso.

### CI: lo que nunca se ejecutaba, y una limpieza que era un no-op por accidente

CI corría `cargo build`, los tests del engine, typecheck y build de la web, más el escenario
Docker. **No corría** ESLint, ni Vitest, ni los tests de integración contra Postgres — que son
**la mayor parte de la suite**. Con colaboradores externos eso no
aguanta: quien manda una PR no va a levantar un Postgres a mano.

- Job `integration` nuevo, con `services: postgres:16.4-alpine` y `cargo test --workspace`. El
  `pg_isready` lleva `-h 127.0.0.1` a propósito: durante el `initdb` la imagen oficial levanta un
  servidor temporal que solo escucha en el socket Unix, y sin host el healthcheck da OK antes de
  que la base exista — el mismo flake que ya mordió en el paso de `pg_upgrade`.
- `npm run lint:web` y Vitest, verificados verdes antes de entrar como bloqueantes.
- `cargo clippy` y `cargo fmt --check` quedan **preparados y comentados**, con los números
  medidos al lado: 50 avisos únicos de clippy en 20 ficheros y 1.175 bloques de formato en 72.
  Meterlos en rojo hoy sería dejar CI rota, que es peor que no tener el gate.
- Job `main-guard`: `main` no publica `CLAUDE.md` ni `.claude/`. **Ver el aviso de CLAUDE.md
  § Git workflow**: el guard ya está, la limpieza de `main` todavía no, así que hay un orden que
  respetar.

### Un tag publicaba imagen y Release aunque CI estuviera en rojo

Nada conectaba `publish-image.yml` con `ci.yml`. Ahora hay un job `ci-gate` del que depende la
publicación. Ni `needs:` ni `on: workflow_run` servían —el primero solo enlaza jobs del mismo
workflow y el segundo no dispara porque CI **no corre en tags**—, así que la puerta consulta a la
API el resultado de CI sobre el **SHA exacto** al que apunta el tag. El flujo de release mergea
dev→main, empuja y después taguea: ese commit siempre ha pasado por CI.

También: el push a Docker Hub y su login se condicionan a que existan los secretos —antes un fork
reventaba, y no en el login sino en el push, porque el nombre entraba igualmente en la lista de
imágenes—, y la imagen gana **seis etiquetas OCI** (`source`, `url`, `title`, `description`,
`licenses` = `AGPL-3.0-only`, `vendor`). Sin `source`, el package de GHCR no enlaza con el
repositorio; sin `licenses`, sale «sin licencia». Van explícitas y no autodetectadas porque
`metadata-action` las saca de la API de licencias de GitHub, que no responde igual en un repo
privado ni en un fork.

### `cleanup-ghcr.yml` no borraba nada, y era pura suerte

El workflow semanal borraba versiones **sin tag** de más de 60 días. En un package multi-arquitectura
—este se publica para amd64 y arm64— las versiones sin tag son **los manifests hijos de los tags
publicados**: borrarlas deja `:3.0.0`, `:2.3.0`… apuntando a capas que ya no existen y el `docker
pull` falla. Hoy hay 24 versiones sin tag.

Al auditarlo con fixtures aparecieron **otros dos** bugs en el jq: un `as` sobre un flujo vacío
—no existe ninguna versión con tag `dev`— que en jq **anula toda la expresión posterior**, y una
precedencia rota que reventaba con «Cannot index boolean». Los dos errores se los tragaba un
`2>/dev/null || true`. O sea: la salvaguarda que impedía la catástrofe era **accidental**, y
alguien que "arreglara" el jq sin entender el multi-arch habría empezado a destruir releases
publicadas de inmediato.

Ahora la regla es explícita: **solo se borra una versión cuyos tags sean todos `sha-*`**, y una
versión sin tag no se toca jamás. Fuera el `2>/dev/null`, y añadido un `dry_run` por defecto en las
ejecuciones manuales — es un workflow irreversible cuyo camino de borrado no se había ejercitado
nunca.

### La descripción de Docker Hub estaba vacía

`maxlainz/futurefin` es público, lleva 3.285 descargas y no dice qué es. Nuevo
`.github/dockerhub-README.md` (español, con el `docker compose` mínimo) sincronizado por un
workflow con `peter-evans/dockerhub-description`.

### Marca: la pestaña del navegador enseñaba el icono por defecto

`apps/web/index.html` eran once líneas sin favicon, sin descripción y con un `<title>` de una sola
palabra, y **no existía ni un solo fichero de imagen en el repositorio**: el «logo» era un cuadrado
CSS con las letras `FF`. Ahora hay `favicon.svg` (la misma marca, en SVG), `apple-touch-icon.png`,
`site.webmanifest`, `<meta name="description">` y `theme-color` por esquema de color. Los tres
ficheros viven en `apps/web/public/`, así que el build los copia a `dist/` y `ServeDir` los sirve
antes que el fallback de la SPA.

### Migración

`20260822120000_installation_onboarding.sql` — aditiva y sin pérdida. Añade
`installation.onboarding_completed_at`; las instalaciones que ya existen se marcan como
completadas, porque su dueño ya configuró el hogar a mano y enseñarle un asistente de bienvenida
ahora sería absurdo.

### Limpieza

`PlaceholderTab` («Próximamente.») era inalcanzable desde que las nueve pestañas tienen vista
propia: fuera, junto a su rama de render. La clase `dev-panel` deja de viajar a producción y
«Estado del sistema» ya no enseña `/v1/health` como si el usuario supiera qué es.

### Los errores de la API se pintaban en inglés y en jerga

`ErrorBody.message` viajaba del backend a la SPA y se enseñaba **literalmente**: la cadena era
`error.rs` → `api/client.ts` → `throw new Error(body.message)` → cada `setError(e.message)`, unos
cincuenta sitios. El resultado eran frases como «resource conflict» al registrar un usuario
repetido, o «currency_not_eur: row 3 has currency 'USD' (only EUR is supported)» al importar un
CSV, en una interfaz por lo demás íntegramente en español.

La API **sigue hablando inglés**: es superficie para desarrolladores, para OpenAPI y para clientes
de terceros. Lo que cambia es que ahora manda además un **código estable** con el que traducir.

#### Contrato (aditivo, no rompe nada)

`ErrorBody` gana el campo `code`, junto a los ya publicados `error` y `message`:

```json
{ "error": "conflict", "code": "username_taken",
  "message": "username_taken: that username is already registered" }
```

- `code` sale del prefijo `snake_code: ` del mensaje —una convención que **ya existía a medias** en
  el repo (`csv_preset_unrecognized:`, `preview_confirm_mismatch`)— y que ahora se aplica en los
  ~307 sitios de validación. Sin prefijo válido cae a la clase HTTP, que también es un código.
- El criterio de `derive_error_code` es estrecho a propósito (3–64 caracteres, `[a-z][a-z0-9_]*`):
  un mensaje corriente con dos puntos no debe inventar un código. Un código inventado es **peor**
  que ninguno, porque el catálogo no lo tendrá y el usuario verá el genérico creyendo que hay
  traducción.
- Dos variantes nuevas de `ApiError` existen solo para poder llevar código donde antes no cabía:
  `ConflictWith` (el `Conflict` pelado nace del mapeo automático del SQLSTATE 23505 y no sabe QUÉ
  colisionó) y el ya existente `NotFoundWith`.

#### En el cliente

`ApiRequestError` sustituye al `Error` pelado y su `.message` **ya viene en español**, así que los
~50 `setError(e.message)` muestran español sin tocar una línea. El texto técnico queda en
`.detail` y se manda a la consola: depurar un 400 no debería obligar a abrir la pestaña de red.

`apps/web/src/lib/errorMessages.ts` es el catálogo, agrupado por dónde las ve el
usuario. Regla de estilo: frase completa, qué ha pasado y qué puede hacer, sin nombres de campo del
API ni jerga HTTP.

#### El gate

Un código sin traducir no rompe nada —cae al genérico— y por eso hacía falta un test: el fallo es
silencioso. `apps/api/tests/error_codes_parity.rs` (sin Postgres) extrae del fuente **todos los
códigos** a `tests/fixtures/error-codes.json`, y `errorMessages.test.ts` lee ese mismo JSON y falla
si alguno se queda sin frase, o si sobra una frase para un código que ya no existe.

La primera versión del extractor solo miraba los constructores de `ApiError` y **se dejaba seis
códigos** de `backup_user/`, donde el error nace como `CryptoError` o como un `Err(String)` y solo
se convierte más arriba. Y recortar por el primer `#[cfg(test)]` costó otros diez, porque
`projection.rs` tiene módulos de test **en medio**. Ahora barre todo literal con forma de código,
salta los módulos de test contando llaves, y lo que sobra se excluye a mano con su porqué escrito
al lado: capturar de más cuesta una línea; capturar de menos no se nota.

#### Fixed — la contraseña equivocada de un backup decía «tu sesión ha caducado»

`CryptoError::Decrypt` mapeaba a `ApiError::Unauthorized` (401). Con el catálogo en español ese 401
se habría leído como «Tu sesión ha caducado. Vuelve a iniciar sesión» — y el usuario se habría ido
a reiniciar sesión en vez de reescribir la contraseña del fichero, que es el error más frecuente de
todo el flujo de importación. Ahora es **400 `backup_wrong_password`**: la sesión es válida; lo que
no cuadra es la contraseña del archivo.

### Fixed — los importes con coma se rechazaban en la mitad de los formularios

En el formulario de activos, «rentabilidad esperada» y «precio de compra» convertían la coma
decimal antes de enviar, y «valor actual» no. Teclear `1234,5` en el valor —con `inputMode="decimal"`
y placeholders que invitan a la coma— lo rechazaba el backend con un error en inglés. Lo mismo con
el principal y la TAE de un pasivo, el importe del presupuesto, el de un movimiento previsto y los
de las reglas de reparto.

La conversión pasa a un único sitio, `toApiDecimalString` en `lib/format.ts`, y todo lo que se
envía a la API pasa por él. Como contrapartida, `formatEditableDecimalString` ahora **sirve el
valor con coma** (`2,5`, no `2.5`): es lo que el usuario espera teclear y lo que ya sugerían los
placeholders. Los `<input>` son de texto con `inputMode="decimal"`, no `type="number"`, así que la
coma no rompe nada. Hay un test que cierra el ciclo: lo que se precarga en un input tiene que poder
reenviarse tal cual.

### Changed — etiquetas que seguían en inglés

`Focus` → **Vista cercana**, `Inflation Adjusted` → **En dinero de hoy**, `Milestone` → **Hito**,
`Budget` → **Presupuesto**, `Target FIRE` → **Objetivo FIRE**, `Runway` → **Autonomía** (también su
título en el catálogo de ayuda), `PnL vs compra` → **Ganancia vs compra**, `Actual / Target` →
`Actual / Objetivo`, `YTD` → **Año** / «año en curso», `items` → `ítems`, «solo el owner» → «solo el
propietario».

Y los valores crudos de la API que se pintaban sin traducir: el rol (`owner`/`member`/`viewer`) se
enseñaba traducido en el `<select>` de Ajustes pero **crudo** en la píldora de la cuenta y en la
ficha de la instalación, así que el mismo usuario se veía como «Miembro» en un sitio y «member» en
otro. Nuevo `lib/enumLabels.ts` con los rótulos, usado en los cuatro sitios (incluye el estado del
servicio, `ok` → «Correcto»).

#### Pendiente, dicho a las claras

El detalle técnico se manda a la consola pero **todavía no se enseña plegado** bajo «Detalles
técnicos»: los ~14 estados de error de `App.tsx` guardan una cadena, no el objeto, y convertirlos
es un cambio de sesenta sitios que no toca hacer en medio de un barrido de idioma. Va con la
reorganización de `App.tsx` del onboarding.

### Higiene de datos — los fixtures del importador eran extractos bancarios reales

Auditando el repositorio antes de hacerlo público se encontró que
`apps/api/tests/fixtures/n26_junio.csv` y `myinvestor_junio.csv` no eran fixtures fabricados sino
**exportaciones auténticas**: IBAN español completo, nombre y apellidos de una persona, nómina al
céntimo de dos meses consecutivos, gimnasio con sucursal, calle y barrio, y el perfil completo de
suscripciones. El IBAN estaba en el árbol de **109 commits**.

La cabecera del propio fichero de tests decía «Los CSV son fixtures **anonimizados** de los bancos
reales». Ahí está la trampa: anonimizar un export real es borrar sobre datos que siguen ahí, y no
tiene estado final verificable; **fabricar** un fixture sí lo tiene.

- Los dos CSV se han **reconstruido desde cero** conservando cada caso que los tests ejercitan: la
  cabecera literal de cada banco, la escala rara de N26 (`-26.000000000` → 4 decimales), el decimal
  español con coma, filas sin `Partner Name`, el par opuesto a ≤3 días, el partner «Cuenta de
  Ahorro», los tokens `TRANSFERENCIA`/`ENVIADA DESDE`/`ESTALVI`, el hint de ahorro por
  `APORTACION`/`CARTERA`, el sufijo numérico variable que colapsa varias filas en un solo patrón de
  regla aprendida, y el sufijo de referencia que `derive_rule_pattern` recorta.
- **Ningún IBAN, ni siquiera sintético**: el parser de N26 no lee esa columna, así que va vacía. Un
  IBAN falso solo serviría para disparar el escáner para siempre.
- `myinvestor_win1252.csv` ya era sintético (prueba de codificación) y no se ha tocado.
- Saneados también los literales derivados de esos extractos en los tests unitarios de
  `handlers/transactions/schema.rs` y en `handlers/backup_user/schema.rs`.

### Las tablas del CHANGELOG citaban una instalación real

Las entradas de 3.9.0 y de la auditoría del promedio razonaban «sobre una instalación **real**» y publicaban el
alquiler, el ingreso mensual y la tasa de ahorro del owner. Las cifras pasan a ser inventadas y la
fórmula sigue cuadrando: donde había `540,00 ÷ 6` vs `÷ 3` ahora hay `540,00 ÷ 6` vs `÷ 3` → 90 y
180 €. Un ejemplo que no cuadra vale menos que ninguno.

### Para que no vuelva a pasar

- **`scripts/scan-sensitive.sh`** — escáner de los ficheros trackeados: IBAN, tarjetas, claves
  privadas y tokens de GitHub/AWS/Slack/OpenAI-Anthropic/FutureFin. Excepciones en
  `scripts/sensitive-allowlist.txt`, cada una con el porqué escrito al lado. Verificado en ambos
  sentidos: **detecta** el IBAN del fixture antiguo y **pasa** con los nuevos.
- **Job `secrets-scan` en CI**, bloqueante y el primero de todos.
- **`apps/api/tests/fixtures_shape.rs`** (3 tests, **sin Postgres**): fija el contrato del material
  de entrada del importador y que ningún fixture lleva una cadena con forma de IBAN. Falla en
  segundos si alguien vuelve a tocar los CSV. Control negativo comprobado: falla contra el fixture
  antiguo.
- **Skill `futurefin-data-hygiene`**: qué no entra nunca, cómo se fabrica un fixture que siga
  valiendo como prueba, y el procedimiento si algo se cuela (reescritura del historial, no borrado).
- **No negociable §2.0** en `futurefin-change-control` y **§3.2b** en `futurefin-docs-and-writing`
  (las cifras de ejemplo son inventadas pero aritméticamente coherentes).

### Movido

`.claude/skills/futurefin-diagnostics-and-tooling/scripts/` → **`scripts/diagnostics/`**. La rama
publicada no va a llevar `.claude/`, y el gate de shellcheck de CI apuntaba ahí dentro; el comodín
`scripts/*.sh` no alcanza subdirectorios, así que la ruta se lista explícitamente.

Dos cifras que el consumidor no podía interpretar sin recalcularlas a mano: el promedio de la
comparativa mensual y la jubilación de las tools de proyección. Aditivo en el contrato; **cambia
números** en la pestaña Gastos y en `get_transactions_summary`.

### El promedio contaba como cero los meses sin datos reales

`GET /v1/transactions/summary` dividía entre `months_with_data` = meses del tramo con ≥1
movimiento **de cualquier tipo**. Un mes cuyo único contenido eran instancias recurrentes contaba
como mes con datos, así que hundía la media de todas las demás categorías. Sobre una instalación
de ejemplo con importación completa solo de abril a julio de 2026 y el alquiler recurrente materializado
desde noviembre, ventana `6` sobre julio:

| Categoría | Antes | Ahora | Por qué |
|---|---|---|---|
| Comer Fuera | 90 € | **180 €** | 540,00 ÷ 6 vs 540,00 ÷ 3 |
| Supermercado | 120 € | **240 €** | mismo denominador |
| Alquiler | 700 € | **700 €** | su cuota real, no 1.400 € |

El denominador pasa a ser `avg_months` = meses del tramo con ≥1 movimiento **real**
(`recurring_rule_id IS NULL`) — el mismo predicado que ya usaba `transactions_avg` para alimentar
el engine en los modos B y C. La divergencia entre ambos estaba anotada en el código como
deliberada, «no alinear sin una decisión de producto»: la decisión se tomó.

Un mes no real queda fuera del **numerador y del denominador** a la vez. Excluirlo solo del
denominador dejaría su importe arriba y dispararía las categorías presentes en él: el alquiler de
700 €/mes saldría a 1.400 €.

El denominador sigue siendo **único para todas las líneas**, no por categoría. Así
`Σ avg de categorías == totals.expense_avg` y el KPI «Gasto promedio» y la tasa de ahorro no se
inflan. La contrapartida, aceptada y ahora documentada en los textos de ayuda: un mes real sin
movimientos de una categoría concreta sí cuenta como cero para ella — es la media del hogar, no
«cuánto gasto cuando gasto».

Sigue sin cambiar: la ventana es de calendario (`"6"` = seis meses civiles anteriores), el mes
seleccionado sigue excluido, y las transferencias conciliadas siguen fuera de todos los buckets.

#### Añadido al response (aditivo)

- `avg_months` — **el denominador**. `0` ⟺ no hay promedio y todas las medias son 0.
- `months_with_data` — **sin cambios de semántica**: meses con movimientos de cualquier tipo. Se
  mantiene porque describe lo que hay en el tramo; ya no es el denominador, y su doc lo dice.
- `avg_basis {months, first_month, last_month, has_gaps}` — de qué meses sale la media. `has_gaps`
  impide etiquetar «abr–jun» una media de abril y junio.
- `avg_unavailable_reason` — `"empty_window"` (no hay nada) vs `"only_recurring_months"` (hay, pero
  todo recurrente). Piden acciones distintas: importar histórico vs bajar la ventana.

En la pestaña Gastos las tarjetas de promedio muestran la base en el paréntesis («media de abr
2026–jun 2026»), porque «Promedio 6m» sobre tres meses de datos se lee como seis meses de datos.

### La jubilación viajaba como índice de mes, sin fecha ni edad (issue #6)

`simulate_projection` devolvía `jubilacion_month_index: 137` y **ninguna ancla con la que
convertirlo**: la respuesta no llevaba ni la fecha del mes 0 ni la de nacimiento, así que el
consumidor tenía que encadenar una llamada a `get_projection` y hacer a mano la aritmética de
calendario y de edad — meses → fecha civil con recorte de fin de mes → años cumplidos. Es
exactamente el cálculo en el que un LLM se equivoca en silencio.

- `jubilacion_date_ymd` y `jubilacion_age` en los KPIs de `simulate_projection` **y** en
  `GET /v1/projection/series`. El índice **no** desaparece: sigue siendo la clave para indexar las
  series.
- `simulate_projection` devuelve además `anchor_date_ymd`, `show_age_mode` y `viewer_birth_date`:
  la respuesta es autocontenida. Todo sale del contexto que `simulate_projection_core` ya resolvía
  y descartaba — **cero queries adicionales**.
- `jubilacion_months_delta` de `deltas` se queda en meses: ahí el delta en meses es la unidad
  natural.

La fecha se calcula sumando N meses al ancla **conservando su día**, con recorte a fin de mes
(31 ene + 1 mes = 28 feb) — exactamente `addMonthsCivil` de la web, de modo que la edad servida
coincide con la etiqueta del chart. Anclar al día 1, como hacen los hitos, restaría un año cuando
el cruce cae en el mes de cumpleaños; hay un test que lo demuestra. `ProjectionMilestone.reached_date_ymd`
conserva su día 1 (contrato ya publicado): ambas fechas coinciden siempre en año y mes.

`jubilacion_age` es `null` sin fecha de nacimiento resuelta, con independencia de `show_age_mode`.

### Tests

- `transactions_summary.rs` +4: el pin del mes solo-recurrente fuera de ambos lados (con la
  aditividad Σ líneas == total), un mes real contando sus recurrentes, `has_gaps` con meses no
  contiguos, y los dos motivos de «sin promedio». Los pins previos del denominador pasan sin
  tocarlos.
- `jubilacion_civil_tests` en `handlers/projection.rs` (8, sin DB): clamp de fin de mes incluido un
  29 de febrero, salto de año, `mi = 0` (ya-FIRE hoy) y la prueba del año de diferencia que
  justifica anclar al día del ancla.
- `mcp_simulate.rs`: paridad de fecha, edad y ancla entre `simulate_projection` y `get_projection`,
  y coherencia fecha ↔ índice.

### Paridad MCP

Desenlace de la evaluación de `futurefin-mcp-parity`: **tool actualizada ×3** (`get_projection`,
`simulate_projection`, `get_transactions_summary`), ninguna omisión. Las tres comparten core con
sus handlers HTTP, así que no hubo código MCP que tocar — solo sus descripciones, que ahora
describirían mal el denominador y la jubilación.

### Deriva de documentación corregida de paso

`CLAUDE.md`, `.claude/api-routes.md` y la skill de FIRE llamaban `transactions_12m_avg` a un helper
que se llama `transactions_avg`.


## [3.9.0] - 2026-08-21

Una sola cifra de ahorro por modo, ventanas del promedio configurables por lado, y los recurrentes
siguiendo a los datos reales. **Breaking de números y de contrato**; migración destructiva firmada
por el owner; `.ffbackup` sube a **9**.

### El problema

El Resumen enseñaba **tres** cifras de ahorro simultáneas, todas aritméticamente correctas y
mutuamente irreconciliables. Sobre una instalación de ejemplo:

| KPI | Ingreso | Gasto | Neto |
|---|---|---|---|
| «Ahorro mensual neto» | 2.500 (presupuesto) | 1.890,00 (real) | **610,00** |
| «…de 650 € esperados» | 2.500 (presupuesto) | 1.850,00 (presupuesto) | **650,00** |
| «Ahorro real» | 2.410,00 (real) | 1.890,00 (real) | **520,00** |

En modo C la cifra que proyectaba el motor (610,00 €) no aparecía en **ninguno** de los dos lados
de la comparativa. En modo A la tarjeta duplicaba el denominador y en modo B el numerador, así que
nunca aportaba información propia. Y `savings_rate` (24,4 %) mezclaba bases —neto híbrido sobre
ingreso de presupuesto—, ni 26,0 % (plan) ni 21,6 % (real). Nadie mentía: ninguna tarjeta decía
cuál era su base.

### Added — ventanas del promedio real configurables por lado

- Cuatro ejes nuevos en `installation.fire_settings`: `income_avg_window_months` /
  `income_avg_window_mode` (default **3 / calendar**) y `expense_avg_window_months` /
  `expense_avg_window_mode` (default **12 / calendar**), cotas 1–60. El modo A no usa ninguna, el
  B las dos y el C solo las de gasto.
- **Por qué asimétricas**: el ingreso es una serie con **escalón** (una subida de sueldo) y el
  gasto es ruidoso pero estacionario. Un promedio plano de 12 meses es el estimador equivocado
  para el primero — arrastra los meses previos a la subida durante un año — y el correcto para el
  segundo. Con ventanas por lado se expresa «ingreso reciente contra gasto histórico» **sin
  mezclar plan y realidad**, que era el defecto del modo C.
- Semántica configurable: `calendar` (los meses con datos dentro de los últimos N civiles) o
  `data` (los N meses **con datos** más recientes, saltando los vacíos).
- Panel nuevo en **Ajustes → Proyección**, visible solo en los modos que promedian.
- Tool MCP `update_fire_settings` actualizada (paridad: *tool updated*; el catálogo sigue en 50).

### Changed — los recurrentes convergen a los meses con datos (**breaking**)

- El cursor monotónico `last_materialized_month` se sustituye por el ancla `origin_month` y una
  **invariante declarativa**: *una instancia de R existe en el mes M ⟺ M es un mes **activo** de
  la instalación y `M >= R.origin_month`*. **Mes activo** = mes civil cerrado con ≥1 movimiento
  real no conciliado.
- El cursor era monotónico, justo lo contrario de lo que hace falta: un CSV de marzo-2025
  importado en abril-2026 dispara un mes que el cursor ya había pasado. Y materializar meses sin
  datos producía meses «pseudovacíos» que el promedio del motor tenía que excluir aparte.
- **Cambios de comportamiento visibles**: borrar una instancia a mano ya **no** la borra para
  siempre (vuelve mientras su mes siga activo; para quitarla se borra la plantilla); el alta con
  fecha pasada ya **no** backfillea meses vacíos; `materialize` pasa a ser una convergencia bajo
  demanda de ámbito instalación y devuelve además `pruned`.
- **Migración destructiva** (`20260821120000_recurring_converge_on_real_movement`): borra las
  instancias recurrentes alojadas en meses sin movimientos reales, **incluida la del mes de
  origen**. Es **FIRE-neutral por construcción** — esos meses ya estaban excluidos por completo
  del promedio que alimenta el motor, así que proyección, target FIRE y runway no se mueven ni un
  decimal. Lo que cambia a propósito es el promedio de la pestaña Movimientos y el listado
  visible. El entrypoint escribe su backup pre-migración automático antes.
- Idempotencia **por existencia**, respaldada por un índice UNIQUE parcial. El cast `::timestamp`
  de su expresión es obligatorio: `date_trunc(text, timestamptz)` es STABLE y no es indexable.

### Changed — una sola cifra de ahorro por modo

- Salud financiera pasa de **cinco tarjetas a tres**. La de ahorro enseña el neto **efectivo** del
  modo (el que usa la proyección) como valor, su tasa como detalle y el contraste con el plan como
  tendencia. Valor y tasa comparten base **por construcción**: no pueden contradecirse.
- `MetricCard` gana un **segundo slot** (`detail`), también siempre reservado, para no romper la
  alineación de baseline entre KPIs de una fila.
- Los KPIs de Movimientos se renombran: «Ahorro promedio» → **«Traspasado a ahorro»** y «Tasa de
  ahorro» → **«% traspasado»**. Eran el bucket de movimientos marcados como ahorro —dinero
  apartado explícitamente, no ingresos menos gastos— con el mismo rótulo que conceptos distintos
  del Resumen, y 11 puntos de diferencia.

### Added — popover de ayuda y catálogo de definiciones

- Cada métrica y cada ajuste que dependa de una base o de una ventana estrena un interrogante que
  abre su descripción. `HelpPopover` es un diálogo **no modal** anclado, con cierre por Escape y
  clic fuera, clampado al viewport.
- **`apps/web/src/lib/helpTexts.ts` es la fuente de verdad en prosa** de cada métrica: qué mide,
  con qué base, con qué ventana. Si el código y el texto discrepan, uno de los dos es un bug.
- Skill nueva **`futurefin-metric-definitions`** con esa disciplina, enganchada a la tabla de
  enrutado de CLAUDE.md y a la §1 de `futurefin-change-control`: tocar la semántica de una métrica
  debe acabar en exactamente uno de {texto actualizado, entrada añadida/retirada, n/a razonado}.
- Test de cobertura en las **dos** direcciones: ni iconos sin texto ni textos huérfanos.

### Removed — vestigios del contrato (**breaking de API**)

De `financial_health`: `expense_derived_monthly_equivalent` (siempre 0 en los tres modos desde
3.7.0), `monthly_net_excluding_derived_debt` y `savings_rate_excluding_derived_debt` (idénticos a
sus gemelos por construcción) y `savings_actual_monthly_avg_12m` / `savings_actual_months_with_data`
(la comparativa que desaparece). **`savings_expected_monthly_equivalent` se queda**: alimenta el
delta «vs plan».

`savings_source_months_with_data` → **`savings_income_basis`** y **`savings_expense_basis`** en
`/v1/summary` y `/v1/projection/series`: con dos ventanas no existe *un* número de meses, y servir
uno solo mal-etiquetaría la mitad de la UI. Cada bloque trae `basis`, meses usados, ventana
configurada, rango real y `has_gaps` — este último impide pintar «media de ene–dic 2025» sobre
doce meses dispersos en tres años.

**Ganancia colateral**: los `savings_actual_*` eran el único consumidor del promedio real en modo
A. Al retirarlos, el promedio pasa dentro del gate del modo y el **modo A por defecto deja de
tocar el ledger** en el endpoint más caliente de la app.

### Compatibilidad

- **`.ffbackup` 8 → 9**: `BackupRecurringRule.last_materialized_month` → `origin_month`. La
  migración `payload_v8_to_v9` ancla en la instancia **más antigua** del payload, no en el cursor
  (que iba por delante del origen, así que copiarlo impediría materializar los meses intermedios).
  Los ficheros v1..v8 siguen importando.
- **Los números se mueven** para quien esté en modo B: la ventana de ingreso pasa de 12 a 3 meses
  por defecto. En modo C no cambia nada (el gasto ya usaba 12). Poner ambas ventanas a 12 con
  semántica `calendar` reproduce exactamente el comportamiento anterior.
- Sin cambios en el login, las rutas ni el catálogo MCP.

### Tests

421 de integración/unitarios en Rust y 334 en el frontend. El del promedio ponderado pasa a ser el
**discriminante de las ventanas** (12/12 → 1200, 3/12 → 1800 sobre los mismos datos: con una sola
ventana ambos casos darían lo mismo). Dos unitarios nuevos fijan que la migración v8→v9 ancla en la
instancia más antigua y no en el cursor — el fixture v6 existente tenía ambos en el mismo mes y por
tanto no discriminaba. Seis tests que probaban el KPI retirado se van con él.


### Changed — La conciliación de transferencias deja de tener botón y gana una red de reintento

- **El malentendido que lo motivó**: «Conciliar ahora» parecía una tarea manual que se hace una vez
  al mes. No lo era: el pase automático ya corría **tras cada mutación** — alta, lote, edición de
  importe/fecha, borrado, **confirm de import CSV**, materialización de recurrentes e import de
  `.ffbackup`. Por eso su mensaje habitual era «Sin transferencias que conciliar».
- **El hueco real, que sí existía**: esos pases son **best-effort por diseño** (un fallo se loguea y
  no convierte una escritura ya persistida en un 5xx, porque el cliente reintentaría y duplicaría el
  movimiento). El precio era que un fallo puntual dejaba el par sin conciliar **para siempre y en
  silencio**: nada lo reintentaba, y el usuario no podía enterarse para pedir el pase manual.
- **La solución**: `sweep_all_owners` + la **primera tarea periódica del binario**
  (`FUTUREFIN_RECONCILE_SWEEP_HOURS`, default **24 h**, `0` la desactiva). Recorre cada
  `(installation, owner)` con movimientos sin conciliar y repite el mismo algoritmo. Un owner que
  falla no aborta el barrido: se cuenta y se reintenta a la siguiente pasada. La primera pasada va
  **tras el primer intervalo**, no al arrancar, y la tarea se **aborta antes de cerrar el pool** en
  el apagado ordenado.
- En una instalación sana el barrido no encuentra nada — el pase es de punto fijo — y loguea a
  `debug`; solo sube a `info` si concilió algo o si algún owner falló.
- **Se retira el botón «Conciliar ahora»** de Movimientos. `POST /v1/transactions/reconcile` y la
  tool MCP `reconcile_transfers` **siguen existiendo**: la recuperación manual no se pierde, solo
  deja de ocupar sitio en una barra de acciones para algo que ya es automático.
- Sin migración y sin cambio de contrato de API. Tests: 4 nuevos en `transactions_reconcile.rs`
  (recupera lo que un pase perdió, recorre todos los owners, **nunca resucita** un par que el
  usuario desconcilió, y es no-op con todo conciliado), verificados con mutantes.

### Fixed — El barrido de conciliación no invalidaba la cache de proyección

- **El bug**: `sweep_all_owners` recibía un `PgPool`, no el `AppState`, así que **estructuralmente
  no podía** invalidar la cache de proyección. Pero concilia exactamente igual que el camino HTTP,
  y conciliar cambia QUÉ cuenta en el promedio 12m (las patas conciliadas salen del numerador **y**
  del denominador). En modos B/C eso es una mutación de inputs del engine: el par recuperado movía
  la proyección y la entrada cacheada se quedaba con la cifra vieja.
- **Por qué no se cerraba solo**: el TTL de la cache es **deslizante** (D7) —
  `projection_cache_get` hace `e.last_used = Instant::now()` en cada hit—, así que un usuario que
  mire su proyección una vez por hora mantiene viva la entrada obsoleta **indefinidamente**. No era
  una ventana de 60 minutos.
- **El arreglo**: el barrido toma `Arc<AppState>` y llama a
  `invalidate_projection_if_savings_uses_transactions` por cada owner cuyo pase **crea pares**,
  igual que hace el camino HTTP desde 3.5.0. El gating por `savings_source` vive dentro del helper,
  así que en modo A sigue sin invalidar nada.
- **Condicionado a `pairs_created > 0` a propósito**: en una instalación sana el barrido no
  encuentra nada, y desalojar una cache caliente cada 24 h a cambio de nada habría sido peor que el
  bug que arregla.
- **Cuatro regresiones, verificadas con tres mutantes** (`transactions_projection_cache.rs`): B y C
  invalidan al recuperar un par, A nunca, y un barrido que visita al owner sin enlazar nada no tira
  la cache. Quitar la invalidación tumba B y C; invalidar siempre tumba el de cache caliente;
  saltarse el gating por modo tumba el de A. La primera versión del test de cache caliente **pasaba
  en vacío** —con todo conciliado el barrido no visita a nadie y el guard no se ejercita—; ahora
  deja un movimiento impar y asserta `owners_scanned == 1`.

### Fixed — `FUTUREFIN_RECONCILE_SWEEP_HOURS` faltaba en el doc de récord de env vars

- Estaba en el CHANGELOG y en dos skills, pero no en [`.claude/env-and-config.md`](.claude/env-and-config.md),
  que CLAUDE.md designa como catálogo de env vars. Añadida con su tope real: se parsea como `u64` y
  se **descarta si supera 168** (una semana), así que un valor no parseable, negativo o `>168` cae
  al default de 24 sin avisar.

### Added — Los GitHub Releases se publican solos desde el CHANGELOG

- **El desajuste**: en GitHub convivían tres listas de versiones que no coincidían. Tags había 38;
  Releases, **dos** (`v2.2.0` y `v2.3.0`, creados a mano en agosto de 2026). `publish-image.yml`
  solo construía y empujaba la imagen — no tenía ningún paso que creara el Release —, así que de
  3.0.0 en adelante toda versión se publicaba en Docker Hub y GHCR sin dejar rastro en la pestaña
  de Releases. No era una decisión: era que nada lo hacía.
- **La solución**: un paso final en `publish-image.yml`, **después** del push de la imagen (un
  Release que anuncie una versión que no llegó a publicarse es peor que no tenerlo), que redacta
  las notas con `scripts/changelog-section.sh` y llama a `gh release create`. El workflow pasa a
  `contents: write`; el checkout mantiene `persist-credentials: false`, así que el token solo
  viaja como `GH_TOKEN` a ese paso.
- **El CHANGELOG es la única fuente de las notas.** El script extrae la sección de la versión
  comparando la cabecera de forma literal (`index($0, want) == 1`), no por regex: así ni los
  puntos de la versión actúan como comodines ni `1.0.1` se traga la sección de `1.0.10`. Si la
  versión no tiene sección **falla loud** (exit 1) en vez de publicar unas notas vacías — el mismo
  criterio que las migraciones (§2.7 de `futurefin-change-control`).
- **Idempotente y acotado**: si el Release ya existe no lo toca, y solo actúa en `push` de tag —
  un `workflow_dispatch` para reconstruir una imagen antigua no reescribe notas.
- **Backfill completo**: los **38 tags** del repo tienen ya su GitHub Release, redactado con ese
  mismo script — histórico y futuro comparten formato. Los dos Releases antiguos (`v2.2.0` y
  `v2.3.0`) eran de una línea escrita a mano; pasan también a la sección completa.
- No toca la imagen: ni `scripts/` ni `.github/` entran en el build (el `.dockerignore` excluye
  `.github` y el Dockerfile nunca copia `scripts/`), así que este cambio no exige republicar.

### Fixed — Recuperadas dos secciones del CHANGELOG que se habían perdido

- `1.0.5` y `2.2.0` estaban publicadas (tag + imagen; la 2.2.0 con Release) pero **no tenían
  cabecera** en el CHANGELOG actual, y a la 2.2.0 se la citaba *dentro* de otras entradas como si
  existiera. No fue una omisión al escribirlas: sus commits de bump (`465e3d4`, `0792f9f`) sí
  tocaban `CHANGELOG.md`, y `git show v2.2.0:CHANGELOG.md` devuelve la sección entera. Se
  perdieron después, probablemente al redactar la versión siguiente. Restauradas **verbatim**
  desde git, no reconstruidas a mano.

### Added — `scripts/audit-releases.sh`: la deriva deja de ser invisible

- Compara las tres listas (secciones del CHANGELOG · tags · GitHub Releases) y las clasifica. Un
  **tag sin sección** es bloqueante (exit 1) porque rompe la publicación de notas; una **sección
  sin tag** o un **tag sin Release** son informativos.
- `--version` es el modo CI, ya cableado como primer paso del job `rust`: verifica que la versión
  de `apps/api/Cargo.toml` tiene sección en el CHANGELOG. Es exactamente el guard que habría
  cazado el agujero de la 2.2.0 el día que se abrió, en vez de tres meses después. Verificado con
  un mutante: con `version = "9.9.9"` el paso falla.

### Known — Doce versiones del CHANGELOG que nunca se publicaron

- `1.0.11`–`1.0.20`, `1.4.4` y `3.5.0` tienen sección pero **no tienen tag**, así que no pueden
  tener Release (un Release cuelga de un tag). Las diez de la serie `1.0.1x` **nunca existieron
  como versión**: `apps/api/Cargo.toml` saltó de `1.0.10` a `1.1.0` y ningún commit del repo fijó
  esos números — son numeración de CHANGELOG de una jornada de iteración rápida (todas fechadas
  2026-05-16) cuyo trabajo salió publicado dentro de la `1.1.0`. `1.4.4` y `3.5.0` sí tuvieron
  commit, pero los absorbió la versión siguiente (de la 3.5.0 ya lo dice su propia entrada).
- **No se les crea tag a posteriori**, y es deliberado: además de inventar releases que nunca
  existieron, empujar un tag `vX.Y.Z` dispara `publish-image.yml` — también en su versión antigua,
  que igualmente publica `type=raw,value=latest`. Diez builds de código de mayo de 2026
  sobrescribirían `:latest` en Docker Hub y GHCR con una imagen `1.0.x`.

## [3.8.0] - 2026-08-21

Tren de **ergonomía del servidor MCP** derivado de una sesión de uso real, más la
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

- **El hueco que cerraba esa auditoría**: no había forma de auditar la cascada desde fuera. Con la
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

- **El defecto de contrato** (etiquetado en su día como `bug`, y lo es):
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

- **De dónde viene**: aquella auditoría traía un «posible bug» de sobreasignación de la cascada.
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
  aquella auditoría.
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

## [2.2.0] - 2026-08-14

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

## [1.0.5] — 2026-05-13

### Improved
- **Projection API**: `GET /v1/projection/series` now returns `jubilacion_month_index` and `jubilacion_target_net_worth` — the FIRE milestone is computed server-side (gross-up + SWR division already run by the engine layer) instead of being duplicated in the browser.

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
