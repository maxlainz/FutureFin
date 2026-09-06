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
| Dinero | `rust_decimal::Decimal` end-to-end; strings decimales en el wire; `f64` en **dos** sitios sancionados y solo dos: los arrays de series de chart (D4 del contrato de arquitectura) y el crate `crates/engine-stochastic` (5.0.0), del que **no sale un euro** — solo magnitudes estadísticas | El engine no redondea jamás; el redondeo es de presentación (`money_out` 4 dp, ratios 6 dp, histórico 2 dp). El freezer `crates_engine_src_has_no_f64_outside_comments` de `crates/engine` sigue **sin excepciones** |
| Tipos de interés de pasivos | `apr_percent` = **TIN nominal anual** en puntos (3 = 3 %/año); tipo mensual `i = apr/1200` | Idéntico en proyección (`liability_month`) e histórico (`LoanTerms`) — la misma curva a ambos lados de «hoy». Desde 4.7.0 (#122) la UI y el schema MCP lo etiquetan **TIN** |
| Rentabilidad de activos | `expected_annual_return_percent` **nominal** y **COMPUESTA (CAGR)**, factor mensual geométrico `(1+p/100)^(1/12)` | Raíz 12ª exacta: 12 meses componen la tasa anual. Negativos componen de verdad; ≤ −100 → factor 0. **Es la tasa GEOMÉTRICA** —la que publican los fondos y la que el hogar cobra—, no la media aritmética de los retornos (decisión M8 del modelo v2, owner 2026-09-06). El camino `Decimal` no sabe de volatilidad y la compone tal cual; **quien convierte es el Monte Carlo**, que sube la deriva del factor mensual a `m·exp(σ_m²/2)` con la σ **MENSUAL** para que la MEDIANA del factor sorteado sea `m` — una sola implementación, `PathEngine::new` en `crates/engine-stochastic/src/mc.rs`. Consecuencia publicada: **la línea determinista es la MEDIANA de los caminos, no su media**. Los valores ya guardados se **reinterpretan como CAGR sin convertir** (decisión C6, owner 2026-09-06): no hay migración ni recálculo, solo un aviso en Activos y la ayuda reescrita |
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
  panic ni saturación silenciosa. Desde 5.0.0 (WP5.5) el bucle vive en
  `crates/engine/src/sim_core.rs::simulate`, genérico sobre `MoneyOps`; `projection.rs` conserva los
  tipos públicos y envuelve. El factor mensual por activo se calcula **una vez** (loop-invariante,
  WP1a: 31,5 → 12,6 ms por proyección de 840 meses en release) con la MISMA llamada a `powd`, y el
  pin dorado lo comprueba bit a bit.
- **Base de coste al vender** (#120): `b' = b·v_post/v_pre` — `checked_mul` con reordenamiento a
  `b·(v_post/v_pre)` **solo** cuando el producto no cabe (issue #209: un activo en el techo de
  `NUMERIC(18,4)` componiendo al 20 % desbordaba `Decimal` y panicaba). El orden natural
  multiplica antes de dividir porque drenar el activo entero deja la base en 0 EXACTO, y ese orden
  es el que 4.15.0 pineó: la forma reordenada no se ejecuta en ninguna entrada que hoy funciona.
- Drenaje en déficit (4.12.1): el déficit ENTERO se vende — `surplus_cash` murió; su exención
  fiscal la hereda la base alimentada por la cascada (`basis_declared`, extensión de #178:
  b = v ⇒ g = 0 en el sumidero al 0 %); lo
  que falte se vende **BRUTO** (4.10.0/#140: `gross_up_monthly(neto, tramos, enabled, g)` — M1,
  dentro del bucle, en todo drenaje) sobre TODOS los activos — líquidos primero, dentro de cada
  grupo menor rentabilidad primero, desempate por índice de entrada (orden de entrada total:
  `ORDER BY sort_index, name, id`; implementación única `sim_core::drain_order_g`); la base de
  coste de cada activo baja con lo vendido (#120). Desde 5.0.0 quien ejecuta la venta del mes es
  `sim_core::execute_month_sale_g`, que además reparte las **tres magnitudes** de §2.5.
  Lo no cubierto se acumula en `undrained_cumulative` **NETO** (mide gasto que faltó, no ventas
  que no ocurrieron) y RESTA del patrimonio: la curva puede ser negativa y no se aplana —
  correcto.
- **Bit-identidad con 4.15.0, restaurada fuera del golden** (pase de correcciones de la revisión
  D20, hallazgos F1/F2): `undrained_cumulative` tiene que ACUMULARSE con el operando LITERAL que
  publica el paseo de venta (`dd.net_shortfall_monthly`), no re-derivarse como `need − (need − s)`
  — algebraicamente igual, pero cambia la ESCALA del `Decimal` (`"0"` vs `"0.00"`) y movía el
  28.º dígito, y el `Display` es lo que el pin dorado hashea. `debt_service` tiene que sumarse con
  la MISMA agrupación de 4.15.0 (`acc + ((cash + extra) + fee)`); reagrupar a
  `((acc + cash) + extra) + fee` redondea distinto en el dígito 28 con dos pasivos y la diferencia
  se propaga mes a mes en el drenaje. Pines `P24_undrained_scale` / `P25_debt_service_assoc`
  (`crates/engine/tests/golden_pins.rs`). **Un golden de 19 casos no demuestra bit-identidad**: la
  regresión de escala solo se veía en 438 de 3.000 entradas de un fuzz DIFERENCIAL contra el motor
  de `main` (hogares aleatorios, mismas entradas por los dos motores); la campaña completa de fuzz
  diferencial bajó las divergencias de 536/496/496 a 24/21/27 por 3.000 entradas en las tres
  semillas, y las que quedan son todas «el motor viejo entraba en pánico» (desbordamientos que
  4.15.0 no tipaba), no desacuerdos numéricos. Lección para `futurefin-failure-archaeology`: un
  fuzz diferencial contra el motor anterior encuentra lo que un golden pequeño no puede, porque
  compara la MISMA entrada por los dos caminos en vez de fijar unas pocas por adelantado.
- **D10 CERRADO en 4.12.1**: `surplus_cash` (caja al 0 %, invisible e ilimitada) se ELIMINÓ del
  modelo por decisión del owner («antinatural, sin espejo en la realidad — el dinero siempre vive
  en un activo»): siembra + retro-siembra + sumidero indestructible (#176) hacen que el sobrante
  siempre tenga destino; el euro sin regla queda FUERA del balance, cuantificado en
  `unallocated_savings_total` (decisión 3).

### 2.3 Caja y asignación
- Orden del mes (`sim_core::simulate`, invariante desde 4.2.0 y **reordenado en 5.0.0 sin mover un
  dígito**): servicio de deuda → **transición de fase** (cruce `líquido(k−1) ≥ target(k−1)` o mes
  forzado, §2.5) → caja neta (ingreso de la fase, gasto indexado, **pensión con fecha**, ajustes de
  Próximos) → **cascada del sobrante** → **venta del mes** → crecimiento → asiento de principales →
  series. **La venta ya no vive en un `else` de la cascada**: hasta 4.15.0 las dos ramas eran
  excluyentes, así que bajarla después del reparto no cambia ningún caso de 4.15.0 — quien necesita
  ese orden es `rule_is_spend` (§2.5), donde se invierte primero y se vende después.
- Cascada: `fixed`/`percent` (sobre el restante del paso)/`remainder`, caps a techo absoluto sobre
  el valor VIVO del activo; conservación exacta `Σ per_asset + leftover = base_cash` (pinneada en
  `allocation_resolution.rs`). Desde 4.12.1 (#175) la cascada corre TAMBIÉN
  jubilada — la misma del usuario, con los techos de la fase (#171) gobernando euros de verdad;
  el literal `in_retirement` murió con ella.
- Modos de ahorro: A (presupuesto), B (promedio real ambos lados), C (ingreso plan + gasto real);
  fallback por lado. En B/C la cuota vive dentro del promedio (decisión explícita del owner) y el
  principal se congela — la parte «para siempre» es divergencia (§4: D17, decidida).
- **El colchón de caja se retiró en 5.0.0 antes de publicarse**: la caja es un activo más y las
  reglas de ahorro fijan cuánto se guarda.

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
- **El número FIRE clásico (5.0.0 E4)** — `crates/engine/src/target.rs::PlanFireTarget::at` y
  `fire_target_at_month_index_with_plan`. **UNA sola base y ningún trigger**: el objetivo es una
  LECTURA informativa («25× tu gasto», el número que la literatura FIRE publica) y la fecha de
  jubilación la decide el umbral de éxito sobre miles de caminos (§2.5, `crates/engine-stochastic`).

  ```text
  T(i) = gross_up(12·max(0, E·f(i) − I_persist)) / (SWR/100) + deuda(i)
  ```

  con la rejilla **0-based** (`i = k−1`: el bucle evalúa su mes `k` contra ese índice), `E·f(i)` el
  gasto de jubilación indexado, `I_persist` el ingreso PLANO que persiste (la pensión SIN fecha de
  4.15.0, dentro de `FireNeed`) y `deuda(i)` el término finito de #142. Es EXACTAMENTE
  `fire_target_at_month_index` —el objetivo de 4.15.0 que `pins-4.15.json` hashea—: `PlanFireTarget`
  llama a la misma función del núcleo (`sim_core::fire_target_at_index_g`) en vez de reproducir su
  fórmula, así que la bit-identidad es por construcción y no por revisión.

  - **NO resta la pensión CON FECHA** (decisión M4 del modelo v2, owner 2026-09-06). La pensión es
    un FLUJO que el bucle cobra mes a mes, no un descuento sobre un stock. La base que sí la restaba
    desde `P` tenía un **acantilado de construcción**: con una pensión que cubriera el gasto entero,
    `need_net(i) ≤ 0` dejaba `T(i) = deuda(i)` —0 € sin deuda— y el objetivo se desplomaba de
    600.000 € a 0 € entre dos meses consecutivos. Medido en la batería: P19 cruzaba en el mes **121**
    (el mes siguiente al de la pensión) y hoy cruza en el **306**, cuando la acumulación llega de
    verdad.
  - **RETIRADOS en E4** (no reintroducir sin releer esta fila): `TargetBasis` y su base
    `bridge_to_pension`, `bridge_discount_annual_pct` y su tabla sufijo `O(P)`,
    `MAX_BRIDGE_MONTHS = 1.200` (con su violación de contrato LATENTE: la degradación más allá de
    los 1.200 meses podía bajar el objetivo un 77 %), `EngineError::BridgeDiscountOverflow`, y las
    tres lecturas derivadas `bridge_effective_withdrawal_pct`, `pension_coverage_ratio` y
    `partial_gap_target`. El **puente** sigue existiendo, pero como lo que la corrección C2 dice que
    es: un TOPE DE TASA INICIAL con fecha límite (`BridgeCap`, §2.5), no una forma de dimensionar un
    objetivo.
  - **El cruce sigue vivo como lectura**: `líquido(k−1) ≥ T(k−1)` se anota en
    `liquid_crossing_month_index` y sigue siendo el default de `PhasePlan::classic` (P1–P13 de
    `pins-4.15.json` lo hashean), pero es una lectura, no la fecha que la app publica.

- **Capital necesario (5.0.0 E7)** — `crates/engine-stochastic/src/needed_capital.rs`
  (`needed_capital_today`, `needed_liquid_at_month`, `needed_capital_curve`). **Es EL número del
  plan**, y sustituye al objetivo FIRE en ese papel: la cifra que Jubilación, Resumen y Proyección
  enseñan como «cuánto necesitas» ya no sale de una fórmula de perpetuidad sino de **ejecutar el
  motor entero miles de veces**. El número FIRE clásico (25× el gasto, la fila de arriba) sobrevive
  como escalar INFORMATIVO (`fire_number_classic_today`) y como pin de `fire-parity.json`, nunca
  como el objetivo que decide nada.

  ```text
    λ*                    = mín{λ : éxito(escalar_líquido(λ), k) ≥ umbral}
    capital_necesario(k)  = líquido al cierre de k−1 de la trayectoria del hogar ESCALADO por λ*
    capital necesario HOY = capital_necesario(1)     (M9; en k = 1 coincide con λ*·L(0), el estado inicial)
  ```

  - **Qué magnitud es**: el patrimonio **LÍQUIDO** (Σ activos con `is_liquid`, #143 — la vivienda
    NO cuenta y NO se escala), en euros, publicado en dos bases: `amount_nominal` (euros del mes
    `k−1`) y `amount_today` (deflactado con `(1 + π/100)^((k−1)/12)`, el factor del motor). Las dos
    **redondeadas a cientos HACIA ARRIBA**: a la baja quedarían por debajo del umbral que prometen.
  - **Con qué convención**: `λ` escala **valor Y base de coste** de cada activo líquido. Escalar
    solo el valor subiría la `g_i = 1 − b_i/v_i` de la fila de fiscalidad de arriba y cobraría un
    impuesto FANTASMA sobre una plusvalía inventada; con las dos, `g_i` es invariante exacta y el
    neto de una liquidación escala exactamente por `λ`.
  - **Es una ESTIMACIÓN muestral, y viaja rotulada** (D4 enmendado): el error que lleva es el del
    sorteo, no el del tipo numérico, y por eso viene con su medición al lado (`success_at_lambda`:
    `N`, fallos por motivo y cota de Wilson) y con `capital_is_approximate` cuando la confirmación
    no cerró. La contabilidad del hogar sigue saliendo del camino `Decimal`. **La condición nueva
    de D4 (panel adversarial, 2026-09-06)**: cuando el sorteo fija el mes de un hito —una fecha
    válida, un capital necesario, una aportación mínima—, la SEMILLA y el número de CAMINOS son
    parte de la identidad del resultado, no un detalle interno: viajan en la respuesta (`seed`,
    `paths_used`) y en la clave de cache del plan (`PlanKey`, `handlers/retirement_solver.rs`),
    porque dos sorteos con distinta semilla o distintos caminos son, literalmente, dos mediciones
    distintas del mismo plan.
  - **Nunca 0 €**: sin activos líquidos (o con `L_det(k−1) = 0`) se publica
    `absent_reason: no_liquid_assets`; si ni multiplicando la cartera por 4.096 se cumple el umbral,
    `threshold_unreachable`. Un 0 € se leería como «no necesitas nada», la respuesta contraria.
  - **No es `λ*·L_det(k−1)`**: el importe se lee de la trayectoria del hogar ESCALADO
    (`project_net_worth_series(retiring_at(scale_liquid_assets(input, λ*), k)).liquid_worth[k−1]`),
    a costa de una proyección `Decimal` por cifra. El producto solo coincide en `k = 1`, y en un
    hogar cuyo camino actual se agota antes del horizonte valdría 0 € y borraría los nodos tardíos
    de la curva.
  - **Divergencias declaradas** (no son bugs; están escritas en el doc del módulo): (a) escalar
    supone reparto **proporcional** de la cascada, y con un tope por importe un `λ` mayor lo llena
    antes y desvía las aportaciones a otro destino; (b) lo garantizado es un `λ` **verificado** que
    cumple, no el mínimo demostrable.

### 2.5 Jubilación — motor por FASES (5.0.0)

Desde 5.0.0 la jubilación deja de ser un evento del hogar y pasa a ser una **estrategia por usuario**
(`users.retirement_profile`) que decide cuatro cosas a la vez: el disparador, la base del objetivo,
las fases y la regla de retirada. El motor las ejecuta como un `PhasePlan`
(`crates/engine/src/phases.rs`), consumido por el bucle (`sim_core::simulate`) y por
`first_month_allocation` — que hasta 4.15.0 duplicaban el mismo `if` con dos redacciones distintas.

**Fases**, latch **monótono** `Accumulating → (Partial) → Retired` (#141 generalizado): una vez
avanzada no se vuelve atrás, ni porque el patrimonio caiga un mes por debajo del objetivo inflado.

**Las cuatro estrategias** (§3 del modelo v2, owner 2026-09-05/06; corrección C7 retiró la quinta;
quien traduce el perfil a `PhasePlan` es `apps/api/src/handlers/projection.rs::resolve_forced_month`,
no el motor):

> **E4 (modelo v2) vació la columna «Objetivo».** El objetivo dejó de ser un eje de la estrategia:
> hay UNA sola base —el número FIRE clásico de §2.4, una lectura informativa— y ningún trigger
> cuelga de ella. Las lecturas tachadas abajo ya no existen en el motor; sus preguntas las responde
> el solve estocástico (`crates/engine-stochastic`) contra el umbral de éxito. La columna «Trigger»
> ya no distingue cruce de edad: **las cuatro llegan al motor como `AtMonth`** (mes forzado), la
> diferencia es SOLO quién decide ese mes.

| Estrategia | Cómo se resuelve el mes forzado | Aportación simulada | Lecturas propias |
|---|---|---|---|
| `asap` | `solve_mc::valid_retirement_month` — el umbral de éxito busca el mes (`ForcedMonth::NeedsSolve`) | toda la cascada | fecha válida, éxito, capital necesario hoy, fechas al 100/90 |
| `retire_at_age` | la edad declarada, `ForcedMonth::Known(R)` — **sin sorteo**, el sorteo solo mide si `R` cumple | toda la cascada | veredicto (`SuccessAt` en `R`), aportación mínima (`minimum_extra_contribution`) |
| `coast` | modo A: `Known(R)` (edad fija) + `coast_stop_month(R)` resuelve `C`; modo B: `NeedsSolve` con `contributions_stop_month = C` fijo | toda la cascada hasta `C`, cero después (S4: liberado = disponible, no se reinvierte) | mes coast (`C`, el PRIMER que cumple, C8), ahorro liberado |
| `partial` | total: `Known(R)` si hay edad, si no `NeedsSolve`; la fase entra en `AtMonth(S)` (modo A: edad fija; modo B: `earliest_partial_start`) | toda la cascada | inicio de la fase (`S`), `partial_phase_capital_growing` |

**`pension_bridge` se retiró como estrategia (C7)**: no tiene fila propia — es el interruptor
`pension.bridge_enabled` de la tarjeta Pensión, disponible en las cuatro de arriba y apagado por
defecto. Un perfil guardado con la estrategia `pension_bridge` se acepta como ALIAS de
deserialización y migra a `asap` + `bridge_enabled: true` (5 %/7 años si faltan), con el aviso
`strategy_pension_bridge_migrated`.

- **Un solo trigger por simulación (D17) — y desde el modelo v2 es SIEMPRE el mes forzado, en las
  CUATRO estrategias.** El bucle conserva la UNIÓN de 4.15.0 (`cruce || k ≥ mes forzado`) porque es
  lo que el pin dorado tiene fotografiado, pero el ensamblado pone
  `phase_plan.crossing_is_reading_only = true` **incondicionalmente**
  (`handlers/projection.rs:2531` — ya no depende de si la estrategia es por edad): el cruce
  **nunca** jubila a nadie, solo se anota como `liquid_crossing_month_index`. Lo que antes leía
  esta fila («las estrategias por edad siguen necesitando el objetivo») describía el ensamblado
  ANTES de E4/A4: hoy el objetivo entra siempre como lectura informativa (el número FIRE clásico,
  §2.4), pero de eso no depende ninguna estrategia — `retire_at_age`/`coast` fijan su mes por la
  edad (`ForcedMonth::Known`), no por comparar con un objetivo. El wire ya no publica
  `retirement_trigger: liquid_crossing|target_age`: publica
  `retirement_date_basis: success_threshold|target_age|not_reachable|pending`
  (`retirement_solver.rs::DATE_BASIS_*`).
- `retirement_month_index` es el mes **EFECTIVO** (1-based) y es lo que la API publica como
  `jubilacion_month_index` (R8); `liquid_crossing_month_index` es el cruce puro, evaluado TODOS los
  meses —también después de que el latch cierre— y **no gobierna nada**.
- **La edad manda** (D17): en `retire_at_age`/`coast` el hogar se jubila en `R` aunque el capital no
  llegue. Hasta E1 de 5.0.0 el motor lo etiquetaba con `EngineWarning::RetireAtAgeUnderfunded`
  comparando `L(R−1)` con el objetivo de perpetuidad; **ese aviso se retiró**: un booleano contra un
  objetivo descontado no distingue «no llegas por poco» de «no llegas jamás». Hoy la misma pregunta
  se responde con `1 − éxito(R)` —la proporción de caminos que fallan jubilándose en `R`— y, sobre
  la línea determinista, con el veredicto del camino (abajo).

**Ingreso y gasto por fase** (pasos 3 y 4 del mes):

- ingreso: regular | `partial.income_monthly` (**PLANO**, como todos los ingresos del motor, #139) |
  `income_retirement_monthly` (las partidas `persists_after_retirement`, plano);
- gasto: `expense_regular` | la base de la fase parcial (`expense_basis`, D10: **el de jubilación por
  defecto**, el regular si el perfil lo dice) | `expense_retirement` (las partidas
  `!ends_at_retirement`), **siempre × `f(k−1)`** (#139, decisión del owner: el gasto se indexa, los
  ingresos no) y **del presupuesto en los 3 modos de `savings_source`**;
- **pensión CON fecha: es ingreso en CUALQUIER fase** desde `start_index` (rejilla 0-based), con el
  MISMO factor de inflación que el gasto si está indexada (default D8) o plana si no, y
  × `fraction_while_partial` durante la media jornada. La pensión SIN fecha sigue viajando dentro de
  `income_retirement_monthly` y de `FireNeed::ExpenseMinusPension` — no ha cambiado;
- `income_pause` (P8.c) multiplica el ingreso **GANADO** dentro de una ventana **semiabierta**
  `[from_month, from_month + months)`. La pensión con fecha **no se pausa**: se suma después.
- El superávit corre la **MISMA cascada del usuario** también jubilado (4.12.1/#175): lo reinvertido
  sube la base de coste (#120) y abarata las ventas posteriores (#178).

**Las cuatro reglas de retirada × dos modos de gasto** (`crates/engine/src/withdrawal.rs`, D5/D6).
`L(k−1)` es el líquido de cierre del mes anterior —el MISMO valor que consume el cruce—, `R` es el
primer mes jubilado y el ancla de las reglas con memoria es `(L(R−1), f(R−1))`. **Los `pct` son
BRUTOS de impuestos, igual que el SWR** (R9): el techo topa la VENTA, no los euros que llegan al
bolsillo, así que con impuestos encendidos el neto de un techo del 4 % es menor que ese 4 % — eso es
el contrato, no un error de unidad.

| Regla | Permitido BRUTO del mes jubilado `k` |
|---|---|
| `fixed_real` | la necesidad del mes, **sin techo** (`None`: no hay regla que aplicar). Es el drenaje de 4.15.0 bit a bit |
| `percent_of_balance {pct}` | `pct/100 · L(k−1) / 12` |
| `hybrid {start,end}` | `start_pct` hasta el latch `end·L(k−1) ≥ start·L(R−1)·f(k−1)/f(R−1)`, `end_pct` a partir de ahí |
| `guardrails {pct,band,adjust}` | `W_R · mult · f(k−1)/f(R−1)`, con `mult` revisado cada 12 meses desde `R` |

- **`ceiling`**: se vende `min(necesidad, permitido)` y **solo en meses con déficit**.
- **`rule_is_spend`** (R7): se vende `permitido` **todos** los meses jubilados — la regla ES el gasto
  del patrimonio, y la pensión y las rentas son gasto aparte.
- Con `fixed_real` los dos modos COINCIDEN, y no por casualidad: el permitido se define como el
  déficit del mes, así que en un mes sin déficit no hay nada que gastar del patrimonio. Es la
  propiedad que mantiene 4.15.0 bit-idéntico bajo cualquiera de los dos modos (test
  `under_fixed_real_both_spend_modes_are_the_same_simulation`).
- **La fase parcial NO pasa por la regla**: las reglas se anclan en `L(R−1)`, que durante la media
  jornada todavía no existe.
- Guyton-Klinger (2006) implementa **solo** *capital preservation* (`ratio > ratio₀(1+band)` ⇒
  `W ·= 1−adjust`) y *prosperity* (`ratio < ratio₀(1−band)` ⇒ `W ·= 1+adjust`) sobre
  `ratio = 12·W_k/L(k−1)`; **la regla de la ventana de 15 años y el salto de inflación tras un
  recorte NO están implementados** (§4). En el camino determinista con rentabilidad > SWR la
  prosperity dispara todos los años (ratchet): es lo que la regla dice sobre un camino sin
  volatilidad, y por eso los guardarraíles solo tienen sentido pleno con Monte Carlo.
- Cotas del **MOTOR** (no las de producto, que viven en `handlers/retirement_profile.rs`): `pct`,
  `band_pct` y `adjust_pct` > 0, y `adjust_pct < 100 %`; si no, `EngineError::InvalidWithdrawalRule`.
  **Rechazar es la única salida honesta**: aceptar una regla y simular otra publicaría el patrimonio
  de un plan que nadie configuró.

**Las TRES magnitudes de la venta, separadas a propósito** (`sim_core::MonthSale::account`; hallazgo
B2 de la revisión + D22/D24). Confundirlas es el error caro, porque dos de ellas **no son** dinero
perdido:

| Magnitud | Qué mide | ¿Resta patrimonio? |
|---|---|---|
| `withdrawal` | retirada NETA efectiva del mes: `after_tax(bruto vendido)` | sí — sale de los activos |
| `withdrawal_shortfall` | lo que **la REGLA rechazó**: `max(0, need_net − neto que el techo permitía)` | **NO** — informativo; no entra en `uncovered_deficit_total` y **no cuenta como fracaso** (D22) |
| `unmet_need` (serie) / `uncovered_deficit_total` (acumulado) | lo que **los ACTIVOS no pudieron vender** de la venta intentada, acotado a la necesidad | sí — deuda implícita del hogar, NETA (mide gasto que faltó, no ventas que no ocurrieron) |
| `withdrawal_excess` | lo vendido y gastado **por encima** de la necesidad en `rule_is_spend` | sí — sale de la cartera y no vuelve |

**La identidad del mes cierra siempre**, y desde el pase de correcciones de la segunda revisión
adversarial (D20) es testable sobre hogares aleatorios (`crates/engine/tests/fuzz_invariants.rs`,
1.500 casos):

```text
withdrawal + withdrawal_shortfall + unmet_need − withdrawal_excess = need_net
```

La serie `unmet_need` es la tercera magnitud publicada mes a mes, y sin ella el reparto solo cerraba
cuando la venta se fundaba entera. **El contrato de cobertura de `crates/engine-stochastic`
(`McOutcome::withdrawal_to_need_ratio_p50`), completo tras dos correcciones sucesivas**:

```text
Σ max(0, withdrawal − withdrawal_excess) / Σ max(0, withdrawal + withdrawal_shortfall + unmet_need − withdrawal_excess)
```

sobre los meses JUBILADOS de cada camino, cada término CLAMPADO a `≥ 0` MES A MES antes de sumar
(`need_net` puede ser negativo desde el mes en que la pensión supera el gasto). Ninguna de las dos
correcciones es redundante con la otra:

- **Sin `unmet_need`** (fix anterior a 5.0.0): con `fixed_real` el recorte es cero por
  construcción, así que `Σw / Σ(w + recorte)` valía **1,0** («la regla cubrió el 100 %») en 1.000
  caminos de un hogar que cubrió el 8,7 % de su gasto (hallazgo #4 de la revisión adversarial).
- **Sin descontar `withdrawal_excess` del numerador y del denominador** (bug B2 del panel
  adversarial de 2026-09-06, corregido en `crates/engine-stochastic` WP E9 — DISTINTO del
  «hallazgo B2 de la revisión» de más abajo, de una revisión anterior): bajo `rule_is_spend` (D5)
  `withdrawal` incluye el EXCESO sobre la necesidad —la regla ES el gasto y vende `permitido`
  aunque sobre—, y ese exceso contaba en las dos mitades de la fracción sin restarlo, inflando la
  cobertura. Medido en el mismo hogar y la misma semilla: **0,9888 → 0,9793**.

La identidad `withdrawal + withdrawal_shortfall + unmet_need − withdrawal_excess = need_net`
(`crates/engine/tests/fuzz_invariants.rs`) es la que exige el clamp mes a mes y la que garantiza que
las dos correcciones son coherentes entre sí.

**`assets_depleted_month_index` — DOS condiciones, no una** (`sim_core`, corregido en el pase de
correcciones de la revisión D20): (1) primer mes cuya venta dejó lo vendible a CERO, medido
DESPUÉS de vender sobre los saldos —nunca comparando la venta con la capacidad antes—, **y** (2)
alguna venta sin fundar en ese mes o después. Sin la segunda condición, un aterrizaje EXACTO —la
cartera se vacía justo el mes en que entra una pensión que cubre todo el gasto posterior— se
publicaba como «cartera agotada» con `uncovered_deficit_total = 0`; con las dos, ese caso da
`None` (pin: 200.000 €/2.000 €/mes ⇒ mes 100 con pensión desde el 121, y un euro menos de capital
SÍ agota). **Corrige además un bug de 4.15.0**: el predicado antiguo (`venta_bruta >= drenable`,
evaluado ANTES de vender) fallaba por un ULP en la vía mixta y publicaba `uncovered_deficit_total
> 0` junto con «nunca agotado» — 184 → 47 casos por 3.000 entradas del corpus diferencial tras el
fix, con los restantes ≤ 5,6·10⁻²³ € (cola de redondeo, no el bug). Regresión:
`an_exact_landing_that_covers_every_later_need_is_not_a_depletion`.

**La vía mixta bajo techo tasa el rechazo con la `g` marginal, no con lo que faltó vender**
(hallazgo #3 de la revisión). Hasta el pase de correcciones, la vía mixta decidía si el techo de
la regla ataba comparando contra `dd.gross_monthly` —que el paseo YA había recortado a la
capacidad—, así que un techo por encima de lo vendible se descartaba en silencio: el rechazo
completo de la regla se contaba como `uncovered_deficit_total` (caso mínimo: 1.095 € de
descubierto en la vía mixta contra 916 recorte / 179 descubierto en la uniforme, con la MISMA
venta byte a byte). Ahora se decide contra lo que la NECESIDAD pide y el neto del techo se tasa
con la `g` MARGINAL (la del último tramo con material). **Los dos hogares no tienen por qué dar el
mismo número tras el fix**: solo coinciden en lo que se vende, no en cómo se tasa el neto de un
techo que la cartera no puede fundar — el uniforme tiene `g = 0,5` en todo, el mixto tiene el
tramo barato agotado y `g = 1` en el margen, y de ahí quedan 21 € de diferencia (937 vs 916 de
recorte, 158 vs 179 de descubierto) **por diseño**, la misma asimetría que ya existe cuando la
venta es parcial. Regresión: `the_binding_allowance_is_a_cut_on_the_mixed_path_too`.

**`rule_is_spend` financia el gasto de la regla PRIMERO con la caja del mes** (hallazgo #4 de la
revisión). Hasta el pase de correcciones, un mes jubilado con superávit hacía las dos cosas: la
cascada invertía el superávit en el fondo y la venta sacaba acto seguido el bruto de la regla del
MISMO fondo — comprar y vender el mismo euro el mismo mes no mueve patrimonio, pero el ida y
vuelta SÍ realiza plusvalía. Medido: 3.991,72 €/año de impuesto sobre un hogar con 1 M€ en un
fondo a `g = 0,5`, jubilado, ingreso 5.000 €/gasto 2.000 € (3.000 €/mes de superávit) y una regla
`percent_of_balance` al 4 % en `rule_is_spend` — ×10,7 el coste económico real. Ahora la venta es
0 y el impuesto también. Regresión: `rule_is_spend_funds_the_month_surplus_first`.

- Con `fixed_real`, `shortfall` y `excess` son cero **por construcción** (el permitido ES la
  necesidad), y ahí es donde el pin aditivo demuestra que las reglas no movieron la semántica de
  4.15.0.
- El descubierto se acota a la necesidad bajo `rule_is_spend` porque **nadie se endeuda para gastar
  de más**; con el objetivo = necesidad se conserva la expresión LITERAL de 4.15.0 (sin `min` ni
  `max`), que es lo que mantiene el pin bit a bit.
- `partial_phase_capital_growing` es `true` ⟺ **hubo** fase parcial y el líquido no bajó ni un mes
  durante ella; basta UN mes a la baja para `EngineWarning::PartialPhaseCapitalShrinking`. **El motor
  publica un `bool`** (es una función pura y debe definir el estado) y **la API publica
  `Option<bool>`** — `null` sin fase parcial, porque «no hubo media jornada» y «hubo y menguó» no
  pueden compartir valor en el wire.

**Necesidad ordinaria, puerta de tasa inicial y fallo de un camino** (E1 de 5.0.0; decisión M3 del
owner corregida por C1/C2 tras el panel adversarial de 2026-09-06).

La **necesidad ORDINARIA** del mes es `max(0, gasto_del_mes + retirada_extra − ingresos_del_mes)`,
con la pensión con fecha y las rentas persistentes ya dentro de los ingresos, el gasto ya indexado
(`×f(k−1)`) y **sin dos cosas a propósito**:

| fuera de la ordinaria | por qué |
|---|---|
| **servicio de deuda** | una cuota se extingue; capitalizarla a perpetuidad pide capital para un gasto que se acaba (el objetivo ya lleva su propio término finito de deuda, #142) |
| **`planning_adj` («Próximos»)** | un ingreso o un gasto puntual no describe el tren de vida, y un ingreso puntual que tape la caja de un mes NO puede convertir un plan roto en uno sano |

No confundirla con `need_assets_net` (el déficit de CAJA del mes), que sí lleva las dos y sigue
siendo lo que la venta persigue. La ordinaria es lo que **juzgan** el SWR y la regla de retirada;
la neta es lo que **se vende**. Se publica mes a mes en `SimOutput::ordinary_need` (no en
`ProjectionOutput`: la consume el crate estocástico, no la API).

**El SWR es la tasa INICIAL, no una comprobación mensual** (C1). Con `PhasePlan::initial_rate =
Some(InitialRateGate { swr_pct, bridge })`, en el **primer** mes jubilado `R` se compara

```text
12 · ordinaria(R)   >   tope/100 · L(R−1)          ⇒  el camino falla en R  (initial_rate_exceeded)
```

con `L(R−1)` el líquido de cierre del mes anterior —el mismo escalar que usan el cruce y las
reglas— y el tope evaluado por la MISMA función que topa las reglas por saldo
(`withdrawal::monthly_allowance`, una sola escritura de la fórmula). El `<` lo decide el tipo
(`MoneyOps::strictly_below`), como todos los booleanos publicados del bucle. **`initial_rate: None`
⇒ no hay puerta**: ninguna comparación se ejecuta (es el default de `PhasePlan::classic` y
`forced_at`, y por eso `pins-4.15.json` no se mueve).

Medido por el panel: la alternativa —comparar la venta ordinaria contra `SWR/12 · L(k−1)` TODOS los
meses— ponía la fecha válida de la demo en la edad de la pensión (72) y el capital necesario en
56 M€, porque cualquier caída del saldo convierte un plan sano en un fallo retroactivo.

**El puente (C2)** sustituye el tope por `bridge.max_pct` **solo** si hay pensión con fecha y llega
dentro de la ventana: `P − R ≤ 12·max_years`, con `P` el mes del BUCLE en que la pensión entra en
caja (`start_index + 1`). La resta es con signo: una pensión que YA se cobra en `R` entra en la
ventana, y ahí el tope mayor se aplica a una necesidad que **ya está neta de esa pensión**. No hay
tope mensual durante el puente: lo que queda después lo juzgan F1 y F3 mes a mes.

**Los tres motivos de fallo de UN camino** (`PathFailure`, literales públicos entre paréntesis),
evaluados solo en meses `Retired` o `Partial`, con prioridad **F1 > F2 > F3** dentro del mismo mes
y latches monótonos (`failure_month_index` / `failure_kind`, fijados la primera vez y nunca
movidos):

| # | Motivo | Predicado |
|---|---|---|
| F1 | `PortfolioDepleted` (`portfolio_depleted`) | la venta del mes no se pudo fundar (`unfunded_sale`, el booleano que publica el paseo) **y** quedó necesidad neta sin cubrir (`unmet_need > 0`) |
| F2 | `InitialRateExceeded` (`initial_rate_exceeded`) | la puerta de arriba, **solo en `R`** |
| F3 | `RuleBelowNeed` (`rule_below_need`) | con regla por saldo (`percent_of_balance`, `hybrid`, `guardrails`; **nunca** `fixed_real`), el NETO que la regla permitió está por debajo de la necesidad ORDINARIA del mes |

- **S1 — durante la media jornada solo puede fallar F1**: las reglas se anclan en `L(R−1)`, que
  todavía no existe, y la tasa inicial es una propiedad de la fecha de jubilación total.
- **Un mes ACUMULANDO no falla**, aunque vacíe la cartera: eso lo marca
  `assets_depleted_month_index` y describe a un hogar que gasta más de lo que gana hoy, no a un
  plan de jubilación que no aguanta. Por eso `failure_month_index` y `assets_depleted_month_index`
  **no** son el mismo campo ni tienen por qué coincidir (batería: `P1` agota en el 20 y no falla —
  nunca se jubila—; `P23` agota en el 16 y falla en el 17 — el 16 vacía la cartera con aterrizaje
  exacto y el 17 es el primer mes sin fundar).
- **F1 lleva `unfunded_sale` y no solo `unmet_need > 0`**, y es de dinero: `undrained` es una resta
  de netos y arrastra la cola de ±1e-24 € que `after_tax(gross_up(n))` deja. Medido sobre la
  batería: 6 de los 25 casos llevan una cola de **1e-25 €**, y con el predicado literal 3 de ellos
  —`P7` (mes 2), `P18` (mes 155) y `P21` (mes 122), los que la tienen en un mes jubilado— se
  publican como caminos FALLIDOS. Son hogares que jamás se acercan a quedarse sin cartera, y en el
  sorteo hundirían la probabilidad de éxito a cero.
- **F3 mira la ordinaria, no el déficit de caja**: una cuota de hipoteca puede atar el techo de la
  regla (y generar `withdrawal_shortfall`) sin que el gasto de vivir quede descubierto. Regresión:
  `f3_ignores_the_mortgage_and_looks_at_the_ordinary_need`.
- **El veredicto de un camino NO es el del plan.** El plan se juzga con la proporción de caminos
  sin fallo contra el umbral del perfil (`crates/engine-stochastic`); sobre la línea determinista
  estos dos campos describen un escenario, que es uno de los miles que deciden la fecha.
- Las dos decisiones son DISCRETAS y entran en la puerta de degeneración: `Decimal` y `f64`
  publican el mismo mes y el mismo motivo en los 25 casos de la batería (§`tests.md`).

**Fecha válida (definición A)** — decisión M1 del owner, corregida por C2/C3 tras el panel
adversarial. Vive en `crates/engine-stochastic/src/solve_mc.rs`; la API la publica como
`safe_date_month_index` y su base como `retirement_date_basis`.

> **La fecha válida es el primer mes `k ≥ k_min` que la búsqueda encuentra y la confirmación
> verifica, tal que jubilándose en `k` la proporción de caminos SIN NINGÚN fallo hasta el horizonte
> alcanza el umbral del perfil.**

Cada camino trae su propia acumulación (se sortea entero, desde hoy hasta el horizonte, con la
jubilación forzada en `k`) y **falla** ⟺ `SimOutput::failure_month_index.is_some()` — es decir, F1,
F2 o F3 en cualquier mes jubilado o parcial. «No volver a trabajar nunca» es exactamente eso: cero
fallos en todo el horizonte, no un saldo positivo al final.

La regla del umbral, con `z = 1,96`:

| umbral `u` | criterio | qué se publica al lado |
|---|---|---|
| `80 ≤ u < 100` | **`wilson_low ≥ u/100`** — el límite inferior del intervalo de score de Wilson al 95 %, no el estimador puntual | `success`, `wilson_low`, `half_width_pp` |
| `u = 100` | **`failures == 0`** (estimador puntual) | además `rule_of_three_upper = 3/N` — con 0/2.500, el riesgo real está por debajo del **0,12 %** |

Wilson y no la aproximación normal porque con `p̂ = 1` la normal da una barra de error
**exactamente cero**, y «100 % seguro con 2.500 caminos» es la clase de cifra que esta casa no
publica. Con cero fallos Wilson colapsa a la forma cerrada `n/(n + z²)` — `0,998466` con 2.500
caminos, `0,992376` con 500— y de ahí sale una cota que hay que tener presente al elegir
presupuestos: **un umbral `u < 100` es inalcanzable con menos de `z²·u/(1−u)` caminos** (73 para el
95 %, 381 para el 99 %), no falle ni un camino. `half_width_pp` es la distancia del estimador
puntual a la cota INFERIOR —el lado que decide—, no media anchura: el intervalo es asimétrico.
Medido antes de C3: con el estimador puntual y umbral 100 la fecha era el **mínimo muestral** y
bailaba ±10 años según la semilla, sin converger al subir `N`.

**El suelo `k_min` lo pone el llamante**: `1` sin puente y `max(1, P − 12·bridge_max_years)` con
puente, con `P` el mes en que la pensión entra en caja. **Es el único sitio donde
`bridge_max_years` acota la FECHA** — el tope de tasa inicial que el puente levanta es cosa del
motor (`InitialRateGate::bridge`), y confundir los dos ejes fue exactamente lo que el panel midió.

**La honestidad de lo que se devuelve.** Lo que la fecha garantiza es **«un mes verificado que
cumple», no «el mínimo demostrable»**, y la UI no puede prometer lo segundo. La búsqueda es una
bisección con extremo verificado (bracket de 5 años → año → mes) y el mes devuelto se **reevalúa
con el presupuesto grande** antes de publicarse; lo que no se supone en ningún punto es la
MONOTONÍA de `éxito(k)`, porque se sabe que se rompe: un «Próximo» es un flujo en un mes absoluto,
una fase de media jornada con base de gasto regular se encarece al alargarse, y con la inflación
por encima del crecimiento neto de la cartera el éxito **decrece** con `k` en tramos enteros del
horizonte. El único dato honesto sobre la minimalidad es `predecessor_success`: cuánto éxito tiene
el mes inmediatamente anterior, medido con el mismo presupuesto — `null` cuando el mes devuelto es
el suelo, **nunca un 0**. Y `date_is_approximate = true` dice que ni el mes que la búsqueda verificó
ni los doce siguientes cumplieron al confirmarlos: la fecha se publica igual, con su éxito real al
lado, porque «no lo sé» no es lo mismo que «no existe».

**Sin fecha en el horizonte se publica `month: null`, jamás un 0** —un 0 se leería como «ya
puedes»— junto a `best_effort` (el mejor par mes/éxito observado) y `failures_by_kind`, que dice
por qué no la hay: todo F2 ⇒ la tasa inicial no da; todo F1 ⇒ la cartera no aguanta el horizonte.

Presupuestos y coste (release, P9 a 840 meses): **se busca con 500 caminos y se confirma con
2.500** (≈ 100–110 ms y ≈ 445–465 ms por sorteo). Un solve completo A→E son 12–14 sorteos de
búsqueda más 2 de confirmación ⇒ **1,8–1,9 s**; la cota del peor caso (25 + 14 sorteos) es
**≈ 8,7–9,2 s**. Objetivo del plan: ≤ 3,5 s típico, ≤ 10 s peor caso. Los mide, sin afirmarlos,
`crates/engine-stochastic/tests/timing_mc.rs::the_date_solve_costs_what_the_plan_says`.

**Los solves — inversas por bisección sobre el MOTOR ENTERO** (`crates/engine/src/solve.rs`,
`MAX_SOLVE_ITERATIONS = 24`, una `project_net_worth_series` completa por evaluación). **E4 se llevó
del motor los dos que preguntaban «¿llego a `T(R−1)`?»** —`required_contribution_monthly` y
`coast_fire_month_index`, con `SolveResult`, `CoastSolve` y el aviso `coast_not_reachable`—: su
criterio murió con el objetivo como decisión (M4), y sus preguntas se responden ahora contra el
umbral de éxito en `crates/engine-stochastic`. Lo que queda aquí son
`max_extra_monthly_expense_keeping_date` y `retirement_delay_months` (las dos sobre
`retirement_month_index`, sin objetivo de por medio) más los dos motores de escenario públicos
`run_with_cap`/`run_stopping_at`, que el crate estocástico reutiliza en vez de copiarlos. **La
doctrina de abajo sigue rigiendo esos solves**, estén donde estén. No hay forma
cerrada y es deliberado (hallazgo M8): un «capital necesario» descontado a una tasa escalar ignora la
cascada, los topes de las reglas, el servicio de deuda, los Próximos, la fiscalidad del drenaje y el
propio latch — sería un número plausible que **ninguna simulación produce**. Cada bisección mantiene
un extremo verificado BUENO y otro verificado MALO y devuelve el bueno, así que el valor publicado
está *comprobado*; lo que la monotonía aporta es la minimalidad, no la validez. **Y la monotonía
NO siempre aguanta** (revisión adversarial, contra la afirmación anterior de que «se aplana, no se
invierte»): sobre valores por activo `líquido(R−1)` es no decreciente en la aportación, pero el
criterio real es líquido POST-IMPUESTOS, y subir el techo cambia el MES en que cada tope por
activo se llena — con él, la trayectoria de la BASE DE COSTE, y dos ejecuciones con el mismo valor
por activo y distinta base pagan distinto impuesto por el mismo neto. Medido en un barrido de 320
hogares aleatorios: 35 violaciones de 270 barridos del techo, la PEOR de 3,4416 € (~5.700 veces la
resolución de la bisección). Hacen falta impuestos activados y al menos un activo ilíquido; apagar
cualquiera de las dos cosas la hace desaparecer. No compromete el resultado: la bisección solo
devuelve `hi` tras comprobar que `hi` CUMPLE, así que nunca es un falso positivo — lo que la
inversión pone en duda es que `c` sea la mínima DEMOSTRABLE, no que sea válida.

- La aportación mínima es un **TECHO** sobre lo que la cascada invierte cada mes, no un importe que
  se aporte pase lo que pase: en un mes con menos sobrante se aporta el sobrante (R5). Vale igual
  para el solve estocástico que heredó la pregunta.
- **El techo de búsqueda es el MÁXIMO SOBRANTE MENSUAL del horizonte** (`search_ceiling`, todavía en
  `solve.rs`), no el neto recurrente del mes 1 que R5 dejaba abierto — decidido con la medición
  delante: sobre el caso P9 el neto del mes 1 son 500 €/mes, y a 600 meses la ejecución con ese
  techo cierra en **91.444 €** frente a **725.197 €** sin techo. Con la cota de R5 se declararía
  «no llegas» a hogares cuya simulación REAL sí llega: un rojo falso. El sobrante del mes 1 se
  conserva como SUELO de la cota. Regresión:
  `the_solve_ceiling_is_the_max_monthly_surplus_not_the_first_months_headroom`.
- `max_extra_monthly_expense_keeping_date` (P8.b) sube **solo `expense_regular_monthly`** — ni el
  gasto de jubilación ni la necesidad que el objetivo capitaliza: la pregunta es «¿cuánto margen
  tengo AHORA?», no «¿cuánto puedo subir mi nivel de vida para siempre?». Con un trigger por EDAD
  —que no depende del gasto— devuelve la cota como **suelo honesto**, nunca un infinito inventado.
- `retirement_delay_months` (P8.c): dos simulaciones, sin bisección; `delay_months = null` cuando
  cualquiera de los dos escenarios no se jubila dentro del horizonte — «la pausa te saca del
  horizonte» es una respuesta, pero no es un número de meses.

**Techo de aportación y margen disponible.** `contribution_cap_monthly` (la palanca del solve de
aportación mínima) y `contributions_stop_month` (la del solve de coast) — las dos se mutan desde
fuera con `run_with_cap`/`run_stopping_at` — recortan a `min(sobrante, c)` el pool que llega a la cascada; el resto **no se invierte, no compone y
no entra en `net_worth`**: sale del balance y se publica en `disposable_cash` — el mismo trato que
`unallocated_savings_total` y por la misma razón (el modelo no simula un euro sin destino declarado).
Identidad del mes con sobrante > 0: `sobrante = Σ aportado + no_asignado + disposable`. Sin techo es
cero mes a mes y no se ejecuta ni una operación de más (bit-identidad).

**Los tres solves de ESTRATEGIA** (5.0.0 E8, `crates/engine-stochastic/src/strategy_solves.rs`;
decisiones M10/M11/M12 y corrección C8). Mismo criterio que la fecha —el umbral de éxito sobre el
sorteo— y misma doctrina —bisección sobre el motor entero con **extremo verificado**—, cada uno
sobre un eje distinto del `PhasePlan`:

| solve | qué significa la cifra | criterio | presupuesto de iteraciones |
|---|---|---|---|
| `minimum_extra_contribution(r)` | **euros/mes de aportación extra, PLANA EN NOMINAL** (supuesto S2, #139), redondeada **hacia arriba a decenas** | `éxito(r)` con `c` inyectado en los meses `1..=r−1` cumple el umbral | 1 sonda de `c = 0` · ≤ 1 + **12 doblajes** del techo · ≤ **12** de bisección · 1 + ≤ 6 de confirmación |
| `coast_stop_month(r)` | **el PRIMER mes desde el que se puede dejar de aportar** (C8: el primero, no el último) | `éxito(r)` con `contributions_stop_month = C` cumple el umbral | 1 sonda alta (`C = r`) · 1 sonda baja (`C = 1`) · ≤ **12** de bisección · 1 + ≤ 6 de confirmación |
| `earliest_partial_start()` | **el PRIMER mes en que puede empezar la media jornada** | la FASE no falla, evaluada con `AtMonth(H+1)` (nunca se jubila del todo) | 2 sondas · ≤ **12** de bisección · 1 + ≤ 6 de confirmación · **UNA** `valid_retirement_month` con `k_min = S*+1` |

- **Se busca con 500 caminos y se confirma con 2.500**, la misma partición que la fecha. Medido en
  release sobre P9 sin inflación (840 meses): aportación **1,9 s** (16+1 sorteos; ≤ 3 s del plan),
  coast **1,2 s** (11+1; ≤ 2 s), jornada reducida **2,4 s** (23+1 contando la fecha anidada; ≤ 5 s).
- **Los euros que publican son `Decimal` del camino EXACTO.** `extra_monthly` y
  `contribution_required_search_ceiling` los construye el solve (suelo de 100 €, sobrante del mes 1,
  doblajes y medias exactas); el ahorro liberado del coast sale de una ejecución determinista. El
  sorteo decide **qué escenario cumple**, nunca **cuánto vale** — la regla «de aquí no sale un euro»
  del crate estocástico se refiere a cifras DERIVADAS de la coma flotante, y ninguna de estas lo es.
- **La aportación se inyecta como «Próximo», no como ingreso**, y por una razón de contrato:
  `planning_adj` está FUERA de `ordinary_need` (§ puerta de tasa inicial), así que aportar más no
  rebaja la necesidad que el SWR capitaliza. Subir `income_regular_monthly` sí lo haría, y la
  respuesta saldría optimista por partida doble.
- **El techo de búsqueda de la aportación se DESCUBRE doblando**, no se lee de
  `solve.rs::search_ceiling`: aquella cota (el máximo sobrante mensual) es correcta para un solve que
  pone TECHO a lo que la cascada invierte, pero aquí la incógnita es **dinero nuevo** que la caja
  actual no acota. `underfunded` ⟺ ni el techo cumple, y entonces la cifra es **null, nunca un 0**.
- **El ahorro liberado del coast es caja DISPONIBLE (supuesto S4), no se reinvierte**: con el corte,
  el pool que llega a la cascada es 0 y el sobrante entero cae en `disposable_cash`. Se lee del mes
  `C` —el corte es INCLUSIVO (`k ≥ C` ⇒ techo 0), así que `C` ya es el primer mes sin aportación— y
  la regresión comprueba la identidad que lo cierra: lo liberado es **exactamente** lo que le falta a
  la cartera frente a la ejecución sin corte.
- **Durante la media jornada solo puede fallar F1** (supuesto S1 del motor): F3 está restringida a
  `Retired` y la puerta de tasa inicial se evalúa en el primer mes jubilado, que con `AtMonth(H+1)`
  no llega. Por eso «la fase no falla» significa literalmente «la media jornada no se come la
  cartera».
- **Avisos** (literales de cable): `coast_not_reachable` (ni aportando siempre se cumple),
  `partial_never_starts` (la fase falla incluso empezando en el horizonte),
  `partial_never_fully_retires` (se puede empezar, pero no hay fecha de jubilación total) y
  `retire_at_age_underfunded` (el sucesor probabilístico del `EngineWarning` que E1 retiró: hoy sale
  de `underfunded`, no de comparar un líquido contra una perpetuidad).
- **Lo que NO se garantiza, igual que en la fecha: la MINIMALIDAD.** Los tres devuelven un valor
  **verificado**, no el mínimo demostrable.

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
- **`success_verdict`: contra el umbral del PERFIL, con Wilson (V7 sustituida por §2.5/C3, modelo
  v2, WP A6)** — `crates/engine-stochastic::SuccessAt::meets`, espejado bit a bit en
  `handlers/projection_bands.rs::success_verdict`. Verde ⟺ `meets(threshold_pct)`: con
  `threshold < 100` es `wilson_low ≥ threshold/100` (la cota INFERIOR de Wilson, no el estimador
  puntual); con `threshold == 100` es `success == 1.0` exacto (`n/n` en IEEE 754, sin épsilon).
  Ámbar ⟺ el estimador puntual llega pero el intervalo no (`success ≥ threshold/100` sin cumplir
  `meets`); con `threshold = 100` **no hay ámbar** — o ningún camino falla, o es rojo
  (`the_hundred_percent_threshold_has_no_amber_band`). Rojo en el resto. La V7 vieja («corte fijo
  al 100 %, `success_threshold_pct` se acepta y se ignora») describía el modelo ANTES del panel
  adversarial de 2026-09-06 (C3): hoy el umbral SÍ gobierna el veredicto y viaja en la respuesta
  (`success_threshold_pct`, 80–100, default 95) junto con `success_wilson_low` y
  `success_sampling_error_pp`. **Qué NO decide esta función**: si el plan TIENE fecha —con
  `retirement_date_basis: not_reachable` el escenario sorteado es «no jubilarse dentro del
  horizonte», que casi nunca falla, así que un verde ahí dice «este plan sin jubilación aguanta»,
  no «llegas»; quien pinte el semáforo mira antes `retirement_date_basis` en la serie.
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
12. **Bit-identidad PINEADA, no prometida (5.0.0)**: `crates/engine/tests/golden_pins.rs`
    canonicaliza a TEXTO todas las salidas del motor caso a caso —hasta el último dígito de cada
    `Decimal`, vía `Display`— y las resume en un SHA-256 por caso contra dos fixtures:
    `tests/fixtures/pins-4.15.json` (las salidas que 4.15.0 ya publicaba) y
    `pins-5.0-outputs.json` (las lecturas de fase, las tres series de retirada y las de WP3). El
    refactor por fases, las cuatro reglas de retirada, el objetivo con puente y la conversión del
    bucle a un núcleo genérico sobre `MoneyOps` pasaron **sin mover un byte del primero**. Que la
    red funcione tiene su propio control: `the_hash_actually_notices_a_single_moved_decimal` y
    `the_5_0_hash_notices_a_moved_withdrawal_and_a_moved_phase`. Regenerar es un acto DECLARADO
    (`UPDATE_ENGINE_PINS=1` / `UPDATE_ENGINE_PINS_5_0=1`) y **exige entrada de CHANGELOG**: un pin
    regenerado sin ella es un cambio de números que nadie declaró. Cuenta los casos, no te fíes de
    una cifra escrita:
    `python3 -c "import json;print(len(json.load(open('crates/engine/tests/fixtures/pins-4.15.json'))['cases']))"`.
13. **La bisección se usa donde de verdad no hay forma cerrada, y sobre el modelo entero**: los
    solves de §2.5 bisecan ejecutando la simulación completa (≤ 24 iteraciones) y devuelven un
    extremo VERIFICADO. Es lo contrario del gross-up, donde la bisección se retiró por tener forma
    cerrada (§2.4). No «arregles» ninguno de los dos convirtiéndolo en el otro.

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

### Aceptadas por el owner (2026-09-03, tren 5.0.0) — decisiones del plan de #207

| Divergencia | Coste (sintético) | Razón de aceptación |
|---|---|---|
| **El recorte de una regla de retirada NO es fracaso ni descubierto** (D22/D24): `withdrawal_shortfall` puede crecer todo un horizonte sin que el patrimonio lo note | 0 € de patrimonio; sí cambia la lectura de «¿me va bien?» | Un hogar que gasta menos porque su regla se lo dice **está siguiendo su plan**, no arruinándose. Meterlo en `uncovered_deficit_total` mezclaría una decisión con una imposibilidad — hallazgo B2 de la revisión adversarial |
| ~~**Éxito de Monte Carlo = el plan OCURRE y AGUANTA** (D22 corregida por la revisión D20): jubilarse dentro del horizonte —o tener un trigger por edad— **y** no agotar la cartera~~ — **decisión SUSTITUIDA en 5.0.0 v2 (E9, 2026-09-06)**, ver abajo | La definición D20 bajaba la probabilidad publicada donde el cruce era tardío: medido 0,960 → 0,629 en un hogar que cruza en el mes 655 de 840, porque D22 premiaba al hogar que **no se jubila jamás** (33,1 % de los caminos no llegaba a jubilarse y los 1.000 contaban como éxito; sesgo hasta **+6,8 pp** con SWR 6 %) | Paso intermedio, no el final: D20 separaba «¿ocurre?» de «¿aguanta?» mientras el disparador seguía siendo binario (cruce o edad). **E9 la sustituye** porque en v2 el disparador YA NO ES la pregunta —todo plan trae un mes FORZADO que el solver externo resuelve—, así que la única pregunta es si ALGÚN motivo (F1/F2/F3) falla en el camino: `success_probability` = caminos con `failure_month_index.is_none()` / N. `never_retired_probability` y `success_given_retired` **se retiraron de `McOutcome`** — esa pregunta murió con el disparador variable, no con una corrección de sesgo |
| **Guyton-Klinger sin la *portfolio management rule* (ventana de 15 años) ni la *inflation rule*** (saltarse la subida por IPC del año siguiente a un recorte) | Modelo **más reactivo**: recorta antes y más veces que el artículo de 2006 | ~~Las dos omitidas SUAVIZAN la regla; omitirlas va en la dirección prudente.~~ **Esa lectura es el signo FALSO que el modelo v2 dejó escrito y aún no se ha corregido** (declarado en `docs/jubilacion.md` §Lo que el modelo NO hace): la *inflation rule* de la literatura sirve para no acumular DOS recortes reales el mismo tramo malo (el recorte de capital preservation Y la subida de IPC completa encima); omitirla no es «más prudente», es una regla MÁS agresiva de lo publicado en 2006, sin que el modelo lo declare como tal. Deuda con dueño: issue [#220](https://github.com/maxlainz/FutureFin/issues/220) |
| **Un solo shock de mercado común por mes, escalado por la sd de cada activo** (D11), en vez de una matriz de correlaciones | Subestima la diversificación entre clases: las bandas salen **más anchas** de lo que daría una correlación < 1 | Una matriz de correlación exige datos que la instalación no tiene (el usuario declara μ y σ por activo, no covarianzas); inventarlas sería falsa precisión, y el sesgo es conservador. **Simulado desde WP6a** (commit `ba6bdfe`, 2026-09-03): `engine_stochastic::project_percentile_bands` inyecta por mes `f_ik = d_i·exp(σ_i·z_k − σ_i²/2)` con `d_i = m_i·exp(σ_i²/2)` (un solo `z_k` por mes para toda la cartera; **`mediana(f) = m_i` exacta** y `E[f] = d_i`, porque la declarada es la CAGR — M8, 2026-09-06; `σ = 0` ⇒ `d_i = m_i` y `f = m_i`, las dos por rama explícita) sobre el MISMO bucle genérico. **La sd NO viaja en `SimAsset`**: se pasa como slice alineado a `assets[]`, así que el camino `Decimal` la ignora por construcción y su bit-identidad no depende de nadie. (La suite del crate está en VERDE desde el pase de correcciones de la revisión D20.) |
| **`partial_phase_capital_growing`: `bool` en el motor, `Option<bool>` en la API** | 0 € | El motor es una función pura y debe definir el estado (sin fase parcial ⇒ `false`); el wire no puede darle el mismo valor a «no hubo media jornada» y a «hubo y menguó», así que la capa que serializa lo convierte en `null` mirando `partial_retirement_month_index`. Verificado en `apps/api/src/handlers/projection.rs` |
| **Cola de redondeo negativa de `uncovered_deficit_total` clampada al PUBLICAR, no en el motor** | medido hasta ≈ −1,7·10⁻²⁴ € (y hasta +5,6·10⁻²³ en el corpus diferencial) | El descubierto se acumula como residuo de ventas brutas y puede salir con una cola negativa que no es «−0,0000000000000000000000005 € descubiertos», es cero. El motor debe seguir publicando su aritmética tal cual —el golden la hashea—; quien redondea para un humano es la capa que serializa (`money_out(… .max(ZERO))`) |
| **La sd del activo no llega al motor determinista** | 0 € en el camino `Decimal` | Por diseño: la volatilidad **no es un campo de `SimAsset`** — viaja como argumento del evaluador estocástico, así que el camino exacto no puede verla y su bit-identidad con 4.15.0 no depende de una rama que alguien pueda tocar. De ese camino no sale un euro (§1) |

### Aceptadas por el owner (2026-09-05/06, modelo de jubilación v2) — decisiones M1–M13 + C1–C8

El rediseño completo del modelo de jubilación (entrevista 2026-09-05/06 + panel adversarial de la
mañana del 06): «el éxito define la fecha», no un objetivo estático que el patrimonio cruza. Las
decisiones M1–M13 son la entrevista; C1–C8 son las correcciones que el panel adversarial (5 lentes,
39 objeciones, 17 refutadas con evidencia) forzó sobre esas mismas M al reimplementar el modelo en
un Monte Carlo independiente con los datos de la demo. Documento fuente: «Modelo de jubilación v2»
(memoria de sesión); implementación: WPs E1–E9 (motor/estocástico), A1–A12 (API), W1–W9 (SPA),
D1–D3 (docs) de la rama `release/5.0.0`.

| Divergencia / decisión aceptada | Coste o consecuencia (sintético) | Razón de aceptación |
|---|---|---|
| **El número FIRE clásico (25× el gasto) sobrevive solo como lectura informativa** (M9, S7): no dispara nada y no resta la pensión con fecha | Casi nunca coincide con la fecha ni el capital que el plan publica — es una cifra de cultura FIRE, no del modelo | Es el número que la literatura y la comunidad FIRE conocen; retirarlo del todo habría sido más disruptivo que rotularlo bien. Vive en «Detalle del cálculo» y como pin de `fire-parity.json` |
| **Guardrails (Guyton-Klinger) sin ventana de 15 años ni *inflation rule*** (M3/C1 heredan la limitación ya aceptada arriba) | Regla más reactiva que el artículo de 2006 | Ver la fila de arriba — **el signo se corrigió de «prudente» a issue declarado (#220)** en esta misma pasada |
| **El hogar (`view=household`) no resuelve fecha ni éxito propios** (§5 del modelo v2, `plan_state: household_not_solved`) | Sin fecha válida agregada del hogar; cada persona resuelve la suya en `view=mine` | Un sorteo por miembro más una agregación por hogar es un tercer modelo (correlación entre planes) que nadie pidió resolver en esta fase; issue #219 |
| **`fire_number_mode` / ingresos actuales gobiernan el GASTO que el bucle drena, no solo un objetivo** (S8) | La tarjeta «Gasto en jubilación» decide una magnitud que antes solo alimentaba el objetivo perpetuo | Mantiene un solo lugar de verdad para «cuánto gasta el plan jubilado»: el bucle y el número clásico leen la MISMA cifra en vez de dos derivaciones que podían divergir |
| **La aportación mínima y el ahorro liberado de coast son NOMINALES PLANOS** (S2/S9, #139) | Con inflación alta, el poder adquisitivo de una aportación fija cae con los años | Coherente con que el motor NUNCA indexa ingresos (#139, decisión de 4.9.0): indexar solo esta aportación habría sido una excepción sin justificar |
| **`MAX_SWR_PCT` sube de 4 a 6** (S9) | Perfiles con tasas de retirada agresivas (5–6 %) dejan de ser rechazados por el servidor | Los guardarraíles de la literatura (Guyton-Klinger, variable %) arrancan en 5–5,6 %; un tope en 4 los habría hecho irrepresentables |
| **Parámetros MCP retirados se DEPRECAN, nunca se borran** (schemas `deny_unknown_fields`) | Un cliente MCP viejo que siga mandando `target_basis`/`cash_buffer_months`/etc. no rompe, pero tampoco hace nada | Rotura silenciosa de un agente en producción es peor que un campo fantasma documentado como «ignorado desde 5.0.0» |
| **El excedente en jubilación sigue las reglas de ahorro sin publicar su coste fiscal** | Reinvertir el sobrante tiene un coste en impuestos que el plan no enseña; los tres modos de gasto (`ceiling`/`rule_is_spend`) acaban pareciendo el mismo «gasto de hoy» por dentro | Calcular y publicar ese coste es una campaña de proyección-realismo aparte, no una corrección de esta; issue [#227](https://github.com/maxlainz/FutureFin/issues/227) |

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

**Ampliado el 2026-09-06 (WP D2, cierre documental del modelo v2 — API y SPA ya en v2, WPs A1–A12 y
W1–W9 mergeados en `release/5.0.0`)**: tres correcciones de deriva doc↔código que las entradas de
abajo (escritas por E7/E5/E3, ANTES de que A4/A6 tocaran el ensamblado) habían dejado desactualizadas:
§2.5 tenía la tabla de estrategias vieja (cinco filas, `pension_bridge` con fila propia,
`crossing_is_reading_only` descrito como «solo para estrategias por edad») — sustituida por la de
cuatro estrategias con el mes forzado uniforme (`ForcedMonth::Known`/`NeedsSolve`,
`handlers/projection.rs:2531` pone `crossing_is_reading_only = true` INCONDICIONAL, verificado
`grep -n "crossing_is_reading_only = true" apps/api/src/handlers/projection.rs` → 1 hit sin guardas).
§2.7 tenía `success_verdict` descrito con el corte fijo al 100 % de V7 — sustituido por el contrato
real de `handlers/projection_bands.rs::success_verdict` (Wilson contra el umbral del perfil),
verificable con `grep -n "fn success_verdict" -A 15 apps/api/src/handlers/projection_bands.rs`. La
fila de Guyton-Klinger en §4 tenía el signo «prudente» que el propio `docs/jubilacion.md` (WP D1,
mismo día) ya había marcado como falso — corregida para apuntar al issue
[#220](https://github.com/maxlainz/FutureFin/issues/220) en vez de repetir la promesa rota. D4 gana
la condición de las decisiones M1–M13/C1–C8 (semilla y caminos como identidad del resultado cuando
fijan un hito) y se añade el bloque «Aceptadas por el owner (2026-09-05/06)» con `#227` como deuda
declarada. Re-verificación: `grep -n "DATE_BASIS_SUCCESS_THRESHOLD\|DATE_BASIS_TARGET_AGE" apps/api/src/handlers/retirement_solver.rs`
(el wire real, no `retirement_trigger`) y `cargo test -p futurefin-engine-stochastic 2>&1 | grep "test result"`
(75 passed, 0 failed, 7 ignored — 6 binarios de test: `degeneration` 3, `monte_carlo` 13,
`needed_capital` 9, `solve_mc` 11, `strategy_solves` 13, más 26 unitarios de `src/`).

**Ampliado el 2026-09-06 (WP E7 del plan «Modelo de jubilación v2», decisión M9 del propietario y
corrección C4 del panel adversarial)**: §2.4 gana el **capital necesario**, que sustituye al
objetivo FIRE como el número del plan (el clásico 25× sobrevive como escalar informativo). Se
calcula biseccionando un factor `λ` que escala el patrimonio LÍQUIDO —valor **y** base de coste, o
aparece una plusvalía fantasma— hasta que el sorteo cumple el umbral, y se publica redondeado a
cientos **hacia arriba**. El importe se lee de la trayectoria del hogar ESCALADO, no del producto
`λ*·L_det(k−1)` (corrección del mismo día: el producto solo es exacto en `k = 1` y anulaba los nodos
tardíos de la curva). Re-verificación:
`cargo test -p futurefin-engine-stochastic --test needed_capital` (9 tests) y
`grep -n "round_up_to_hundreds" crates/engine-stochastic/src/needed_capital.rs` (4 hits: la
definición, sus dos consumidores en `publish` y el test unitario). Coste medido en release sobre P9:
`cargo test -p futurefin-engine-stochastic --release --test timing_mc -- --ignored the_needed_capital_solve_costs_what_the_plan_says --nocapture`.

**Ampliado el 2026-09-06 (WP E5 del plan «Modelo de jubilación v2», decisiones M8 y C6 del
propietario)**: la rentabilidad declarada de un activo es **COMPUESTA (CAGR)** y no la media
aritmética de los retornos. El motor `Decimal` no cambia —sigue componiendo `(1+p/100)^(1/12)`— y
las cifras ya guardadas **no se convierten**: se reinterpretan (C6). Lo que cambia es el sorteo, que
ahora deriva el factor mensual a `m·exp(σ_m²/2)` para que su MEDIANA sea `m`; la consecuencia
publicable es que **la línea determinista es la mediana de los caminos**, no su media. La conversión
vive en **un solo sitio** y ese sitio es el único que puede cambiarla. Re-verificación:
`grep -n "0.5 \* s \* s" crates/engine-stochastic/src/mc.rs` (2 hits: la deriva en `PathEngine::new`
y la corrección de Itô en `run`) y
`cargo test -p futurefin-engine-stochastic --test monte_carlo mc_median_is_the_deterministic_line`.
**Matiz que hay que decir**: la identidad «mediana = línea determinista» es exacta solo SIN flujos;
con aportaciones o retiradas la mediana del patrimonio se separa unos pocos puntos porcentuales
(±2–4 % a 20–35 años, medido por el panel adversarial del modelo v2), porque cascada, drenaje y
fiscalidad no son lineales.

**Retirado el 2026-09-06 (WP E3 del plan «Modelo de jubilación v2», decisión del propietario)**: el
colchón de caja se elimina ENTERO de `crates/engine` y `crates/engine-stochastic` antes de que
5.0.0 se publique — la caja es un activo más y las reglas de ahorro fijan cuánto se guarda; no hay
un mecanismo de relleno aparte. §2.3 y la fila de la tabla de §4 sobre el colchón se sustituyen por
una sola frase. Esto vuelve OBSOLETOS los comandos de re-verificación de la entrada anterior
(2026-09-05): `CashBufferTarget`, `CashBufferPlan`, `CashBufferSpec`, `BufferInactiveReason`,
`cash_buffer_index`/`safe_cash_buffer_index` y `refill_cash_buffer_g` ya no existen en ningún
crate — un grep de cualquiera de ellos en `crates/` debe salir VACÍO, y eso es lo esperado, no
deriva. `apps/api/src/handlers/cash_buffer.rs` queda huérfano (referencia tipos retirados) hasta
que un WP posterior lo retire también: el binario `futurefin-api` NO compila entre este commit y
ese WP, por diseño.

**Ampliado el 2026-09-05 (WP-F del tren 5.0.0, decisiones V6/V7)**: §2.3 ganó el contrato del
colchón derivado y §2.7 el veredicto de corte fijo al 100 %. La parte del colchón queda descrita
arriba como retirada; ~~§2.7 (veredicto al 100 %) sigue vigente y su re-verificación no cambia:
`grep -n "VERDICT_GREEN_FLOOR_PCT" apps/api/src/handlers/projection_bands.rs` (2 hits) y
`cargo test -p futurefin-api --lib projection_bands::tests::el_verde_exige_todos_los_caminos`~~ —
**esto dejó de ser cierto el 2026-09-06 (C3, entrada WP D2 arriba)**: el corte fijo se sustituyó por
el contrato Wilson-contra-umbral-del-perfil; `VERDICT_GREEN_FLOOR_PCT` ya no existe
(`grep -c "VERDICT_GREEN_FLOOR_PCT" apps/api/src/handlers/projection_bands.rs` → 0) y el test citado
tampoco está en el árbol.

Escrito 2026-08-30 (auditoría del modelo financiero; rama `audit/modelo-financiero`).
**Ampliado y re-verificado el 2026-09-03 para 5.0.0** (rama `release/5.0.0`, issue #207): §2.2/§2.3
re-anclados al núcleo genérico, §2.4 gana el objetivo consciente del plan, §2.5 se reescribe por
fases, §3 gana los pines de bit-identidad y §4 seis divergencias nuevas. **Todos los comandos de
abajo se ejecutaron el 2026-09-03 y ninguno sale vacío.**

**Re-sincronizado el 2026-09-03 tras el pase de correcciones de la revisión adversarial** (commit
`0668f37`, issue #207 cerrado): §2.4 gana `BridgeDiscountOverflow` y la tabla de `d` alcanzable;
§2.2 gana la restauración de bit-identidad (P24/P25) y el resultado del fuzz diferencial; §2.5
gana la definición de dos condiciones de `assets_depleted_month_index` (con el bug de ULP de
4.15.0 que corrige), el residuo de 21 € de la vía mixta, `rule_is_spend` financiado desde el
superávit y la inversión de monotonía medida en los solves. Los seis documentos que citaban la
suite del crate estocástico «en ROJO» quedan corregidos (era la predicción de un test, no un
estado permanente — ver §El crate estocástico de `.claude/tests.md`).

**Re-sincronizado el 2026-09-06 con E4 del modelo de jubilación v2** (rama `release/5.0.0`): §2.4
sustituye el «objetivo consciente del plan» por el **número FIRE clásico** (una base, sin restar la
pensión con fecha, sin trigger) y retira las filas del puente, del descuento, de
`MAX_BRIDGE_MONTHS` con su violación LATENTE y de `BridgeDiscountOverflow`; §2.5 tacha en la tabla
de estrategias las lecturas que se fueron con ellos y marca los dos solves deterministas que
emigraron a `crates/engine-stochastic`. Los greps de re-verificación de esos contratos se
sustituyeron por greps de AUSENCIA — un grep vacío es la señal correcta cuando lo que se vigila es
que algo no vuelva.

El arnés de verificación es permanente: `crates/engine/tests/audit_dump.rs` vuelca las series de la
batería de casos límite (`cargo test -p futurefin-engine --test audit_dump -- --nocapture`),
comparables con un oráculo externo, y desde 5.0.0 el pin dorado
(`cargo test -p futurefin-engine --test golden_pins`) hashea esas mismas salidas. Re-verificación
(un comando por contrato; si un grep no devuelve nada, el ancla se movió — actualiza esta ficha en
el mismo cambio):

- Devengo francés: `grep -rn "payoff = P·(1 + i)" crates/engine/src/ || grep -n "fn liability_month_g" crates/engine/src/sim_core.rs`
- Convención TIN/1200 compartida: `grep -rn "1200" crates/engine/src/{projection,history}.rs | grep -c "apr"` (≥2)
- Raíz 12ª: `grep -n "fn monthly_multiplier" crates/engine/src/projection.rs`
- Target móvil único: `grep -rn "fn fire_target_at_month_index" crates/engine/src/projection.rs` y sus ≥2 llamantes en `apps/api/src/handlers/projection.rs`
- Overflow tipado: `grep -n "AssetValueOverflow" crates/engine/src/projection.rs` (enum + checked_mul + test)
- Cascada también en jubilación (4.12.1) — **el bucle vive en el núcleo desde 5.0.0 WP5.5, y el grep viejo contra `projection.rs` salía VACÍO**: `grep -n "la MISMA cascada" crates/engine/src/sim_core.rs` (1 hit) y `grep -c "unallocated_savings_total" crates/engine/src/projection.rs crates/engine/src/sim_core.rs` (8 y 4 el 2026-09-03: el tipo público y el bucle)
- Orden total de activos: `grep -rn "sort_index ASC, name ASC, id ASC" apps/api/src/handlers/` (2 hits)
- Paridad tramos altos: `grep -c "tramo" apps/api/tests/fixtures/fire-parity.json` (≥2) y `python3 -c "import json;print(len(json.load(open('apps/api/tests/fixtures/fire-parity.json'))['cases']))"` (≥9)
- Tramos vigentes por defecto: `grep -n "300_000" apps/api/src/handlers/installation.rs` — **el grep anterior (`300000`, sin el separador) llevaba vacío desde siempre**: el literal del código es `Decimal::from(300_000u32)`. Grep vacío = señal, también cuando la señal es que el comando estaba mal escrito
- Freezer f64: `cargo test -p futurefin-engine no_f64 -- --list`
- Predicado único de devengo (#121): `grep -n "pub fn liability_interest_accrues" crates/engine/src/projection.rs` y su espejo `grep -n "liabilityAccruesInterest" apps/web/src/lib/ledger.ts`
- Ley por modelo en el histórico (#129): `grep -n "repayment_model" crates/engine/src/history.rs | head -3`
- LOCF del histórico (#130): `grep -n "last_is_live_ledger" crates/engine/src/history.rs apps/api/src/handlers/history.rs | head -3`
- Comisión de amortización (#151): `grep -n "early_repayment_fee" crates/engine/src/projection.rs | head -3`
- **Número FIRE clásico (§2.4, 5.0.0 E4)**: `grep -n "pub fn fire_target_at_month_index_with_plan\|pub struct PlanFireTarget" crates/engine/src/target.rs` (2 hits) y, para la bit-identidad con 4.15.0, `grep -n "fire_target_at_index_g(self.view(), month_index)" crates/engine/src/target.rs` (1 hit: la ÚNICA rama — desde E4 no hay base alternativa que delegue)
- **Lo que E4 retiró del objetivo, comprobable por su AUSENCIA** (un grep VACÍO es la señal correcta aquí; si alguno devuelve algo, la base puente ha vuelto): `grep -rnE "TargetBasis|BridgeToPension|build_bridge_table|MAX_BRIDGE_MONTHS|bridge_discount_annual_pct|bridge_effective_withdrawal_pct|pension_coverage_ratio|partial_gap_target|BridgeDiscountOverflow" crates/engine/src crates/engine-stochastic/src` — solo debe haber notas históricas en comentarios (`grep -v "^[^:]*:[0-9]*: *//"` deja el resultado vacío)
- **El objetivo NO cae a cero en la pensión**: `grep -n "fn the_classic_fire_number_ignores_the_dated_pension_and_never_drops_to_zero" crates/engine/src/target.rs` (1 hit)
- **Fases y trigger único (§2.5)**: `grep -n "enum Phase\b\|enum RetirementTrigger\|enum SpendMode\|enum WithdrawalRule" crates/engine/src/phases.rs` (4 hits — eran 5 con `TargetBasis`, que E4 retiró) y `grep -n "crossing_is_reading_only" crates/engine/src/sim_core.rs` (3 hits: el mes 1, el bucle y su comentario)
- **Literales estables de los avisos y de los motivos de fallo**: `grep -n -A6 "pub fn code(self)" crates/engine/src/phases.rs` (aviso `partial_phase_capital_shrinking` — único desde E4; motivos `portfolio_depleted`, `initial_rate_exceeded`, `rule_below_need`)
- **Reglas de retirada y sus cotas de motor**: `grep -n "fn allowed_gross\|fn validate_rule\|fn review_guardrails" crates/engine/src/withdrawal.rs` (3 hits)
- **Las tres magnitudes separadas**: `grep -n -A30 "fn account(" crates/engine/src/sim_core.rs` (`undrained` / `shortfall` / `excess`, cada una con su comentario)
- **Solves por bisección sobre el motor**: `grep -n "pub const MAX_SOLVE_ITERATIONS\|fn search_ceiling\|pub fn max_extra_monthly_expense_keeping_date\|pub fn retirement_delay_months\|pub fn run_with_cap\|pub fn run_stopping_at" crates/engine/src/solve.rs` (6 hits desde E4: los dos solves que biseccionaban contra el objetivo se fueron al crate estocástico y los dos motores de escenario se hicieron públicos); la medición de P9 que fijó el techo está en el doc-comment de `search_ceiling`
- **Clamp de publicación del descubierto (§4)**: `grep -n "uncovered_deficit_total.max(Decimal::ZERO)" apps/api/src/handlers/projection.rs` (1 hit, en el handler — **nunca** en el motor)
- **`bool` en el motor, `Option<bool>` en la API (§4)**: `grep -c "pub partial_phase_capital_growing: bool" crates/engine/src/projection.rs` (1) y `grep -c "pub partial_phase_capital_growing: Option<bool>" apps/api/src/handlers/projection.rs` (2: serie y simulate)
- **Frontera f64 (§1)**: `grep -n "pub trait MoneyOps" crates/engine/src/money.rs`,
  `grep -c "impl MoneyOps for F64Money" crates/engine-stochastic/src/lib.rs` (1) y el freezer intacto
  `grep -n "fn crates_engine_src_has_no_f64_outside_comments" crates/engine/src/lib.rs`
- **La puerta de degeneración que sostiene esa frontera**: `grep -n "const EUR_TOLERANCE\|fn every_case_degenerates_from_decimal_to_floating_point" crates/engine-stochastic/tests/degeneration.rs` (2 hits) — 1 € por mes en todo el horizonte, cota relativa declarada solo por encima de 2⁵³ €
- **Pines dorados (§3)**: `grep -n "fn golden_pins_match_4_15_0\|fn golden_pins_5_0_outputs_match" crates/engine/tests/golden_pins.rs` (2 hits); recuento de casos con el `python3 -c` de §3
- La tabla de §4: cada fila con estado «pendiente» debe tener issue ABIERTO (`gh issue view <n>`); si el issue se cierra, la fila se actualiza o se borra en el mismo cambio.
- **Bit-identidad restaurada (§2.2)**: `grep -n "fn p24_publishes_the_undrained_operand_with_the_scale_of_4_15_0\|fn p25_keeps_the_debt_service_grouping_of_4_15_0" crates/engine/tests/golden_pins.rs` (2 hits)
- **`assets_depleted_month_index` de dos condiciones (§2.5)**: `grep -n "fn an_exact_landing_that_covers_every_later_need_is_not_a_depletion" crates/engine/tests/review_fixes.rs`
- **Vía mixta bajo techo (§2.5)**: `grep -n "fn the_binding_allowance_is_a_cut_on_the_mixed_path_too" crates/engine/tests/review_fixes.rs`
- **`rule_is_spend` financiado del superávit (§2.5)**: `grep -n "fn rule_is_spend_funds_the_month_surplus_first" crates/engine/tests/review_fixes.rs`
- **Inversión de monotonía en los solves (§2.5)**: `grep -n "la peor de 3,4416" crates/engine/src/solve.rs`
- **Suite estocástica verde (§4)**: `cargo test -p futurefin-engine-stochastic 2>&1 | grep "test result"` (13 + 3 + 13 = 29, 0 fallos)
