# Contratos financieros de FutureFin

Qué magnitud representa cada cifra del modelo, con qué unidad y convención, por qué esa convención
refleja la realidad española, y qué divergencias conocidas quedan pendientes — **deuda
contabilizada, no excusada**. Nace de la auditoría del modelo financiero de 2026-08-30, cuya
verificación de implementación dio **acuerdo exacto** entre el engine y un oráculo independiente
(2.131/2.131 filas, delta ≤ 3·10⁻²⁶): todo lo que se lista en §4 es divergencia de **modelo contra
realidad** o entre superficies, no error de aritmética.

## 0. Cómo se lee este documento

- Un **contrato** es una afirmación falsable: tiene número o fórmula, unidad y ancla
  `path::función`. «Calcula el interés» no es un contrato; «interés = saldo de apertura ×
  TIN/1200» sí.
- El ancla es la función, no la línea (las líneas caducan). El §6 lista un grep por contrato: si
  un grep deja de encontrar, el contrato se ha movido o retirado — **un grep vacío es la señal**.
- Las divergencias de §4 llevan su coste en euros sobre un escenario SINTÉTICO y su issue. Que una
  divergencia esté documentada aquí no la convierte en decisión de diseño: la convierte en defecto
  conocido con dueño.

## 1. Unidades y bases

| Magnitud | Unidad/base | Regla |
|---|---|---|
| Dinero | `rust_decimal::Decimal` end-to-end; strings decimales en el wire; `f64` SOLO en los arrays de series de chart (D4 del contrato de arquitectura) | El engine no redondea jamás; el redondeo es de presentación (`money_out` 4 dp, ratios 6 dp, histórico 2 dp) |
| Tipos de interés de pasivos | `apr_percent` = **TIN nominal anual** en puntos (3 = 3 %/año); tipo mensual `i = apr/1200` | Idéntico en proyección (`liability_month`) e histórico (`LoanTerms`) — la misma curva a ambos lados de «hoy». Desde 4.7.0 (#122) la UI y el schema MCP lo etiquetan **TIN** |
| Rentabilidad de activos | `expected_annual_return_percent` **nominal**, factor mensual geométrico `(1+p/100)^(1/12)` | Raíz 12ª exacta: 12 meses componen la tasa anual. Negativos componen de verdad; ≤ −100 → factor 0 |
| Inflación | `annual_inflation_assumption_percent`; factor `(1+i/100)^(m/12)` con exponente en años fraccionarios | La conversión geométrica es la correcta (la lineal x/12 sesga hasta +13,7 % a 30 años) |
| Tiempo | Meses civiles (`checked_add_months`), mes k = mes civil que empieza en `month_first_calendar(ref_date)+k−1`; interés mensual = 1/12 del año sin importar los días (30/360 español) | `month_index` es un número de MES en la rejilla, jamás una posición de array (densidad `hybrid`) |
| Nominal vs real | La simulación es TODA nominal; el target FIRE es lo único que se infla; la deflactación es capa de presentación keyed por `month_index` | Lección v1.0.12: simular deflactado dentro del engine produjo incoherencia silenciosa — camino vetado |

## 2. Contratos por magnitud (los que cargan peso)

### 2.1 Deuda
- **Devengo francés**: interés sobre saldo de **apertura**, cuota a fin de mes; `payoff = P·(1+i)`,
  `cash = min(M, payoff)`, `closing = payoff − cash` — `crates/engine/src/projection.rs::liability_month`.
  **Coincide al céntimo** con la práctica bancaria española (BdE: «capital pendiente × TIN/1200»,
  base 30/360) — verificado contra cuadro independiente 100.000 €/3 %/278 m (cuota 499,51; interés
  m1 250,00; total 38.862,97). Test: `french_two_months_hand_checked`, `french_extinction_at_month_278`.
- **Amortización negativa**: cuota < interés ⇒ el saldo CRECE y `principal_repaid` se publica
  negativo, sin clamp — correcto (así funciona una revolving mal pagada). Test:
  `french_payment_below_interest_grows_the_principal`.
- **Identidad del calendario**: `payment + extra == interest + principal_repaid` por construcción
  (el interés es residuo de saldos) — `liability_amortization_schedule`. Test:
  `schedule_payment_identity_holds_in_every_model`.
- **Catálogo honesto (4.7.0, #144)**: el default es `french` (columna + formulario; la migración
  firmada convirtió las filas fixed+TIN a `french` y anuló el TIN residual). `fixed_payments` es
  el préstamo SIN intereses y **rechaza** TIN (`apr_forbidden_for_model`); `interest_only` cobra
  el interés del período (`cash = min(M, P·i)`, el déficit capitaliza — carencia real);
  `revolving` cobra `max(min_payment_pct·saldo, min_payment_eur)`, no la cuota declarada. La
  misma regla firmada se aplica al IMPORT de backups ≤ v10 (tercer sitio del predicado).
  El párrafo pedagógico del owner (incluir tal cual donde se explique el cambio de default):
  «200 vs 278 meses» — un préstamo de 200.000 € a 1.000 €/mes que se salda en 200 meses SIN
  intereses no tiene una «cuota neta equivalente» en un préstamo francés al 3 % que tarda 278
  meses en extinguirse pagando la MISMA cuota nominal. Bajar la cuota para que el francés dure
  también 200 meses no reproduce `fixed_payments` — cambia el producto entero. Los dos números
  NO son intercambiables y el catálogo no debe sugerir que sí.
- **Degeneración que queda**: TIN ausente/≤0 en datos legacy/import ⇒ `french`/`revolving`
  colapsan a la recurrencia sin intereses y `interest_only` a caja 0 con principal congelado
  (por eso la migración 3b lo manda a `fixed_payments`); plan vencido con saldo vivo ⇒
  resta constante congelada, ahora VISIBLE y marcada (#145) — la demora no se modela (decisión
  del owner, §4 aceptadas).
- **Actividad**: `monthly_payment > 0 AND (payment_end IS NULL OR >= inicio de mes)` — predicado
  único `liability_active`; y **devengo** = modelo con intereses + TIN > 0 + plan vivo, predicado
  único `liability_interest_accrues` (#121: lo comparten `liability_month`, el `net_return` de
  `/v1/summary` y su espejo TS `liabilityAccruesInterest`). Granularidad declarada: el motor lo
  evalúa con el PRIMER día de cada mes simulado y los KPIs con «hoy» — un plan que vence a mitad
  de mes devenga ese mes en la curva pero ya no en el KPI (ventana ≤ 1 mes, no un bug).
- **Amortización anticipada (what-if, #151)**: compensación legal default 2 % del extra (cota
  [0,2], Ley 5/2019 art. 23 a tipo fijo; opt-out «0») — coste puro FUERA de la identidad del
  calendario; `reduce_payment` λ-escala la cuota (`M' = M·P'/P`); con un lump PUNTUAL el mes de
  extinción se conserva EXACTAMENTE (el plazo solo depende de `P·i/M`, pineado); con extra
  RECURRENTE la invariancia es un `≤` — el importe absoluto cancela antes cerca del final
  (verificado: 200 €/mes adelanta 239→232). Nunca alarga. Sobre `revolving` el efecto se
  RECHAZA (su caja es la cuota mínima, no la declarada). No se modela: caída al 1,5 % tras el año 10,
  topes de variable, pérdida financiera del prestamista.

### 2.2 Capital
- Crecimiento **después** de los flujos del mes (aportación cobra el mes completo);
  `values[i] = values[i].checked_mul(m)` — desbordar es error tipado `AssetValueOverflow`, nunca
  panic ni saturación silenciosa.
- Drenaje en déficit (4.12.1): el déficit ENTERO se vende — `surplus_cash` murió; su exención
  fiscal la hereda la base alimentada por la cascada (`basis_declared`, extensión de #178:
  b = v ⇒ g = 0 en el sumidero al 0 %); lo
  que falte se vende **BRUTO** (4.10.0/#140: `gross_up_monthly(neto, tramos, enabled, g)` — M1,
  dentro del bucle, en todo drenaje) sobre TODOS los activos — líquidos primero, dentro de cada
  grupo menor rentabilidad primero, desempate por índice de entrada (orden de entrada total:
  `ORDER BY sort_index, name, id`); la base de coste de cada activo baja con lo vendido (#120).
  Lo no cubierto se acumula en `undrained_cumulative` **NETO** (mide gasto que faltó, no ventas
  que no ocurrieron) y RESTA del patrimonio: la curva puede ser negativa y no se aplana —
  correcto.
- **D10 CERRADO en 4.12.1**: `surplus_cash` (caja al 0 %, invisible e ilimitada) se ELIMINÓ del
  modelo por decisión del owner («antinatural, sin espejo en la realidad — el dinero siempre vive
  en un activo»): siembra + retro-siembra + sumidero indestructible (#176) hacen que el sobrante
  siempre tenga destino; el euro sin regla queda FUERA del balance, cuantificado en
  `unallocated_savings_total` (decisión 3).

### 2.3 Caja y asignación
- Orden del mes: servicio de deuda → estado de jubilación (NW(k−1) vs target(k−1)) → caja neta →
  (drenaje | acumulación en jubilación | cascada) → crecimiento → asiento de principales → NW.
- Cascada: `fixed`/`percent` (sobre el restante del paso)/`remainder`, caps a techo absoluto sobre
  el valor VIVO del activo; conservación exacta `Σ per_asset + leftover = base_cash` (pinneada en
  `allocation_resolution.rs`). Desde 4.12.1 (#175) la cascada corre TAMBIÉN
  jubilada — la misma del usuario, con los techos de la fase (#171) gobernando euros de verdad;
  el literal `in_retirement` murió con ella.
- Modos de ahorro: A (presupuesto), B (promedio real ambos lados), C (ingreso plan + gasto real);
  fallback por lado. En B/C la cuota vive dentro del promedio (decisión explícita del owner) y el
  principal se congela — la parte «para siempre» es divergencia (§4: D17, decidida).

### 2.4 FIRE y fiscalidad
- target del mes k = `gross_up(need(k), tramos, g)/(swr/100) + término_deuda(k)` (4.10.0/#170:
  evaluado POR MES sobre la necesidad real — en `annual_expense` la pensión plana se resta
  DESPUÉS de inflar el gasto; en `manual`/`current_income` la cifra se indexa entera; helpers
  `fire_target_at_month_index` + `fire_target_base_at_month_index`; NO monótono por partida
  doble: término de deuda decreciente y, con pensión, base súper-inflada). Cruce desde 4.8.0/#143: **`líquido(k−1) ≥
  target(k−1)`** (Σ activos vendibles, bruto — sin término de caja desde 4.12.1; teorema: el
  cruce solo pudo irse MÁS TARDE con ese cambio, y en producción es invariante), con latch
  absorbente (#141).
- Gross-up: forma cerrada por tramos (escala **marginal**), tramos por defecto = escala del ahorro
  VIGENTE 2025-26 (19/21/23/27/30 @ 6k/50k/200k/300k — Ley 7/2024). Paridad Rust↔TS por
  `fire-parity.json` (recuenta los casos con `python3 -c "import json;print(len(json.load(open(
  'apps/api/tests/fixtures/fire-parity.json'))['cases']))"` — el «9» que aquí vivió congelado ya
  mordió una vez).
- **Una sola fiscalidad, dos regímenes declarados (4.12.0/#178).** La escala de tramos y el
  switch `taxes_enabled` son únicos (`crates/engine/src/tax.rs`) y los consumen los cuatro
  sitios. El **objetivo FIRE** y el **umbral SWR del runway** son PERPETUIDADES: usan el escalar
  `taxable_gain_ratio` (g, [0,1], default 1 — que no es solo prudencia: con la base cayendo
  proporcional al vender y el valor recreciendo, `ρ_k = ρ₀·m^{−k} → 0`, o sea `g → 1` es el
  LÍMITE correcto de lo que una perpetuidad dimensiona). El **drenaje del bucle** y el **bucle
  finito del runway** son TRAYECTORIAS: la `g_i` de cada activo CON coste declarado
  (`purchase_price` presente, 0 incluido) se DERIVA de su base viva — `g_i = max(0, 1 − b_i/v_i)`,
  invariante al drenaje del propio mes (teorema: `b' = b·v_post/v_pre ⇒ b'/v_post = b/v_pre`) y
  creciente con el crecimiento —; el escalar es el valor de los activos SIN coste declarado.
  Con `g` heterogénea el bruto lo resuelve la forma cerrada por tramos
  (`gross_up_mixed_monthly`: la base agregada `Σ g_i·venta_i` atraviesa los tramos progresivos —
  paseo exacto, sin iteración; la familia iterada está RETIRADA por arqueología). La dirección
  del error residual es la SEGURA: el objetivo dimensiona con g=1 mientras los primeros años del
  drenaje pagan menos ⇒ se cruza sobrecapitalizado. La respuesta declara qué rigió
  (`drawdown_gain_basis`) y la `g₀` informativa de hoy (`taxable_gain_ratio_today`).
- La ley exacta grava con **FIFO por participaciones** y diferimiento (LIRPF arts. 33/37/94); el
  modelo usa **coste medio proporcional** — lo único que la estructura de datos permite (UN
  `purchase_price` por activo, sin lotes) y lo que hace un reembolso real de fondo UCITS. La
  diferencia con FIFO es de CALENDARIO, no de importe total (la base agregada es la misma);
  divergencia aceptada en §4. Matiz declarado: `g_i` clampada a 0 descarta las minusvalías (el
  art. 49 permitiría compensarlas) — el modelo sobreestima ligeramente el impuesto con pérdidas
  latentes, mismo signo prudente de siempre.
- Fiscalidad de fondos: la rentabilidad publicada de un fondo YA es neta de TER/transacción
  (RD 1082/2012 art. 5; CNMV); los traspasos entre fondos están exentos (art. 94) — por eso «sin
  rebalanceo» es carencia funcional, no fiscal.
- **El objetivo FIRE no ve los «Próximos»** (`planning_flows`, puntuales ni recurrentes —
  4.11.0/#148): alimentan la CAJA de la proyección, no la necesidad que el target capitaliza.
  Decisión del owner en #148, explícita («no arreglarlo por coherencia»): un Próximo es un evento
  de tesorería, no gasto estructural — el gasto que define la jubilación vive en el presupuesto
  (o en el promedio real, según el modo).

### 2.5 Jubilación
- Disparador único: cruce patrimonial LÍQUIDO (el trigger por edad está vetado —
  failure-archaeology). Latch absorbente desde 4.8.0 (#141): una vez jubilado, siempre jubilado.
- Tras el cruce: `income_retirement` (partidas `persists_after_retirement`) y `expense_retirement`
  (partidas `!ends_at_retirement`), **del presupuesto en los 3 modos**; desde 4.9.0 (#139) el
  gasto se INDEXA a la inflación de la instalación y los ingresos quedan planos (decisión del
  owner); el superávit corre la MISMA cascada del usuario (4.12.1/#175): lo reinvertido sube la
  base de coste (#120) y abarata las ventas posteriores (#178); la retirada TRIBUTA
  desde 4.10.0/#140 — todo drenaje vende bruto con la MISMA escala de tramos que el objetivo, y
  desde 4.12.0/#178 con la `g` de cada activo derivada de su base real cuando el coste está
  declarado (el escalar rige perpetuidades y activos sin coste — ver §2.4).

### 2.6 Histórico
- Interpolación entre snapshots: activos lineal en días civiles (o anclada a cash-flow); pasivos
  por la **ley del modelo CAPTURADO** (#129, 4.7.0): `french`/`revolving` ⇒ curva compuesta
  «corregida por residuo» (exacta en los extremos pese a `powd`); `fixed_payments`,
  `interest_only` y `None` (snapshot pre-4.7.0) ⇒ la CUERDA — para cuota fija no es aproximación
  (pendiente constante). El modelo viaja en `history_snapshot_items.repayment_model` y en el
  `.ffbackup` v11. El mes 0 se evalúa en `today` real.
- Ausencias (#130, 4.7.0): un item ausente de una captura ARRASTRA su último valor observado
  (LOCF) — una foto incompleta no desploma el agregado. Vale cero: la ausencia del ledger vivo
  (`last_is_live_ledger`, borrado/vendido de verdad) y — matiz pineado — el punto EXACTO de la
  última fecha del timeline aunque sea una captura manual (rama `a == m−1`: una foto final que
  omite el item tampoco lo resucita EN su fecha; alcanzable con un snapshot parcial fechado hoy).
- Los snapshots JAMÁS son inputs del engine de proyección (D12 de arquitectura). El empalme con la
  proyección es solo del frontend (`history-merge.ts`): **mismo mes civil** del ancla (#130) —
  cruzar la medianoche dentro del mes fusiona; cruzar la frontera de mes es identidad (la rejilla
  se desplaza un mes entero, «±1 día» sería incorrecto ahí).
- El quiebro de pendiente en «hoy» de los pasivos de cuota fija (pasado francés, futuro lineal)
  desapareció con #129: pasado y futuro usan la misma ley.

### 2.7 KPIs
- `net_return`: expectativa (no realizado), ponderada por valor; desde 4.7.0 (#121) resta el TIN
  SOLO de los pasivos que devengan (`liability_interest_accrues`, el MISMO predicado del engine:
  modelo con intereses + TIN > 0 + plan vivo); el visible que no devenga pesa en el denominador a
  coste 0. Real por Fisher (división de factores, no resta).
- `runway`: retirada-antes-de-crecimiento, multiplicador ponderado por valor (aprox. conservadora
  del drenaje real), «indefinido» ⟺ umbral SWR sobre el saldo líquido; 1200 meses es SUELO.
  Incoherencias con la simulación en §4 (D29).
- El contrato en prosa de cada métrica vive en `apps/web/src/lib/helpTexts.ts`
  (skill `futurefin-metric-definitions`).

### 2.8 Devoluciones (4.15.0)

| Magnitud | Representación | Convención |
|---|---|---|
| **Devolución** (copago por Bizum, abono de comercio, reembolso) | fila de clase `expense` con `amount > 0` | Netea **dentro de la categoría de lo que compensa** (`actual = −Σ` firmado por categoría): un cargo de −30 y su copago de +12 dejan 18 en la misma categoría. **No hay categoría «Devoluciones»** (decisión del owner, 2026-09-02: una categoría-cajón rompe la atribución). La UI la señala con el badge «Devolución» y la comparativa publica `totals.refunds_actual` / `refunds_avg` (Σ de esos positivos, ≥ 0) como línea **derivada** — hacerla visible no cambia ningún total. |
| Devolución ↔ conciliación | nunca pata de transferencia | La candidatura automática exige signo natural en ambas patas (`expense` negativa ↔ `income` positiva, `candidates_from_where`): un +49,90 de reembolso no puede «comerse» un cargo real de −49,90. La manual (`POST /v1/transactions/{id}/reconcile`) sigue kind/sign-agnóstica. |
| Solo el importador y el restore de backup pueden crear un `expense` positivo | `assert_amount_sign_matches_kind` exime a ambos | El alta manual sigue exigiendo signo por clase; reclasificar un ingreso a gasto (PATCH de solo `kind`) también produce una devolución legítima. |

**`net_avg` ↔ «Ahorro mensual» del Resumen — homónimos con base distinta.** `totals.net_avg`
(`GET /v1/transactions/summary`) es `income_avg − expense_avg` sobre meses **reales** y siempre
movimientos; el «Ahorro mensual» del Resumen (`financial_health.net_monthly_equivalent`) sigue el modo
`savings_source` y en modo A sale del presupuesto. La tarjeta «Ahorro» de Movimientos y su texto de ayuda
lo declaran (regla de `futurefin-metric-definitions` §4: decir lo que la métrica NO es).

## 3. Lo que el modelo YA acierta — no lo «arregles»

1. **Francés español exacto**: interés = saldo apertura × TIN/1200, base 30/360, cuota fin de mes,
   última cuota de ajuste menor. Validado al céntimo contra referencia independiente.
2. **Raíz 12ª geométrica** para tasas anuales (activos e inflación). No convertir a `p/1200`.
3. **Todo nominal + target móvil**, deflactación solo en el borde de display, keyed por
   `month_index`. La simulación deflactada dentro del engine está vetada por historia (v1.0.12).
4. **Decimal sin redondeo interno**; redondeo solo de presentación.
5. **Cruce FIRE por helper único** (serie y decisión no pueden divergir).
6. **Amortización negativa sin clamp** y **NW negativo sin aplanar** (`undrained_cumulative`):
   los números feos correctos se publican.
7. **Tramos del ahorro por defecto = escala vigente**, aplicada como marginal.
8. **Retornos negativos componen**; ≤ −100 % → factor 0 (pérdida total), jamás negativo.
9. **Efectivo al 0 %**: realista para cuenta corriente española (BdE: ~0,15 % TEDR hogares).
10. **Gross-up en forma cerrada** idéntico ±céntimos a la escala marginal en TODOS los tramos
    (verificado en vivo hasta el tramo abierto del 30 %).
11. **Pensión como ingreso configurado por el usuario**: la modelización correcta (derivarla de
    cotizaciones sería falsa precisión).

## 4. Divergencias conocidas — deuda contabilizada

Estado 2026-08-30. «Decidida» = el owner eligió dirección (constan en el issue); «aceptada» = el
owner decidió no actuar (consta aquí, con fecha). Cifras de escenarios SINTÉTICOS.

**Resueltas en 4.5.0**: overflow del engine tipado (D32-motor), cascada en jubilación con
orden total de activos (D34), parsing de umbrales (D33-tramos), fixture
27/30 %, dos erratas de prosa (S1 parcial); más la Ola 1 completa (#95 #96 #97 #99 #105 #113
#135 #137 — null-que-borra, techo del cap siempre resuelto, MCP en inglés con `id`, owner-only
en la core, puertas de escritura, campos muertos).
**Resueltas en 4.6.0 (Ola 2)**: estados de fallo publicados (#119 — agotamiento, descubierto,
amortización negativa, razón del objetivo ausente, con paridad MCP), la vista Jubilación lee el
servidor y la forma cerrada TS sustituye a la bisección con el 10.º caso de paridad (#118), el
drawdown completo para el ya-jubilado (#132), prosa reconciliada + 6 contratos de métrica nuevos
+ importes declarados netos (#131 #133 #134-parcial #138-parcial #147).
**Resueltas en 4.7.0 (Ola 3 — «La deuda dice la verdad»)**: catálogo de amortización honesto —
default `french`, migración firmada, carencia y revolving reales, mínimos `min_payment_*` (#144);
etiqueta TIN donde siempre se calculó TIN (#122); vencimientos contados desde el día ancla (#123);
el plan vencido con saldo vivo visible, congelado y marcado `plan_expired_with_balance` (#145);
una sola base de coste de la deuda — `liability_interest_accrues` compartido por motor, Resumen y
Pasivos (#121); compensación por reembolso anticipado (2 % default) + «reducir cuota» con
extinción invariante en el what-if (#151); el modelo de amortización viaja al snapshot y al
`.ffbackup` v11 — la interpolación histórica usa la ley capturada (#129); el item ausente de una
captura arrastra su último valor y el empalme del chart es por mes civil (#130).
**Resueltas en 4.8.0 (Ola 4 — «El cruce, la base y la jubilación»)**: la jubilación es un estado
absorbente — una vez cruzado el objetivo (o alcanzada la edad), jubilado para siempre, sin
parpadeo mes a mes (#141); el objetivo FIRE gana el término finito de deuda — perpetuidad + TODAS
las cuotas pendientes + cola residual, decreciente al amortizar (el objetivo deja de ser monótono:
cruce por escaneo lineal), y en B/C la deuda vuelve a amortizar (opción 3 del owner: la cuota
declarada se RESTA del promedio real, una sola regla contable en los 3 modos) (#142); el cruce se
decide contra el patrimonio LÍQUIDO bruto (Σ vendibles — sin caja desde 4.12.1), emparejado
algebraicamente con el término de cuota completa del objetivo (#143); una partida de presupuesto
vencida deja de contar EN TODAS PARTES a la vez — sumatorios y `expense_end_entries` juntos, sin
caja fantasma (#124); el gasto medio real solo divide entre meses con movimientos CLASIFICADOS,
las dos «medias de N meses» comparten ancla (HOY, la de `transactions_avg`) y los euros nominales
sin deflactar quedan declarados en la ayuda (#125); `net_recurring_monthly`/`net_cash_monthly`
convergen al primer paso real del motor (`first_month_allocation`, que ya no atajea a ceros sin
activos) (#127); «Autonomía: indefinida» exige rentabilidad esperada ponderada > 0 además del
umbral SWR, y el caso finito drena secuencialmente como la simulación (#128).
**Resueltas en 4.9.0 (Ola 5 — «La inflación y el horizonte»)**: el GASTO del bucle (regular y de
jubilación) se indexa a la inflación de la instalación con el factor único sobre el eje `(k−1)/12`
— los INGRESOS quedan planos por decisión del owner («las subidas hay que pelearlas»); la
corrección del «coste medido» del issue está publicada en el propio #139 (su «mes 335» era la
alternativa rechazada de indexarlo todo: con la decisión firmada el hogar del ejemplo no cruza en
840 meses y entra en déficit el mes 247) (#139); la inflación admite negativos — rango [−2, 50],
default 2,5 % SOLO en instalaciones nuevas, y caen las 11 capas de aplanado (5 clamps, la rama
del engine, el deflactor, el gate de milestones_real, 2 regex MCP y los suelos de la SPA): con
deflación el objetivo DECRECE (t(120) = 705.667,217472 sobre 863.652,80 a −2 %) y lo real queda
por encima de lo nominal (#146); la edad límite del horizonte es configurable
(`fire_settings.horizon_lifespan_age`, 85..=105, default 90; basis `lifespan_age` +
`horizon_lifespan_age` ecoada; margen al final = último punto + `final_net_worth_real`) (#149).
**Resueltas en 4.10.0 (Ola 6 — «El impuesto que sí se paga»)**: la base de coste es POR ACTIVO y
baja al vender (`b' = b·v_post/v_pre`; `contributed = Σ basis` desde 4.12.1 — el superávit
jubilado cuenta y la serie DEJA DE SER MONÓTONA) (#120); la retirada simulada TRIBUTA — todo
drenaje de activos vende bruto (`gross_up_monthly`, M1, dentro del bucle; la caja no se grossea;
`undrained` pasa a NETO; el pin de #119 con tramos ES: mes 100/−520.000 → mes 80/−561.200) y la
fracción de plusvalía gravable es configurable (`taxable_gain_ratio`, [0,1], default 1 — misma g
en objetivo, drenaje y los DOS umbrales del runway, cuyo bucle finito también vende bruto desde
esta ola: baseline 10 → 8,0 meses con tramos ES) (#140); el objetivo se evalúa MES A MES sobre la
necesidad real — `gross_up(need(k))/SWR + término_deuda(k)`, con la pensión plana restada DESPUÉS
de inflar (caso central: target(240) 509.467,68 → 676.078,21, el Δ son los 166.610,54 del issue) y
el fiscal drag capturado también sin pensión (+7.140,43 € a 30 años: los tramos son nominales)
(#170); la traza `InRetirement` resuelve los techos con el presupuesto de jubilación — dos
escalares × dos ramas (#171).

**Resueltas en 4.11.0 (Ola 7 — «Próximos con fecha y el sobrante que trabaja»)**: el Próximo
vencido carga íntegro en el mes ancla, declarado (`overdue` en `events[]`) en vez de desaparecer
— los 3 k€ del escenario del issue vuelven a la caja y recuperan 3.000 × 1,05²⁰ = **7.959,89 €**
a 20 años —, la rampa sin fecha se ancla al día 1 del mes civil (el reparto es idéntico todos los
días del mes; antes el mes 0 oscilaba 300 € — un 30 % de una aportación tipo) y el baseline de
hitos deriva del mismo mapeo (#126); «Próximos» habla flujos recurrentes con ventana
(`amount_basis = per_month`, €/MES en `[window_start_date, window_end_date]`) — el alquiler con
contrato a 36 meses deja de cobrarse los 444 meses de más (480 − 36 = 444 × 800 =
**355.200,00 €** de renta inexistente), los `upcoming_*` de portada dejan de mezclar € con €/mes
y `.ffbackup` sube a 12 (#148; la cifra «607 k€ de pensión anticipada» que aquí vivió se RETIRÓ:
no era derivable de ninguna construcción declarada); el primer activo de un scope virgen siembra
la regla `remainder` por la misma función que la valida, la respuesta lo declara
(`seeded_allocation_rule_id`) y la resolución publicaba `surplus_destination` (retirado en 4.12.1 junto a la caja —
`unallocated_savings_reason` lo sustituye) — el escenario 1 del
issue pasa de 108.000,00 € muertos a **147.622,45 €** (+39.622,45; el issue decía 147.378 en
convención pospagable — el motor es prepagable, C1 del spike; la cifra «~1,22 M€» que aquí vivió
exigía un 7,2 % nunca declarado y se retiró) (#150). **Alcance declarado de #150**: el escenario 2
(jubilado) NO se entrega — en jubilación la cascada no corre y el superávit sigue en caja al 0 %;
issue [#175](https://github.com/maxlainz/FutureFin/issues/175) con sus 229.348,92 €. La guarda
dura contra borrar el activo del sumidero es
[#176](https://github.com/maxlainz/FutureFin/issues/176). Y las siete magnitudes duplicadas en TS
quedan disposicionadas (#136): dos ya habían muerto en la Ola 2 (`findFirstMonthNetWorthAtLeast…`,
`jubPos`), dos estaban cerradas con fixture (gross-up de la vista previa — 17 casos —, principal
derivado — 6), el deflactor del chart pasa a CONSUMIR `net_worth_real` y
`deflation_annual_inflation_percent` del servidor en la línea principal (+ fixture cruzado
`deflator-parity.json` para k ≥ 0), el interés mensual aproximado gana su fixture
(`liability-interest-parity.json` sobre el predicado compartido #121), y la línea «aportado» SALE
del modo euros de hoy (su cifra correcta —cada aportación deflactada por su mes: 135.606,13 € en
el escenario del issue— no es computable desde la serie servida, y la aproximación de un solo
factor daba 99.372,76 €, un 26,72 % corta; el servidor rechaza publicarla a propósito).

**Resueltas en 4.12.0 (#178 + retro-siembra)**: la fracción de plusvalía gravable del DRENAJE se
DERIVA de la base de coste real por activo cuando el coste está declarado — `g_i = 1 − b_i/v_i`,
viva mes a mes, con la forma cerrada por tramos `gross_up_mixed_monthly` para la mezcla (§2.4:
«una sola fiscalidad, dos regímenes»). El ancla del issue (500 k€ al 80 % de coste, 5 %, 24 k€
netos/año): agotamiento **mes 403 → 561** (+13,2 años que el default robaba) — y el escalar 0,2
estático que la ayuda antigua invitaba a poner daba **mes 916** (29,6 años de optimismo: era una
trampa publicada, no una mejora de precisión; la ayuda quedó reescrita). Bit-identidad
garantizada por construcción: sin ningún coste declarado, la vía rápida es el camino LITERAL de
4.11.0 — cero pins movidos. El espejo TS muerto `taxOnGrossCapitalAnnual` (cero llamantes, sin
fixture) se retiró. **Y la RETRO-SIEMBRA del sumidero** (orden del owner 2026-08-31, que
REVIERTE el «sin retro-siembra» de 4.11.0): migración `20260901150000` — todo scope con activos
y sin regla `remainder` sin tope la gana, apuntando al LÍQUIDO de menor rentabilidad esperada
(empate: mayor saldo; sin `created_at` en assets, «el primer activo creado» no es recuperable) —
y la misma regla corre al importar un backup pre-siembra (import.rs, cross-referenciado). El
`surplus_cash` residual quedó reducido a: déficits (primera fuente, sin grossear — teorema
`b = v ⇒ g = 0`) y el superávit del JUBILADO
([#175](https://github.com/maxlainz/FutureFin/issues/175), decisión de modelo pendiente).

**Resueltas en 4.12.1 (fin de `surplus_cash` — #175 y #176, entrevista de decisiones del owner
2026-08-31)**: la caja fantasma se ELIMINA del modelo («antinatural, sin espejo en la realidad —
el dinero siempre vive en un activo»). (1) La MISMA cascada del usuario corre también jubilada
(#175): el superávit de pensión compone — el ancla del issue, derivada del bucle real y pineada
en el engine: 500 €/mes al 5 % durante 360 meses = **409.348,92 €** donde antes morían
180.000,00 € en caja (Δ = +229.348,92, la cifra exacta del issue, entregada); lo reinvertido ES
base de coste (#120) y abarata las ventas posteriores (#178). (2) El sumidero es INDESTRUCTIBLE
con activos vivos (#176): borrar su activo quedando otros, deshabilitarlo o degradarlo → 400
`remainder_required` (el último activo del scope sí se borra); migración
`20260901160000` reactiva los sumideros apagados (sin ella el upgrade haría desaparecer dinero
en esos scopes) + espejo en el import de backups. (3) El euro sin destino NO se simula (decisión
3): fuera del balance, cuantificado en `unallocated_savings_total` + razón
(`no_assets`|`no_sink`) — inalcanzable en producción con activos vivos. (4) Identidades nuevas:
`NW = Σ activos − pasivos − descubierto`, `aportado = Σ bases`, `líquido = Σ líquidos`; el
escalón «caja primero» del déficit murió y su exención fiscal la hereda la extensión
`basis_declared` de #178 (la base alimentada por la cascada ES dato: un descubierto de 3.000 €
habría tributado 784,81 € inventados sin ella). Breaking §5: mueren `leftover_to_surplus_cash`
(→ `leftover_unallocated`), `surplus_destination` (→ `unallocated_savings_reason`) y el
`skipped_reason: in_retirement`. El pin del escenario A subió a 676.315,04 (+23.044,82): el
drenaje post-cruce ya solo tributa la ganancia real de la base que la cascada construyó.

### Aceptadas por el owner (2026-08-30) — sin issue, deuda declarada aquí

| Divergencia | Coste (sintético) | Razón de aceptación |
|---|---|---|
| Traspasos no conciliados cuentan como gasto (D23) | ~713 k€ en el peor escenario | La calidad del promedio depende de conciliar; no se inventa clasificación |
| Sin rebalanceo (D12) | 1,93 M€ vs 1,15 M€ a 30 a (deriva de pesos) | Buy&hold deliberado; sin coste fiscal en España (traspasos exentos) |
| Modo `current_income` incluye el ahorro en el objetivo (D37) | +52,6 % de objetivo | Útil para quien no ahorra mes a mes; conservador a sabiendas |
| Regla de millares en campos % («7.125» = 7125 %) (D33-%) | proyección rechazada con 400 tipado (tras 4.5.0) | Trampa documentada; el 400 tipado de 4.5.0 la hace ruidosa |
| Descubierto/`undrained` al 0 % (parte de D9) | agujero subestimado ~220 k€ al 18-20 % TEDR | El agujero se publica (issue #119); su coste financiero no se modela |
| Duplicados cliente↔servidor que QUEDAN, todos con fixture cruzado (#136, 4.11.0): gross-up de la vista previa (`fire-parity.json`), principal derivado (`liability-derived-principal-parity.json`), deflactor TS para k < 0 y mes fraccionario (`deflator-parity.json` pina el dominio compartido k ≥ 0), interés mensual aprox. (`liability-interest-parity.json`) | 0 € mientras los fixtures estén verdes — una suite roja a solas = deriva detectada | Vista previa sin round-trip posible; el `deflator_at_month_index` u32 del servidor no puede servir el pasado ni el grid fino; no existe campo de hogar para el interés aprox. |
| Coste medio proporcional en vez de FIFO por participaciones (4.12.0/#178) | Diferencia de CALENDARIO, no de importe total (misma base agregada); FIFO grava más al principio y menos después | La BD lleva UN `purchase_price` por activo, sin lotes; el coste medio es además lo que hace un reembolso real de fondo UCITS |
| Minusvalías sin compensar (`g_i` clampada a 0; el art. 49 LIRPF permitiría compensar) | Impuesto ligeramente sobreestimado con pérdidas latentes | Mismo signo prudente que el resto del modelo; compensar exigiría estado fiscal anual |
| Estacionalidad del presupuesto alisada a doceavas (D25) | 0 € al horizonte; sin señal de tesorería | Presupuesto mensual por diseño |

## 5. Convenciones españolas de referencia (fuentes)

- Liquidación de préstamo francés: interés = capital pendiente × TIN/1200, base 30/360 (BdE
  Cliente Bancario, simuladores; DGRN 21-6-2019 sobre 365/360). TIN ≠ TAE (Circular 5/2012;
  Ley 16/2011 Anexo I: TAE = (1+TIN/12)^12−1 sin comisiones).
- Revolving: cuota mínima % del saldo con mínimo en €, TEDR medio ~18,3-18,5 % (BdE tabla 19.4),
  capitalización del interés no cubierto; usura ≈ TEDR+6 pp (STS 258/2023).
- Carencia: la cuota ES el interés del período (saldo × TIN/12); cuota ≠ interés no existe como
  producto (BdE, Código de Buenas Prácticas).
- Vencimiento con saldo vivo: siempre devenga (demora rem+2/+3 pp — Ley 5/2019 art. 25; interés
  legal 3,25 % 2025-26); el saldo congelado sin devengo no existe.
- Fondos UCITS: diferimiento hasta reembolso; solo tributa la plusvalía (FIFO, arts. 33/34/37/94
  LIRPF); traspasos exentos (fondos sin requisitos; ETF excluidos desde 2022); retención 19 % a
  cuenta SOBRE LA GANANCIA (Rgto. arts. 96-97). Rentabilidad publicada = neta de TER
  (RD 1082/2012 art. 5; CNMV).
- Escala del ahorro 2025-26: 19/21/23/27/30 @ 6.000/50.000/200.000/300.000 (Ley 7/2024 DF 7ª,
  arts. 66.1+76 LIRPF). Las CCAA no pueden modificarla (Ley 22/2009 art. 46.2.a).
- IPC: índice mensual del INE (base 2025 desde ene-2026); conversión anual→mensual geométrica;
  medias anuales NEGATIVAS en 2009/2014/2015/2016/2020; dic/dic ≠ media anual (2016: signo
  contrario).
- Pensiones: se revalorizan por ley con el IPC medio dic→nov (art. 58 LGSS; 2026: +2,7 %) y tienen
  suelo nominal.
- Cuenta corriente hogares: ~0,15 % TEDR (BdE tabla 19.7, 2024-26).

## 6. Provenance and maintenance

Escrito 2026-08-30 (auditoría del modelo financiero; rama `audit/modelo-financiero`). El arnés de
verificación es permanente: `crates/engine/tests/audit_dump.rs` vuelca las series de la batería de
casos límite (`cargo test -p futurefin-engine --test audit_dump -- --nocapture`), comparables con
un oráculo externo. Re-verificación (un comando por contrato; si un grep no devuelve nada, el
ancla se movió — actualiza esta ficha en el mismo cambio):

- Devengo francés: `grep -rn "payoff = P·(1 + i)" crates/engine/src/ || grep -n "fn liability_month_g" crates/engine/src/sim_core.rs`
- Convención TIN/1200 compartida: `grep -rn "1200" crates/engine/src/{projection,history}.rs | grep -c "apr"` (≥2)
- Raíz 12ª: `grep -n "fn monthly_multiplier" crates/engine/src/projection.rs`
- Target móvil único: `grep -rn "fn fire_target_at_month_index" crates/engine/src/projection.rs` y sus ≥2 llamantes en `apps/api/src/handlers/projection.rs`
- Overflow tipado: `grep -n "AssetValueOverflow" crates/engine/src/projection.rs` (enum + checked_mul + test)
- Cascada también en jubilación (4.12.1): `grep -n "la MISMA cascada" crates/engine/src/projection.rs` y `grep -n "unallocated_savings_total" crates/engine/src/projection.rs` (≥1 y ≥2 hits respectivamente)
- Orden total de activos: `grep -rn "sort_index ASC, name ASC, id ASC" apps/api/src/handlers/` (2 hits)
- Paridad tramos altos: `grep -c "tramo" apps/api/tests/fixtures/fire-parity.json` (≥2) y `python3 -c "import json;print(len(json.load(open('apps/api/tests/fixtures/fire-parity.json'))['cases']))"` (≥9)
- Tramos vigentes por defecto: `grep -n "300000" apps/api/src/handlers/installation.rs`
- Freezer f64: `cargo test -p futurefin-engine no_f64 -- --list`
- Predicado único de devengo (#121): `grep -n "pub fn liability_interest_accrues" crates/engine/src/projection.rs` y su espejo `grep -n "liabilityAccruesInterest" apps/web/src/lib/ledger.ts`
- Ley por modelo en el histórico (#129): `grep -n "repayment_model" crates/engine/src/history.rs | head -3`
- LOCF del histórico (#130): `grep -n "last_is_live_ledger" crates/engine/src/history.rs apps/api/src/handlers/history.rs | head -3`
- Comisión de amortización (#151): `grep -n "early_repayment_fee" crates/engine/src/projection.rs | head -3`
- La tabla de §4: cada fila con estado «pendiente» debe tener issue ABIERTO (`gh issue view <n>`); si el issue se cierra, la fila se actualiza o se borra en el mismo cambio.
