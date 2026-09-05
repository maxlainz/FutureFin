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
- **El colchón de caja es un IMPORTE NOMINAL cuando se deriva del tope de una regla** (5.0.0, V6 y
  P2). Dos convenciones, y confundirlas sobrevalora la protección en silencio:
  `CashBufferTarget::Months(n)` es `n × gasto del mes YA INDEXADO` —el objetivo crece con la
  inflación—, mientras que `CashBufferTarget::Amount(a)` es un euro **nominal fijo que no se indexa
  nunca**, exactamente el mismo que persigue el tope `amount` de la cascada (`resolve_cap_ceiling`).
  El colchón derivado usa `Amount`: **la misma regla gobierna las dos fases** —acumular hasta X y,
  ya jubilado, mantener X—. Convertir el tope a meses a mes 0 y dejarlo indexarse lo revalorizaría
  ~2,4× a 35 años con un 2,5 %; los meses solo se publican como equivalente informativo
  (`buffer_months_effective = floor(tope / gasto de jubilación)`). Puerta:
  `crates/engine-stochastic/tests/monte_carlo.rs::mc_cash_buffer_amount_holds_the_cap` (medido: con
  `Amount(48 000)` el colchón se queda en 48.000 € en todo el horizonte; con `Months(24)` llega a
  113.680 € = 48.000 × 1,025³⁵).

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
- **Objetivo consciente del PLAN (5.0.0)** — `crates/engine/src/target.rs::PlanFireTarget::at`
  (el evaluador de verdad es `PlanTargetG::at`, genérico). Con `plan.pension == None` **llama** al
  objetivo de 4.15.0 (`sim_core::fire_target_at_index_g`) en vez de reproducir su fórmula: la
  bit-identidad es por construcción, no por revisión. La rejilla es **0-based** (`i = k−1`: el bucle
  evalúa su mes `k` contra ese índice) y cada término declara **una sola unidad** — mezclar €/mes con
  €/año dentro de la misma suma fue el hallazgo B1 de la revisión adversarial, y hacía salir el
  puente doce veces mal sin que nada fallara:

  | símbolo | unidad | definición |
  |---|---|---|
  | `E·f(i)` | €/mes | gasto del mes (`PlanFireTarget::expense_monthly_at`) |
  | `I_persist` | €/mes | ingreso PLANO que persiste tras jubilarse (la pensión SIN fecha de 4.15.0) |
  | `P_m(i)` | €/mes | pensión CON fecha: `0` si `i < P`; `monthly_today·f(i)` indexada (default, D8) o `monthly_today` plana |
  | `need_full_m(i)` | €/mes | `max(0, E·f(i) − I_persist)` |
  | `need_net_m(i)` | €/mes | `max(0, E·f(i) − I_persist − P_m(i))` |
  | `T(i)` | € | el objetivo — un STOCK, no un flujo |

  - **`perpetuity`** (default): `T(i) = gross_up(12·need(i))/SWR + deuda(i)`, con `need = need_full_m`
    mientras `i < P` —la pensión todavía no existe y **no se cuenta con ella** (R6, la lectura
    conservadora)— y `need_net_m` desde `P`. Si `need_net_m(i) ≤ 0` la pensión cubre el gasto entero
    y **`T(i) = deuda(i)`, jamás `None`**: un objetivo ausente ahí se leería como «no se jubila
    nunca» cuando la verdad es «se jubila ya» (hallazgo B3).
  - **`bridge_to_pension`** (P2), para `i < P`:
    `T(i) = Σ_{m=i}^{P−1} gross_up_monthly(need_full_m(m))·(1+d)^{−(m−i)/12} + [gross_up(12·need_net_m(P))/SWR]·(1+d)^{−(P−i)/12} + deuda(i)`;
    desde `P` coincide término a término con la perpetuidad neta. Se computa como **suma sufijo**:
    con `q(j) = inflation_factor_at_month_index(d, j)`, `(1+d)^{−(m−i)/12} = q(i)/q(m)`, así que
    `T(i) = q(i)·Σ_m G(m)/q(m)` — `O(P)` una vez por simulación, `O(1)` por evaluación (la suma
    directa sería `O(P²)` con un gross-up y una potencia por término: cientos de miles a 840 meses).
    **Esa forma ES la definición**: en `i = 0`, donde `q(0) = 1` exacto, coincide término a término
    con la suma directa. Nunca por producto acumulado — `powd` enruta los exponentes enteros por
    `checked_powu` y un producto acumulado los desviaría a `exp`/`ln`.
  - **Los dos escenarios de D15 caen solos, ninguno se asume**: si la pensión cubre el 100 % del
    gasto el término perpetuo es **0 exacto** y el objetivo es solo el puente + deuda; si cubre una
    parte, queda la perpetuidad sobre el resto. Lo decide el importe declarado frente al gasto, mes
    a mes.
  - `d = bridge_discount_annual_pct` (D7: `expected_return` | `swr` | `none`, default
    `expected_return`; lo resuelve el handler ponderando la rentabilidad esperada de los activos
    LÍQUIDOS por valor). `d ≤ −100 %` se lee como **sin descuento** — el puente sale MÁS caro, que
    es la dirección conservadora.
  - **`MAX_BRIDGE_MONTHS = 1.200`** (100 años): una pensión declarada más allá degrada a la
    perpetuidad sobre la necesidad ÍNTEGRA. **Esa degradación NO es siempre más prudente** (matiz
    de la revisión D20): medido, el objetivo degradado puede salir MENOR que el puente que
    sustituye (−27 % en el caso del issue con `d = 5 %`; −77 % con `d = 0`). Solo alcanzable con
    una pensión declarada a más de 100 años vista — violación de contrato LATENTE, documentada en
    la constante, no una garantía de «objetivo más grande, nunca menor». El puente degrada también
    con `P = 0` (la pensión ya se cobra), sin SWR positivo o sin necesidad HOY — la misma puerta de
    `i = 0` de §2.4.
  - **`EngineError::BridgeDiscountOverflow`** (5.0.0, revisión adversarial hallazgo #1): con `d`
    muy negativo la base `1 + d/100` se hunde hacia 0 y `q(j) = base^{j/12}` desborda el rango de
    `Decimal` antes de terminar de tabular el puente — sin la puerta, `powd` PANICABA y un solo
    activo con `expected_annual_return_percent: "-50"` bastaba para reventar
    `/v1/projection/series` con un 500 opaco. Ahora `build_bridge_table` detecta el desbordamiento
    (potencia o suma sufijo) y devuelve `None`; una LECTURA suelta del objetivo degrada a la
    perpetuidad (nunca panica), pero una SIMULACIÓN que dependiera de ese puente fallaría en voz
    alta con `BridgeDiscountOverflow` en vez de publicar un plan distinto del configurado. La cota
    alcanzable de `d` depende de `P` (`crates/engine/src/target.rs::build_bridge_table`): **−99,6 %
    a 10 años, −86,6 % a 27, −53,8 % a 70, −41,8 % en `MAX_BRIDGE_MONTHS`** — el rango practicable
    se estrecha con el horizonte del puente porque la suma sufijo desborda antes que la propia
    potencia.
  - Lecturas publicadas, **cada una con su unidad declarada**, y ningún `null` que signifique cero:
    `bridge_effective_withdrawal_pct` = `100·12·need_full_m(R−1)/L(R−1)` en **% ANUAL** (`None` sin
    puente, sin jubilación dentro del horizonte o con `L(R−1) ≤ 0`); `pension_coverage_ratio` =
    `P_m(P)/(E·f(P))` en **FRACCIÓN** (0,6 = 60 %); `partial_gap_target` =
    `gross_up(12·gap_m(X−1))/SWR` en **€**, con
    `gap_m = max(0, E_basis·f − income_partial − P_m·fraction_while_partial)` — informativo, no
    dispara nada; `Some(0)` = la media jornada se paga sola.

### 2.5 Jubilación — motor por FASES (5.0.0)

Desde 5.0.0 la jubilación deja de ser un evento del hogar y pasa a ser una **estrategia por usuario**
(`users.retirement_profile`) que decide cuatro cosas a la vez: el disparador, la base del objetivo,
las fases y la regla de retirada. El motor las ejecuta como un `PhasePlan`
(`crates/engine/src/phases.rs`), consumido por el bucle (`sim_core::simulate`) y por
`first_month_allocation` — que hasta 4.15.0 duplicaban el mismo `if` con dos redacciones distintas.

**Fases**, latch **monótono** `Accumulating → (Partial) → Retired` (#141 generalizado): una vez
avanzada no se vuelve atrás, ni porque el patrimonio caiga un mes por debajo del objetivo inflado.

**Las cinco estrategias** (§C del plan de #207; quien las traduce a `PhasePlan` es
`apps/api/src/handlers/projection.rs`, no el motor):

| Estrategia | Trigger | Objetivo | Aportación simulada | Lecturas propias |
|---|---|---|---|---|
| `asap` | cruce del líquido | `perpetuity` / `bridge` (R6) | toda la cascada | `liquid_crossing_month_index`, series `withdrawal_*` |
| `retire_at_age` | `AtMonth(R)` | `T(R−1)` | toda la cascada (D16) | `required_contribution_monthly` + su techo de búsqueda, `required_capital_path`, `disposable_*`, `underfunded` |
| `coast` | `AtMonth(R)` | `T(R−1)` | toda la cascada | `coast_fire_month_index`, `coast_number`, `coast_path` |
| `partial` | parcial en `AtMonth(X)`; total por cruce o `AtMonth(R)` si hay `R` | perpetuity/bridge; `partial_gap_target` informativo | toda la cascada | `partial_retirement_month_index`, `partial_gap_target`, `partial_phase_capital_growing` |
| `pension_bridge` | cruce del líquido | **`bridge_to_pension`** forzado | toda la cascada | `bridge_effective_withdrawal_pct`, `pension_coverage_ratio`, `pension_start_month_index` |

- **Un solo trigger por simulación (D17) — y lo impone la ESTRATEGIA, no el motor.** El bucle
  conserva la UNIÓN de 4.15.0 (`cruce || k ≥ mes forzado`, o sea `min(cruce, R)`) porque es lo que el
  pin dorado tiene fotografiado; el eje que apaga el cruce es
  `PhasePlan::crossing_is_reading_only`: con `true` el cruce **no jubila**, solo se anota como
  `liquid_crossing_month_index`. Existe porque las estrategias por edad **siguen necesitando el
  objetivo** —el chart lo pinta y el infra-financiado se mide contra él—, así que pasarle
  `fire_target: None` al motor lo habría desactivado tirando también la lectura.
- `retirement_month_index` es el mes **EFECTIVO** (1-based) y es lo que la API publica como
  `jubilacion_month_index` (R8); `liquid_crossing_month_index` es el cruce puro, evaluado TODOS los
  meses —también después de que el latch cierre— y **no gobierna nada**.
- **La edad manda** (D17): en `retire_at_age`/`coast` el hogar se jubila en `R` aunque el capital no
  llegue, y el motor emite `EngineWarning::RetireAtAgeUnderfunded` (literal público
  `retire_at_age_underfunded`) **mirando el objetivo, no el trigger**: si quien jubiló fue el cruce,
  `L(R−1) ≥ T(R−1)` por definición y esa rama no puede darse.

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

La serie `unmet_need` es la tercera magnitud publicada mes a mes; sin ella el reparto solo cerraba
cuando la venta se fundaba entera, y **cualquier cociente de cobertura mentía justo en el caso que
importa**: con `fixed_real` el recorte es cero por construcción, así que
`withdrawal_to_need_ratio` valía 1,0 («la regla cubrió el 100 %») en 1.000 caminos de un hogar que
cubrió el 8,7 % de su gasto (hallazgo #4 de la revisión).

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

**Los solves — inversas por bisección sobre el MOTOR ENTERO** (`crates/engine/src/solve.rs`,
`MAX_SOLVE_ITERATIONS = 24`, una `project_net_worth_series` completa por evaluación). No hay forma
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

- `required_contribution_monthly`: la menor aportación mensual constante con `líquido(R−1) ≥ T(R−1)`.
  Es un **TECHO** sobre lo que la cascada invierte cada mes, no un importe que se aporte pase lo que
  pase: en un mes con menos sobrante se aporta el sobrante (R5).
- **El techo de búsqueda es el MÁXIMO SOBRANTE MENSUAL del horizonte**, no el neto recurrente del mes
  1 que R5 dejaba abierto — decidido con la medición delante: sobre el caso P9 el neto del mes 1 son
  500 €/mes, y a 600 meses la ejecución con ese techo cierra en **91.444 €** frente a **725.197 €**
  sin techo. Con la cota de R5, `underfunded` se encendería en hogares cuya simulación REAL sí llega:
  un rojo falso de D17. El sobrante del mes 1 se conserva como SUELO de la cota y se publica
  (`search_ceiling`) para no obligar al llamante a deducirlo.
- `underfunded = true` ⟺ ni invirtiendo cada euro de sobrante se alcanza el objetivo. **No es un
  error**: la simulación existe y se publica, en rojo.
- `required_capital_path` y `coast_path` son **series líquidas SIMULADAS** de esas ejecuciones, no
  curvas dibujadas aparte: `disponible(k) = líquido_real(k) − required_capital_path(k)`.
- `coast_fire_month_index`: primer mes desde el que se puede dejar de aportar y aun así alcanzar
  `T(R−1)`. El **número coast** (`coast_number`) es el líquido con el que se **ENTRA** en ese mes
  (`coast_path[coast−1]`, el cierre del anterior). Sin coast alcanzable se emite
  `coast_not_reachable` y `coast_path` es la mejor ejecución que el plan da (aportando siempre).
- `max_extra_monthly_expense_keeping_date` (P8.b) sube **solo `expense_regular_monthly`** — ni el
  gasto de jubilación ni la necesidad que el objetivo capitaliza: la pregunta es «¿cuánto margen
  tengo AHORA?», no «¿cuánto puedo subir mi nivel de vida para siempre?». Con un trigger por EDAD
  —que no depende del gasto— devuelve la cota como **suelo honesto**, nunca un infinito inventado.
- `retirement_delay_months` (P8.c): dos simulaciones, sin bisección; `delay_months = null` cuando
  cualquiera de los dos escenarios no se jubila dentro del horizonte — «la pausa te saca del
  horizonte» es una respuesta, pero no es un número de meses.

**Techo de aportación y margen disponible.** `contribution_cap_monthly` (la palanca de
`required_contribution_monthly`) y `contributions_stop_month` (la de `coast_fire_month_index`)
recortan a `min(sobrante, c)` el pool que llega a la cascada; el resto **no se invierte, no compone y
no entra en `net_worth`**: sale del balance y se publica en `disposable_cash` — el mismo trato que
`unallocated_savings_total` y por la misma razón (el modelo no simula un euro sin destino declarado).
Identidad del mes con sobrante > 0: `sobrante = Σ aportado + no_asignado + disposable`. Sin techo es
cero mes a mes y no se ejecuta ni una operación de más (bit-identidad).

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
- **`success_verdict`: el corte es FIJO al 100 %** (5.0.0, decisión V7 del owner). Verde ⟺
  `success_probability == 1`, o sea **ni un camino agota la cartera**; ámbar en `[0,90, 1)`; rojo
  por debajo de 0,90. El borde es EXACTO y no necesita épsilon: la probabilidad es `n/n` con `n`
  caminos enteros y en IEEE 754 esa división da `1.0` para cualquier `n`. El
  `success_threshold_pct` configurable del perfil se retiró de la entrada útil (se acepta y se
  ignora) y de **toda** la salida: un umbral por persona hacía incomparables dos veredictos del
  mismo número. Consecuencia asumida: con 500 caminos, **un solo fallo ya es ámbar**.
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
| **Éxito de Monte Carlo = el plan OCURRE y AGUANTA** (D22 corregida por la revisión D20): jubilarse dentro del horizonte —o tener un trigger por edad— **y** no agotar la cartera | Baja la probabilidad publicada donde el cruce es tardío: medido 0,960 → 0,629 en un hogar que cruza en el mes 655 de 840 | La definición anterior («la cartera no se agota nunca») premiaba al hogar que **no se jubila jamás**: quien nunca drena nunca se agota. El 33,1 % de los caminos de ese hogar no llegaba a jubilarse y los 1.000 contaban como éxito; el sesgo llegaba a **+6,8 pp** con SWR 6 %. `never_retired_probability` y `success_given_retired` se publican al lado para separar «¿ocurre?» de «¿aguanta?» |
| **El colchón de caja (P4) se rellena con el shock del mes ANTERIOR y exige un líquido a σ = 0** | Con el colchón a la rentabilidad de la cartera el éxito SUBE +3,9 pp; con la cuenta al 0 % sigue costando −3,5 pp | Autorizar el relleno con el `z` del propio mes —y ejecutarlo antes del crecimiento— vendía renta variable al precio de antes de una subida que ya se conocía: información del futuro, y cara (−2,5 pp, con 249 caminos arruinados solo bajo esa regla y ninguno bajo la retardada). Y elegir el colchón por el orden de drenaje sin mirar σ ponía el «colchón» en la renta variable, o vendía la vivienda para llenarlo. Lo que cuesta es el **lastre** de tener 24 meses de gasto fuera del mercado, no la política: la ayuda de la UI tiene que decirlo así |
| **Guyton-Klinger sin la *portfolio management rule* (ventana de 15 años) ni la *inflation rule*** (saltarse la subida por IPC del año siguiente a un recorte) | Modelo **más reactivo**: recorta antes y más veces que el artículo de 2006 | Las dos omitidas SUAVIZAN la regla; omitirlas va en la dirección prudente. Declarado en `withdrawal.rs::review_guardrails` y en el `helpTexts` de la regla, para que nadie lo descubra comparando con el artículo |
| **Un solo shock de mercado común por mes, escalado por la sd de cada activo** (D11), en vez de una matriz de correlaciones | Subestima la diversificación entre clases: las bandas salen **más anchas** de lo que daría una correlación < 1 | Una matriz de correlación exige datos que la instalación no tiene (el usuario declara μ y σ por activo, no covarianzas); inventarlas sería falsa precisión, y el sesgo es conservador. **Simulado desde WP6a** (commit `ba6bdfe`, 2026-09-03): `engine_stochastic::project_percentile_bands` inyecta por mes `f_ik = m_i·exp(σ_i·z_k − σ_i²/2)` (un solo `z_k` por mes para toda la cartera; `E[f] = m_i` exacto; `σ = 0` ⇒ `m_i` por rama explícita) sobre el MISMO bucle genérico. **La sd NO viaja en `SimAsset`**: se pasa como slice alineado a `assets[]`, así que el camino `Decimal` la ignora por construcción y su bit-identidad no depende de nadie. (La suite del crate está en VERDE desde el pase de correcciones de la revisión D20; el test que fallaba se rehízo como `mc_cash_buffer_protects_and_the_drag_is_what_costs`.) |
| **`partial_phase_capital_growing`: `bool` en el motor, `Option<bool>` en la API** | 0 € | El motor es una función pura y debe definir el estado (sin fase parcial ⇒ `false`); el wire no puede darle el mismo valor a «no hubo media jornada» y a «hubo y menguó», así que la capa que serializa lo convierte en `null` mirando `partial_retirement_month_index`. Verificado en `apps/api/src/handlers/projection.rs` |
| **Cola de redondeo negativa de `uncovered_deficit_total` clampada al PUBLICAR, no en el motor** | medido hasta ≈ −1,7·10⁻²⁴ € (y hasta +5,6·10⁻²³ en el corpus diferencial) | El descubierto se acumula como residuo de ventas brutas y puede salir con una cola negativa que no es «−0,0000000000000000000000005 € descubiertos», es cero. El motor debe seguir publicando su aritmética tal cual —el golden la hashea—; quien redondea para un humano es la capa que serializa (`money_out(… .max(ZERO))`) |
| **La sd del activo no llega al motor determinista** | 0 € en el camino `Decimal` | Por diseño: la volatilidad **no es un campo de `SimAsset`** — viaja como argumento del evaluador estocástico, así que el camino exacto no puede verla y su bit-identidad con 4.15.0 no depende de una rama que alguien pueda tocar. De ese camino no sale un euro (§1) |

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

**Ampliado el 2026-09-05 (WP-F del tren 5.0.0, decisiones V6/V7)**: §2.3 gana el contrato del
colchón derivado (`CashBufferTarget::Amount` es NOMINAL y no se indexa; los meses solo se publican
como equivalente informativo) y §2.7 el veredicto de corte fijo al 100 %. Re-verificación:
`grep -n "pub enum CashBufferTarget" -A4 crates/engine/src/sim.rs`,
`grep -n "CashBufferTarget::Months(n) => n \* expense" crates/engine/src/sim_core.rs` (1 hit),
`grep -n "pub(crate) fn resolve_cash_buffer" apps/api/src/handlers/cash_buffer.rs`,
`grep -n "VERDICT_GREEN_FLOOR_PCT" apps/api/src/handlers/projection_bands.rs` (2 hits) y las dos
puertas: `cargo test -p futurefin-engine-stochastic --test monte_carlo -- mc_cash_buffer_amount_holds_the_cap`
y `cargo test -p futurefin-api --lib projection_bands::tests::el_verde_exige_todos_los_caminos`.

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
- **Objetivo consciente del plan (§2.4, 5.0.0)**: `grep -n "pub fn fire_target_at_month_index_with_plan\|pub struct PlanFireTarget" crates/engine/src/target.rs` (2 hits) y, para la bit-identidad sin pensión, `grep -n "fire_target_at_index_g(Some(ft), month_index)" crates/engine/src/target.rs` (2 hits: las dos ramas que delegan en el objetivo de 4.15.0)
- **Puente en forma sufijo, no suma llana**: `grep -n "fn build_bridge_table" crates/engine/src/target.rs` y `grep -n "suffix\[m\] = discounted" crates/engine/src/target.rs` (la recurrencia `T(i) = G(i)/q(i) + T(i+1)`)
- **Cota del puente y su degradación prudente**: `grep -n "pub const MAX_BRIDGE_MONTHS" crates/engine/src/target.rs` (1.200)
- **`need_net ≤ 0 ⇒ target = deuda`, nunca `None`**: `grep -n -B4 "return Some(debt);" crates/engine/src/target.rs`
- **Lecturas del puente con su unidad**: `grep -n "fn pension_coverage_ratio\|fn partial_gap_target" crates/engine/src/target.rs` (4 hits: núcleo + cara pública) y `grep -n "bridge_effective_withdrawal_pct" crates/engine/src/sim_core.rs` (2 hits)
- **Fases y trigger único (§2.5)**: `grep -n "enum Phase\b\|enum RetirementTrigger\|enum SpendMode\|enum WithdrawalRule\|enum TargetBasis" crates/engine/src/phases.rs` (5 hits) y `grep -n "crossing_is_reading_only" crates/engine/src/sim_core.rs` (3 hits: el mes 1, el bucle y su comentario)
- **Literales estables de los avisos**: `grep -n -A6 "pub fn code(self)" crates/engine/src/phases.rs` (`retire_at_age_underfunded`, `coast_not_reachable`, `partial_phase_capital_shrinking`)
- **Reglas de retirada y sus cotas de motor**: `grep -n "fn allowed_gross\|fn validate_rule\|fn review_guardrails" crates/engine/src/withdrawal.rs` (3 hits)
- **Las tres magnitudes separadas**: `grep -n -A30 "fn account(" crates/engine/src/sim_core.rs` (`undrained` / `shortfall` / `excess`, cada una con su comentario)
- **Solves por bisección sobre el motor**: `grep -n "pub const MAX_SOLVE_ITERATIONS\|fn search_ceiling\|pub fn required_contribution_monthly\|pub fn coast_fire_month_index" crates/engine/src/solve.rs` (4 hits); la medición de P9 que fijó el techo está en el doc-comment de `search_ceiling`
- **Clamp de publicación del descubierto (§4)**: `grep -n "uncovered_deficit_total.max(Decimal::ZERO)" apps/api/src/handlers/projection.rs` (1 hit, en el handler — **nunca** en el motor)
- **`bool` en el motor, `Option<bool>` en la API (§4)**: `grep -c "pub partial_phase_capital_growing: bool" crates/engine/src/projection.rs` (1) y `grep -c "pub partial_phase_capital_growing: Option<bool>" apps/api/src/handlers/projection.rs` (2: serie y simulate)
- **Frontera f64 (§1)**: `grep -n "pub trait MoneyOps" crates/engine/src/money.rs`,
  `grep -c "impl MoneyOps for F64Money" crates/engine-stochastic/src/lib.rs` (1) y el freezer intacto
  `grep -n "fn crates_engine_src_has_no_f64_outside_comments" crates/engine/src/lib.rs`
- **La puerta de degeneración que sostiene esa frontera**: `grep -n "const EUR_TOLERANCE\|fn every_case_degenerates_from_decimal_to_floating_point" crates/engine-stochastic/tests/degeneration.rs` (2 hits) — 1 € por mes en todo el horizonte, cota relativa declarada solo por encima de 2⁵³ €
- **Pines dorados (§3)**: `grep -n "fn golden_pins_match_4_15_0\|fn golden_pins_5_0_outputs_match" crates/engine/tests/golden_pins.rs` (2 hits); recuento de casos con el `python3 -c` de §3
- La tabla de §4: cada fila con estado «pendiente» debe tener issue ABIERTO (`gh issue view <n>`); si el issue se cierra, la fila se actualiza o se borra en el mismo cambio.
- **`BridgeDiscountOverflow` y la tabla de `d` alcanzable (§2.4)**: `grep -n "BridgeDiscountOverflow" crates/engine/src/{projection,sim_core,target}.rs` (5 hits: enum + 1 `return Err` + 3 doc-comments) y `grep -n "cota depende de \`P\`" crates/engine/src/target.rs`
- **Bit-identidad restaurada (§2.2)**: `grep -n "fn p24_publishes_the_undrained_operand_with_the_scale_of_4_15_0\|fn p25_keeps_the_debt_service_grouping_of_4_15_0" crates/engine/tests/golden_pins.rs` (2 hits)
- **`assets_depleted_month_index` de dos condiciones (§2.5)**: `grep -n "fn an_exact_landing_that_covers_every_later_need_is_not_a_depletion" crates/engine/tests/review_fixes.rs`
- **Vía mixta bajo techo (§2.5)**: `grep -n "fn the_binding_allowance_is_a_cut_on_the_mixed_path_too" crates/engine/tests/review_fixes.rs`
- **`rule_is_spend` financiado del superávit (§2.5)**: `grep -n "fn rule_is_spend_funds_the_month_surplus_first" crates/engine/tests/review_fixes.rs`
- **Inversión de monotonía en los solves (§2.5)**: `grep -n "la peor de 3,4416" crates/engine/src/solve.rs`
- **Suite estocástica verde (§4)**: `cargo test -p futurefin-engine-stochastic 2>&1 | grep "test result"` (13 + 3 + 13 = 29, 0 fallos)
