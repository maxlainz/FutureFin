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
| Tipos de interés de pasivos | `apr_percent` = **TIN nominal anual** en puntos (3 = 3 %/año); tipo mensual `i = apr/1200` | Idéntico en proyección (`liability_month`) e histórico (`LoanTerms`) — la misma curva a ambos lados de «hoy». ⚠ la UI lo etiqueta «TAE» (§4) |
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
- **Degeneraciones**: TIN ausente/≤0 ⇒ cualquier modelo colapsa a `fixed_payments`;
  `fixed_payments` (el **default** de la columna) no devenga jamás; `interest_only` congela el
  principal con cuota arbitraria; plan vencido con saldo vivo ⇒ resta constante congelada. Las
  cuatro son divergencias con la realidad (§4: D13, D15).
- **Actividad**: `monthly_payment > 0 AND (payment_end IS NULL OR >= inicio de mes)` — predicado
  único `liability_active`, compartido por caja, amortización y devengo.

### 2.2 Capital
- Crecimiento **después** de los flujos del mes (aportación cobra el mes completo);
  `values[i] = values[i].checked_mul(m)` — desbordar es error tipado `AssetValueOverflow`, nunca
  panic ni saturación silenciosa.
- Drenaje en déficit: `surplus_cash` primero; luego TODOS los activos — líquidos primero, dentro
  de cada grupo menor rentabilidad primero, desempate por índice de entrada (orden de entrada
  total: `ORDER BY sort_index, name, id`). Lo no cubierto se acumula en `undrained_cumulative` y
  RESTA del patrimonio: la curva puede ser negativa y no se aplana — correcto.
- `surplus_cash` rinde 0 % — realista para cuenta corriente española (~0,15 % TEDR), pero es
  invisible e ilimitado (§4: D10, decisión: regla remainder obligatoria).

### 2.3 Caja y asignación
- Orden del mes: servicio de deuda → estado de jubilación (NW(k−1) vs target(k−1)) → caja neta →
  (drenaje | acumulación en jubilación | cascada) → crecimiento → asiento de principales → NW.
- Cascada: `fixed`/`percent` (sobre el restante del paso)/`remainder`, caps a techo absoluto sobre
  el valor VIVO del activo; conservación exacta `Σ per_asset + leftover = base_cash` (pinneada en
  `allocation_resolution.rs`). En jubilación la cascada NO corre: `first_month_allocation` lo
  declara con `skipped_reason: in_retirement` (auditoría 2026-08).
- Modos de ahorro: A (presupuesto), B (promedio real ambos lados), C (ingreso plan + gasto real);
  fallback por lado. En B/C la cuota vive dentro del promedio (decisión explícita del owner) y el
  principal se congela — la parte «para siempre» es divergencia (§4: D17, decidida).

### 2.4 FIRE y fiscalidad
- `target_base = gross_up(need_annual)/(swr/100)`; target del mes k = `base·(1+i/100)^(k/12)`
  (único helper `fire_target_at_month_index`, compartido engine/handler). Cruce: `NW(k−1) ≥
  target(k−1)`.
- Gross-up: forma cerrada por tramos (escala **marginal**), tramos por defecto = escala del ahorro
  VIGENTE 2025-26 (19/21/23/27/30 @ 6k/50k/200k/300k — Ley 7/2024). Paridad Rust↔TS por
  `fire-parity.json` (9 casos, tramos 27 % y 30 % incluidos desde 2026-08).
- La base imponible que asume es el **reembolso íntegro**; la ley grava solo la plusvalía con FIFO
  y diferimiento (LIRPF arts. 33/37/94) — §4: D4, decidido: fracción g por fases.
- Fiscalidad de fondos: la rentabilidad publicada de un fondo YA es neta de TER/transacción
  (RD 1082/2012 art. 5; CNMV); los traspasos entre fondos están exentos (art. 94) — por eso «sin
  rebalanceo» es carencia funcional, no fiscal.

### 2.5 Jubilación
- Disparador único: cruce patrimonial (el trigger por edad está vetado — failure-archaeology).
  Sin latch hoy: reversible mes a mes (§4: D7, decidido: absorbente).
- Tras el cruce: `income_retirement` (partidas `persists_after_retirement`) y `expense_retirement`
  (partidas `!ends_at_retirement`), **del presupuesto en los 3 modos**, congeladas en nominal; el
  superávit va a `surplus_cash`; la retirada NO tributa (asimetría con el target — §4: D5, decidido).

### 2.6 Histórico
- Interpolación entre snapshots: activos lineal en días civiles; pasivos amortización francesa
  «corregida por residuo» (exacta en los extremos pese a `powd`), con fallback lineal en
  degenerados. El mes 0 se evalúa en `today` real.
- Los snapshots JAMÁS son inputs del engine de proyección (D12 de arquitectura). El empalme con la
  proyección es solo del frontend (`history-merge.ts`, anclas iguales o identidad).
- Divergencia declarada: un pasivo `fixed_payments` tiene pasado francés y futuro lineal (§4: D30).

### 2.7 KPIs
- `net_return`: expectativa (no realizado), ponderada por valor; resta la TAE de TODOS los pasivos
  vivos mientras el engine solo devenga french/revolving con plan activo — deliberadamente más
  prudente, declarado en su texto de ayuda. Real por Fisher (división de factores, no resta).
- `runway`: retirada-antes-de-crecimiento, multiplicador ponderado por valor (aprox. conservadora
  del drenaje real), «indefinido» ⟺ umbral SWR sobre el saldo líquido; 1200 meses es SUELO.
  Incoherencias con la simulación en §4 (D29).
- El contrato en prosa de cada métrica vive en `apps/web/src/lib/helpTexts.ts`
  (skill `futurefin-metric-definitions`).

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

<!-- AUDIT-ISSUES-TABLE -->

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

- Devengo francés: `grep -n "payoff = P·(1 + i)" -r crates/engine/src/ || grep -n "fn liability_month" crates/engine/src/projection.rs`
- Convención TIN/1200 compartida: `grep -rn "1200" crates/engine/src/{projection,history}.rs | grep -c "apr"` (≥2)
- Raíz 12ª: `grep -n "fn monthly_multiplier" crates/engine/src/projection.rs`
- Target móvil único: `grep -rn "fn fire_target_at_month_index" crates/engine/src/projection.rs` y sus ≥2 llamantes en `apps/api/src/handlers/projection.rs`
- Overflow tipado: `grep -n "AssetValueOverflow" crates/engine/src/projection.rs` (enum + checked_mul + test)
- Cascada en jubilación declarada: `grep -n "InRetirement" crates/engine/src/projection.rs apps/api/src/handlers/allocation_rules.rs`
- Orden total de activos: `grep -rn "sort_index ASC, name ASC, id ASC" apps/api/src/handlers/` (2 hits)
- Paridad tramos altos: `grep -c "tramo" apps/api/tests/fixtures/fire-parity.json` (≥2) y `python3 -c "import json;print(len(json.load(open('apps/api/tests/fixtures/fire-parity.json'))['cases']))"` (≥9)
- Tramos vigentes por defecto: `grep -n "300000" apps/api/src/handlers/installation.rs`
- Freezer f64: `cargo test -p futurefin-engine no_f64 -- --list`
- La tabla de §4: cada fila con estado «pendiente» debe tener issue ABIERTO (`gh issue view <n>`); si el issue se cierra, la fila se actualiza o se borra en el mismo cambio.
