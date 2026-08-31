# Projection Engine (crates/engine)

> Este doc describe el CÓMO (API pública y bucle). El QUÉ de cada magnitud — unidad, convención,
> por qué refleja (o no) la realidad española, y las divergencias conocidas con su issue — vive en
> [`financial-contracts.md`](financial-contracts.md) (auditoría 2026-08).

Pure Rust crate — no I/O, no DB, no async. Pure financial math (projection + history interpolation).
Only `Decimal` arithmetic. Four modules:
- `projection.rs` — monthly net-worth / FIRE simulation (this doc's main subject).
- `history.rs` — pure interpolation of the **historical** net-worth series from manual snapshots
  (see [History interpolation](#history-interpolation-historyrs) below). Deps unchanged
  (`rust_decimal` feature `maths` already present for `powd`).
- `net_return.rs` — expected annual net return of net worth (`net_return_percentages`; see
  [Rendimiento neto](#rendimiento-neto-net_returnrs) below). Consumed by `GET /v1/summary`.
- `runway.rs` — liquidity runway with compounded return + inflation (v2.2.0; **SWR threshold for the
  infinite case** since v2.3.0 — `Indefinite` ⟺ the grossed-up annual withdrawal fits inside
  `swr_pct` × liquid balance; see [Runway](#runway-runwayrs) below). Consumed by `GET /v1/summary`.

## Public API

```rust
// Main projection: returns net_worth and contributed_capital series (len = horizon_months + 1, index 0 = today)
pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError>

// Returns nominal contributions routed to each asset in the FIRST simulated month only.
// Thin wrapper over `first_month_allocation` since 3.8.0 — kept because `GET /v1/assets` uses it.
pub fn first_month_per_asset_contribution_nominals(input: &ProjectionInput) -> Result<Vec<Decimal>, EngineError>

// Full resolution of the FIRST month's cascade (3.8.0): what gets distributed, where it comes
// from, what no rule absorbed, and a per-rule trace. Added because the old function returned only
// `per_asset` and threw away both the `leftover` (already computed) and the base — which made it
// impossible to explain why the month-1 contribution does not match the summary's recurring net.
// The gap is `planning_component`, and it is also why that number CHANGES EVERY DAY.
pub struct FirstMonthAllocation {
    pub per_asset: Vec<Decimal>,
    pub base_cash: Decimal,            // what the cascade really distributes (`net_cash_month`)
    pub recurring_net: Decimal,        // income − expense − debt_service (stable en el camino de lectura)
    pub planning_component: Decimal,   // planning_adjustment[0] − retirement_withdrawal (transient)
    pub debt_service: Decimal,         // 4.4.0: incluye la amortización extra del mes 1 (0 en el camino de lectura)
    pub leftover: Decimal,             // ends up in `surplus_cash`
    pub rules: Vec<RuleOutcome>,
}
pub fn first_month_allocation(input: &ProjectionInput) -> Result<FirstMonthAllocation, EngineError>
// 4.0.0 — resuelve el estado del mes 1 EXACTAMENTE como el bucle de simulación: si el patrimonio
// de partida (Σ activos − Σ principales) ya cruza `fire_target_at_month_index(fire_target, 0)`,
// usa ingreso y gasto DE JUBILACIÓN y el retiro mensual, igual que hace `project_net_worth_series`.
// Antes solo miraba `retirement_start_month` e ignoraba `fire_target`, así que en un hogar ya por
// encima de su número FIRE `GET /v1/assets` y `/v1/allocation-rules/resolution` publicaban una
// aportación CON EL SIGNO CONTRARIO al de la proyección —«aportas 2.000 €/mes» sobre un activo que
// la simulación reduce ese mismo mes— y explicaban regla a regla una cascada que no se ejecuta
// jamás. Sostenido en todo el horizonte, y no es un caso raro: es el estado final del público al
// que sirve la app.

// Per-rule trace. `amount_intent` vs `amount_resolved` separates "trimmed by a cap" (not a skip,
// and the most-asked question) from "skipped". Skip reasons are deliberately NOT collapsed —
// they have different remedies: NoCash = "you have no surplus" (touch income/expense);
// NotReached = "the rules above ate it" (touch priorities/caps); CapFull = "the target asset is
// at its ceiling"; ZeroAmount = "the rule resolves to 0"; InvalidTarget = defensive.
pub struct RuleOutcome {
    pub rule_index: usize,             // the engine knows no UUIDs; the handler maps identity
    pub target_index: usize,
    pub amount_intent: Decimal,
    pub amount_resolved: Decimal,
    pub cap_ceiling: Option<Decimal>,
    pub cap_room: Option<Decimal>,
    pub skipped_reason: Option<AllocationSkipReason>,
}

// Único helper para evaluar el target FIRE inflado en un `month_index` dado (0 = punto de
// partida, 12 = un año después). Lo consumen tanto el motor (para `fire_reached`) como el
// handler (para construir `fire_target_series`). Antes había una fórmula duplicada — el motor
// usaba `years = (k-1)/12` y el handler `years = month_index/12`, lo que generaba un off-by-one
// de un mes entre cuándo se disparaba la jubilación y la serie pintada en el chart.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal>

// Liquidity runway (v2.2.0): months the liquid assets cover the monthly expense, compounding the
// assets' expected return and inflating the expense. See the Runway section below.
// NOT an infinity sentinel (v2.3.0): the finite loop's cap. Surviving it returns `Months(1200)`,
// a FLOOR ("at least 100 years"); only the SWR threshold yields `Indefinite`.
pub const MAX_RUNWAY_MONTHS: u32 = 1200;
pub enum RunwayOutcome { Months(Decimal), Indefinite, NoExpenseBase }
pub fn liquid_runway_months(
    liquid_assets: &[(Decimal, Option<Decimal>)], // (current_value, expected_annual_return_percent)
    monthly_expense: Decimal,
    annual_inflation_percent: Decimal,
    swr_pct: Decimal,              // installation fire_settings.swr_pct (%), v2.3.0
    annual_expense_for_swr: Decimal, // ANNUAL expense already grossed up by the handler, v2.3.0
) -> RunwayOutcome

// Expected annual net return of net worth (percent). `None` ⟺ net worth ≤ 0.
pub struct NetReturn { pub nominal_pct: Decimal, pub real_pct: Decimal }
pub fn net_return_percentages(
    assets: &[(Decimal, Option<Decimal>)],      // (current_value, expected_annual_return_percent)
    liabilities: &[(Decimal, Option<Decimal>)], // (principal, apr_percent)
    annual_inflation_percent: Decimal,
) -> Option<NetReturn>

// Calendario de amortización de UN pasivo (4.4.0, Fase 6). NO es matemática nueva: publica el
// `closing_principal` que el bucle de simulación ya derivaba hasta 840 veces por request y tiraba
// (`ProjectionOutput` nunca lo expuso), así que «¿cuánto interés pago?» y «¿cuándo termino?» eran
// incontestables desde fuera del motor. Pura y determinista; `horizon_months` se CLAMPA a 1..=840
// (a diferencia de `project_net_worth_series`, que con < 1 devuelve `EngineError::InvalidHorizon`).
pub const MAX_LIABILITY_SCHEDULE_MONTHS: u32 = 840;
pub fn liability_amortization_schedule(
    liab: &ProjectionLiabilityInput,
    ref_date: NaiveDate,
    horizon_months: u32,
) -> LiabilitySchedule

pub struct LiabilityScheduleMonth {
    pub month_index: u32,            // 1-based desde month_first_calendar(ref_date). NO es índice de array.
    pub opening_principal: Decimal,  // saldo al abrir, antes del devengo
    pub interest_accrued: Decimal,   // RESIDUO: payment − (opening − closing_tras_cuota). Ver abajo.
    pub principal_repaid: Decimal,   // opening − closing (cuota + extra). PUEDE SER NEGATIVO.
    pub extra_principal: Decimal,    // la parte de `principal_repaid` que viene del what-if
    pub payment: Decimal,            // caja de la CUOTA, topada al saldo de cancelación del mes
                                     // (`payoff = P(1+i)` en french/revolving; el principal en
                                     // fixed_payments/interest_only). No incluye `extra_principal`.
    pub closing_principal: Decimal,  // nunca negativo
}

pub struct LiabilitySchedule {
    pub months: Vec<LiabilityScheduleMonth>, // vacío ⟺ no había plan activo (o principal ya 0)
    pub opening_principal: Decimal,          // saldo de partida (mes 0)
    pub final_principal: Decimal,
    pub total_interest: Decimal,             // interés que queda POR PAGAR desde hoy, no el del préstamo original
    pub total_payments: Decimal,             // Σ payment (solo cuotas)
    pub total_extra_principal: Decimal,
    pub total_cash_out: Decimal,             // total_payments + total_extra_principal
    pub payoff_month_index: Option<u32>,     // Some(0) ⟺ ya saldado hoy
    pub payoff_absent: Option<LiabilityPayoffAbsence>, // invariante: exactamente uno de los dos
    pub horizon_months: u32,                 // la COTA tras el clamp, no len(months)
}

// Cuatro variantes porque tienen remedios distintos: no colapsarlas es el mismo criterio que
// `AllocationSkipReason`. El motor no conoce literales de wire — los mapea `payoff_absence_code`
// en `apps/api/src/handlers/liabilities.rs`.
pub enum LiabilityPayoffAbsence {
    NoPaymentPlan,
    PaymentPlanEndsBeforePayoff,
    PaymentDoesNotReducePrincipal,
    NotWithinHorizon,
}
```

## ProjectionInput fields
```rust
pub struct ProjectionInput {
    pub ref_date: NaiveDate,           // Civil "today" from installation calendar_tz
    pub horizon_months: u32,           // >= 1
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    pub allocation_rules: Vec<AllocationRule>,   // cascade, in priority order
    pub liabilities: Vec<ProjectionLiabilityInput>,   // see per-mode contract note below
    pub planning_monthly_cash_adjustment: Vec<Decimal>,
    pub retirement_start_month: Option<u32>,
    pub income_retirement_monthly: Decimal,
    pub expense_retirement_monthly: Decimal,
    pub retirement_monthly_withdrawal: Decimal,
    pub fire_target: Option<FireTarget>,
}

pub struct FireTarget {
    pub base_amount: Decimal,             // FIRE en euros de hoy (gross-up de impuestos aplicado)
    pub annual_inflation_percent: Decimal, // 0 = target plano; > 0 = target móvil
}
```

## ProjectionLiabilityInput and repayment models (4.2.0)

```rust
pub struct ProjectionLiabilityInput {
    pub principal: Decimal,
    pub monthly_payment: Decimal,          // ya convertida a mensual por el handler (weekly ×52/12)
    pub payment_end: Option<NaiveDate>,
    pub repayment_model: RepaymentModel,   // 4.2.0
    pub apr_percent: Option<Decimal>,      // 4.2.0 — TIN nominal anual en puntos (3 = 3 %/año)
    pub min_payment_pct: Option<Decimal>,  // 4.7.0 (#144) — cuota mínima revolving: % del saldo de apertura
    pub min_payment_eur: Option<Decimal>,  // 4.7.0 (#144) — suelo en € de esa cuota mínima
    pub extra_principal_monthly: Decimal,          // 4.4.0 — amortización extra mensual. 0 = pre-4.4.0 bit a bit.
    pub extra_principal_lump_sums: Vec<(u32, Decimal)>, // 4.4.0 — (mes 1-based, importe); varios del mismo mes SUMAN
    pub early_repayment_fee_pct: Option<Decimal>,  // 4.7.0 (#151) — compensación % del extra; None = 0
    pub early_repayment_effect: EarlyRepaymentEffect, // 4.7.0 (#151) — ReduceTerm (default) | ReducePayment
}

pub enum RepaymentModel { FixedPayments, French, InterestOnly, Revolving }
pub enum EarlyRepaymentEffect { ReduceTerm, ReducePayment } // #[default] ReduceTerm
```

`apr_percent` es el **TIN nominal anual**: el tipo mensual es `i = apr_percent / 1200`, la MISMA
convención que `LoanTerms::apr_percent` de `history.rs`. Terminología: la columna SQL y el wire lo
llaman `apr_percent` («TAE») por historia, pero el engine lo trata como **TIN** — lo divide entre 12
sin desanualizar de forma compuesta. Si algún día se quiere TAE de verdad, es una conversión en el
handler, no un cambio de fórmula aquí.

`apr_percent` ausente o `≤ 0` ⇒ **sin interés**: cualquier modelo degenera exactamente en la
recurrencia sin intereses (y `InterestOnly` en un principal congelado). Deliberado: un `.ffbackup`
importado puede colar un `french` sin TIN y el engine no debe panicar ni fallar — devuelve la
serie sin intereses. La validación de coherencia vive en el handler (`liabilities.rs`), no aquí —
y desde 4.7.0 (#144) esa validación cierra el catálogo: el default de columna y formulario es
`french`, `fixed_payments` RECHAZA el TIN (`apr_forbidden_for_model`) y `revolving` exige sus
mínimos (`min_payment_pct`/`min_payment_eur`).

**Dos** helpers privados, y **tres** consumidores de ambos desde 4.4.0.
`liability_month(liab, principal, monthly_payment, active) → (cash, closing_principal)` resuelve
la recurrencia (antes eran tres copias del `min(cuota, principal)`; desde #151 la cuota que se le
pasa es la **efectiva** — un vector/escalar que solo muta «reducir cuota»), y
`liability_extra_principal(liab, k, closing_tras_cuota, active) → (extra, fee)` resuelve la
amortización extra: suma `extra_principal_monthly` + todos los lump sums de ese mes, **topa al
saldo** (`0 ..= closing_tras_cuota`, por eso el cierre nunca es negativo), devuelve **(0, 0) si el
plan no está activo**, y desde #151 devuelve también la **comisión** (`extra ×
early_repayment_fee_pct/100`) — coste puro que sale de la caja y NO baja el principal, fuera de
la identidad del calendario. Los tres consumidores son el bucle de simulación,
`first_month_allocation` y el calendario. El predicado de actividad es único, `liability_active`:
`monthly_payment > 0` **y** (`payment_end` ausente o `>= m_start`) — es lo que impide que un
what-if de amortización mueva el principal en los modos B/C, donde el handler pone
`monthly_payment = 0`. Y el predicado de **devengo** es público desde #121:
`liability_interest_accrues(model, apr, cuota, fin, mes)` = modelo ≠ `FixedPayments` + TIN > 0 +
plan vivo — lo consumen el `net_return` de `/v1/summary` y su espejo TS, para que el KPI nunca
cobre lo que la simulación no cobra.

| Modelo | Caja del mes | Principal de cierre | Notas |
|---|---|---|---|
| `French` (default desde 4.7.0) | `min(M, payoff)` | `payoff − cash` | `payoff = P·(1 + i)`: interés sobre el **saldo de apertura**, cuota a **fin de mes**. Misma recurrencia que `theo(y)` en `history.rs`. |
| `FixedPayments` | `min(M, P)` | `P − cash` | El préstamo **sin intereses** (0 %), bit a bit el modelo pre-4.2.0. Desde #144 el handler le RECHAZA el TIN: el «TIN informativo que el engine ignora» ya no es representable. |
| `InterestOnly` (#144) | `min(M, P·i)` | `P + P·i − cash` | Carencia real: la cuota del mes ES el interés del período; la declarada solo topa por arriba, y por debajo el déficit **capitaliza**. Nunca amortiza (eso es `extra_principal`). |
| `Revolving` (#144) | `min(max(pct·P/100, suelo), payoff)` | `payoff − cash` | La cuota es la MÍNIMA real (`min_payment_pct` del saldo de apertura con suelo `min_payment_eur`), no la declarada. Con pct 0 y suelo = cuota declarada degenera bit a bit en la francesa (forma del backfill de la migración, pineado). |
| cualquiera, **inactivo** | `0` | `P` | Sin plan activo no hay caja, ni amortización, ni **devengo**, **ni amortización extra**. |

**La tabla describe solo la pata de la CUOTA.** Desde 4.4.0 hay una segunda pata: la caja real del
mes es `cash + extra` y el cierre real es `closing − extra`, con `extra` de
`liability_extra_principal`. Consecuencia que la fila `InterestOnly → P (constante)` ya no cuenta
entera: con amortización extra activa **el principal sí baja** en `interest_only` — el modelo
congela lo que hace la cuota, no lo que hace una amortización anticipada.

Consecuencias verificadas por tests del engine:

- **El tope de la cuota es el payoff, no el principal** en los modelos que devengan: cancelar cuesta
  el saldo *con* el interés del mes (P = 400 al 3 % ⇒ `debt_service` = 401,00, no 400).
  `FixedPayments` sigue topando en el principal — cambio de comportamiento acotado a los modelos
  nuevos.
- **Cuota por debajo del interés ⇒ la deuda crece, sin topes**: 100.000 € al 12 % con cuota de 500 €
  cierra el mes 1 en 100.500 y el mes 2 en 101.005. El modelo pre-4.2.0 no podía ni representarlo.
- **Residual congelado**: si `payment_end` llega con principal vivo, ese principal se queda quieto
  para siempre (resta constante al patrimonio), en los cuatro modelos.
- **Saturación, nunca pánico**: `checked_mul`/`checked_add` en el payoff; un TIN absurdo (1.000 %
  × 840 meses) satura y la simulación termina con una serie completa.
- **Extinción**: 100.000 € con cuota de 500 € se extinguía en el mes **200** con `fixed_payments`
  (100.000/500); en `french` al 3 % se extingue en el mes **278** — 78 meses más, ≈ 38.800 € de
  intereses que el modelo viejo no cobraba.

```rust
pub fn present_value_of_payments(monthly_payment: Decimal, months: Decimal,
                                 apr_percent: Option<Decimal>) -> Decimal
```

Valor actual de una renta de `months` cuotas al TIN: `P = M·(1 − (1+i)^−n)/i`. Con `apr_percent`
ausente o `≤ 0` devuelve `M · n` **exacto**, sin pasar por `powd` (el límite cuando `i → 0`, y el
caso más común). `n` puede ser fraccionario. Cualquier `checked_*` que falle cae al mismo `M · n`.
Lo consume la derivación de principal del handler de pasivos **solo en `french`** (brazo
`RepaymentModel::French` de `derive_principal` en `handlers/liabilities.rs`; el resto de modelos
deriva la suma nominal `M · n`): `PV(500 × 200 @ 3 %) = 78.618,1542 €`
frente a los 100.000 € de la suma nominal.

**Per-mode liability contract (handler-side — reform 3.4.0, extendido en 4.2.0):** the engine always
subtracts every input liability's `principal` from net worth each month, and only charges cash /
amortizes / **accrues interest** when `monthly_payment > 0` and the plan is active. The HANDLER
exploits that: in mode A it passes the real payment plan (debt service charged, principal amortizes,
cuota freed at `payment_end_date`); in the real modes B/C (`savings_source.uses_transactions()`) it
zeroes `monthly_payment` **and `apr_percent`** in memory, so the principal becomes a **constant**
net-worth subtraction across the whole horizon — paid cuotas already live inside the raw 12m expense
average. Zeroing the TIN is **deliberately redundant** (without a payment the engine already accrues
nothing): it states the mode-B/C contract in one place, so relaxing the accrual gate in the engine
cannot silently start charging interest in the real modes. The projection input query uses the
shared visibility predicate (#145): plan vivo **o saldo vivo** (`payment_end_date IS NULL OR >=
today OR principal > 0`) — el vencido-con-saldo entra congelado (resta constante), same predicate
as `/v1/summary`. See `build_installation_projection_input` in
`apps/api/src/handlers/projection.rs`.

**Divergencia histórico ↔ proyección: CERRADA en 4.7.0 (#129).** El modelo de amortización viaja
ahora en el snapshot (`history_snapshot_items.repayment_model`, `.ffbackup` v11) y
`amortized_segment_value` elige la ley por el MODELO CAPTURADO (`LoanTerms::repayment_model`):
solo `french` ⇒ curva compuesta corregida por residuo; `revolving` (el snapshot no guarda sus
mínimos y la cuota declarada no gobierna su caja desde #144), `fixed_payments`, `interest_only`
y `None` (snapshot pre-4.7.0) ⇒ la cuerda — exacta para cuota fija (pendiente constante) y la
interpolación menos comprometida para el resto. El quiebro de pendiente en «hoy» desaparece
para las fotos NUEVAS (llevan la ley); las fotos 4.2.0–4.6.0 de un pasivo genuinamente francés
pierden la curva compuesta que hasta ahora se les aplicaba (interior ~300 €/50 k€; extremos
exactos) — el precio de dejar de aplicársela al default mayoritario, donde era el bug de #129. Además (#130): un item ausente de una captura ARRASTRA su
último valor (LOCF, `HistoryTimeline::last_is_live_ledger`); solo la ausencia del ledger vivo
significa cero.

## Calendario de amortización y amortización extra (4.4.0, Fase 6)

`liability_amortization_schedule` publica lo que el bucle **ya derivaba y tiraba**. No hay
matemática nueva: reutiliza `liability_month` + `liability_extra_principal`, así que el calendario
y la proyección no pueden separarse.

**El interés es un RESIDUO, no un devengo aparte.** El orden de derivación es deliberado — mandan
los saldos:

```rust
let (payment, closing_after_payment) = liability_month(model, principal, M, apr, true);
let extra    = liability_extra_principal(liab, k, closing_after_payment, true);
let closing  = closing_after_payment - extra;

let repaid_by_payment = principal - closing_after_payment;
let interest_accrued  = payment - repaid_by_payment;   // ← residuo
let principal_repaid  = principal - closing;
```

De ahí sale, **por construcción y en los cuatro modelos**, la identidad contable

```
payment + extra  ==  interest_accrued + principal_repaid
```

La cancelación es algebraica y no mira el `RepaymentModel`, así que seguiría valiendo si mañana se
añade un modelo nuevo. **Lo que valida es la coherencia interna del desglose publicado, no el
modelo económico** — su valor real es que rompe en cuanto alguien devengue el interés por su cuenta
en vez de derivarlo de los saldos. Pin: `schedule_payment_identity_holds_in_every_model`
(4 modelos × TIN ausente/6 %, con extra mensual y lump sum), espejo HTTP en
`apps/api/tests/liability_schedule.rs`.

Se puede comprobar que el residuo ES el devengo real en `french`/`revolving`: con
`payoff = P(1+i)` y `cash = min(M, payoff)`, sale `interest = P·i` — **también en el mes final de
cuota parcial**, donde `cash = payoff` y el cierre es 0.

**`principal_repaid` puede ser NEGATIVO y no se clampa.** Con `french`/`revolving` y una cuota por
debajo del devengo (`M < P·i`), el saldo crece y la resta sale negativa. Publicarlo como 0
escondería justo el caso que el modelo pre-4.2.0 no sabía ni representar. Lo único que se clampa es
el **saldo**: `closing ≥ 0`, porque `liability_extra_principal` topa al saldo. *(Cobertura: hoy
ningún test asserta directamente `principal_repaid < 0`; el caso «la deuda crece» se pinea de forma
indirecta con `final_principal > 100.000` en `schedule_payoff_absent_reasons_are_distinguishable`.)*

**Ausencia de payoff: cuatro variantes, cuatro remedios.** `payoff_month_index` y `payoff_absent`
son mutuamente excluyentes por invariante. `NoPaymentPlan` (no hay cuota),
`PaymentPlanEndsBeforePayoff` (la fecha fin llega con saldo vivo), `PaymentDoesNotReducePrincipal` (la cuota no cubre el
devengo, o el modelo no amortiza) y `NotWithinHorizon` (no cabe en los meses simulados). Colapsarlas
en un `null` sería el mismo error que colapsar `AllocationSkipReason`.

**Amortización extra: las dos mitades o ninguna.** La cuota liberada al extinguir un préstamo
**vuelve a la cascada, y eso NO es una decisión nueva**: es lo que el motor ya hacía cuando un
préstamo se extingue solo — `liability_month` devuelve `cash = min(M, 0) = 0`, el sobrante sube en
el importe de la cuota y la cascada lo encamina como cualquier euro. Suprimirlo exigiría **añadir**
código para esconder caja que el modelo tiene, y haría que un préstamo extinguido por amortización
extra se comportara distinto de uno extinguido de forma natural. La contrapartida es obligatoria:
la amortización extra **se cobra a la caja del mes** (`debt_service += cash + extra`), porque hacer
solo la mitad que baja el principal *imprimiría dinero*. Pins:
`extra_principal_is_net_worth_neutral_without_interest` (el único cuyo nombre declara la
neutralidad),
`extra_principal_frees_the_quota_into_the_cascade`,
`extra_principal_saves_exactly_the_interest_not_accrued` (100k @ 3 %: extinción 278 → 216, ahorro
9.281,9223 € == Δ`net_worth[300]`), `extra_principal_lump_sum_lands_on_its_month_and_caps_at_the_balance`,
`zero_extra_principal_is_bit_identical_to_the_pin` y `extra_principal_needs_an_active_payment_plan`.

**Matiz honesto sobre «efecto instantáneo cero»**: es exacto **en el balance** (los dos `−E` se
cancelan en `NW = Σ activos + surplus_cash − Σ principales − undrained`), y por eso el test que la pinea
usa un activo de rentabilidad **nula**. En la serie de un hogar cuyo activo marginal
compone a `g` mensual, el mes queda `E·g` por debajo: el euro sale de la caja **antes** del paso de
crecimiento y el principal baja **después**. Efecto colateral: `contributed_capital` también baja
en `E` en los meses con sobrante, porque la cascada reparte menos.

**El deflactado NO vive aquí.** El motor sigue simulando 100 % en nominal; `net_worth_real` y
`GET /v1/projection/deflate` son capa de presentación del handler — ver el bloque de
`deflator_at_month_index` en `.claude/api-routes.md` §Projection y la fila 3 de
`futurefin-failure-archaeology` §1 (el motor «real puro» sigue rechazado).


## Inflación y target FIRE móvil
- Ingresos, gastos y aportaciones se mantienen **constantes en euros nominales** a lo largo de la simulación (filosofía «haciendo lo que hago ahora, ¿qué tal voy?»). No se inflan.
- El rendimiento de activos (`expected_annual_return_percent`) es **nominal**, sin deflactar.
- El **target FIRE crece con la inflación cada mes**: `target(k) = base_amount × (1 + annual_inflation_percent/100)^((k-1)/12)`. Esto preserva el poder adquisitivo del usuario en el momento de jubilarse.
- `annual_inflation_percent = 0` degenera a un target plano (equivalente a tratar el FIRE como un escalar de euros de hoy).

## SimAsset fields
- `expected_annual_return_percent`: **nominal** compound growth rate (7 = 7%/year). None = no compound growth.
- `is_liquid`: liquid assets are drained first when cash is negative; sorted by growth rate (lowest first).
- `purchase_price`: optional cost basis; included in `contributed_capital[0]`.

## AllocationRule fields
```rust
pub struct AllocationRule {
    pub target_index: usize,            // index into ProjectionInput.assets
    pub kind: AllocationKind,           // Fixed | Percent | Remainder
    pub amount: Option<Decimal>,        // €/mes (Fixed); 0..=100 (Percent); None (Remainder)
    pub cap: Option<AllocationCap>,     // Amount(€) | MonthsExpense(N) | IncomeMultiple(N)
}
```
Rules are evaluated **in vector order** (caller passes them sorted by priority ASC). Per rule:
- Resolve `ceiling` via the cap variant: `MonthsExpense(N)` → `N × (expense + debt_service)`; `IncomeMultiple(N)` → `N × income`; `Amount(v)` → `v`. `None` = no ceiling. **Second-order effect since 4.4.0**: `debt_service` now includes the extra principal repayment, so a `months_expense` ceiling **moves** in a what-if that amortizes early. Only reachable through `simulate_projection`'s `liability_overrides` (the read paths pass both extra fields as zero), and there is **no test covering it** — treat it as known, unpinned behavior.
- `cap_room = max(0, ceiling − current_value(target))`. If 0, skip.
- Intent: `Fixed` → `min(amount, remaining)`; `Percent` → `remaining × amount / 100`; `Remainder` → `remaining`.
- `take = min(intent, cap_room?, remaining)` is added to `alloc[target]` and subtracted from `remaining`.

## Simulation loop (per month)
All monetary state is **nominal** throughout (euros del momento). El ajuste por inflación se aplica
únicamente al target FIRE, que crece cada mes para mantener el poder adquisitivo del usuario.

1. Compute `debt_service` = Σ of `liability_month`'s **cash** leg **plus `liability_extra_principal`**
   for every liability (0 when the plan is not active). The same call returns each liability's
   **closing principal**, which is stashed in `closing_principals` **minus the extra** and merely
   *applied* in step 8 — the recurrence is resolved **once per month per liability** since 4.2.0.
   Recomputing it in step 8 would recompute the accrual, and the two copies would eventually
   diverge. Nothing mutates `principals` between the two steps, and the step order is unchanged.
   **The extra is charged to cash AND subtracted from the principal — both halves or neither**
   (4.4.0): only the first would drain cash without reducing debt; only the second would print
   money. On the balance the two `−E` cancel out, so the instantaneous effect on net worth is
   exactly zero — **but read the caveat in the amortization-schedule section below**: the month's
   order is cascade → asset growth → principal assignment, so the euro leaves before compounding
   and the principal drops after it. In a household whose marginal asset grows at `g`, that month
   ends `E·g` lower (opportunity cost). The engine tests that pin the neutrality use a
   zero-return asset on purpose, to isolate the axis.
2. Determine `in_retirement = fire_reached || k >= retirement_start_month`. `fire_reached` compara `nw_prev` contra el target FIRE del mes `k`, que es `base × (1 + inflation/100)^((k-1)/12)`. Si se alcanza, usa `income_retirement_monthly` / `expense_retirement_monthly`; si no, las variantes regulares.
3. `retirement_withdrawal` = `retirement_monthly_withdrawal` if `in_retirement`, else 0.
4. `net_cash = income - expense - debt_service + planning_adj[k] - retirement_withdrawal`.
5. If `net_cash > 0` (surplus): **run the allocation cascade** over `allocation_rules` (see [AllocationRule fields](#allocationrule-fields)). Anything no rule absorbed flows into `surplus_cash` (counted in NW). `distribute_contributions` takes an optional trace sink (`Option<&mut Vec<RuleOutcome>>`): the loop passes `None` — it runs up to 840 times per request and nobody reads the trace there — while `first_month_allocation` passes `Some`. **One cascade implementation, not two**: a second one would diverge silently at the first cap change, and an explanation that disagrees with what the engine does is worse than no explanation. The cascade **cannot over-allocate**: `take` is bounded three times (rule intent, cap room, remaining cash) and the loop breaks when cash runs out.
6. If `net_cash <= 0` (deficit): drain `surplus_cash` first, then drain assets — ALL of them,
   liquids first, then illiquids, each group lowest-return first (tiebreak by input index); any
   need still uncovered accumulates in `undrained_cumulative` and is subtracted from net worth.
   (Erratum fixed 2026-08: this line used to say only "liquid assets", but `drain_from_assets`
   has always continued into illiquid assets once the liquids run dry.)
7. Apply compound growth (`× monthly_multiplier(rate)`) to each asset value — sin deflactar. `monthly_multiplier` = raíz 12ª del factor anual `1 + p/100`; `None` y `0` → factor 1; **las tasas negativas componen de verdad** (−50 % anual ⇒ ×0,5 en 12 meses); `p ≤ −100` se clampa a factor 0 (la capa API rechaza esos inputs con error tipado).
8. Assign each liability its `closing_principal` from step 1. No recomputation, no `min` — just the
   assignment.

## Output
```rust
pub struct ProjectionOutput {
    pub net_worth: Vec<Decimal>,         // nominal, euros del momento, index 0..=horizon_months
    pub contributed_capital: Vec<Decimal>, // cumulative cost basis (nominal)
    pub per_asset_series: Vec<Vec<Decimal>>, // value per asset per month (nominal)
    pub assets_depleted_month_index: Option<u32>, // 4.6.0 (#119): primer mes con déficit ≥ TODO lo drenable
    pub uncovered_deficit_total: Decimal,         // 4.6.0 (#119): undrained_cumulative final
}
```

Sobre los dos campos de 4.6.0 (#119): la definición del mes de agotamiento vive en el bucle — el
caso exacto usa `>=` («la cartera se vacía este mes»), no «primer mes con descubierto», que daría
el mes siguiente; pineado con 200.000 € / 2.000 €/mes ⇒ mes 100 y NW(360) = −520.000. Cero series
nuevas a propósito: la serie del descubierto es derivable de la identidad del NW, y un cuarto
array de 841 Decimals no lo pinta nadie.

## Errors
- `EngineError::InvalidHorizon` — horizon_months < 1
- `EngineError::InvalidPlanningAdjustments` — planning vec length != horizon_months
- `EngineError::InvalidAllocationRuleTarget` — `target_index` out of bounds of `assets[]`
- `EngineError::InvalidHistoryTimeline` — `HistoryTimeline::dates` not strictly ascending

## History interpolation (`history.rs`)

Pure module (no I/O, no async, no clock, **no `f64`** — only `Decimal` + `NaiveDate`) that
reconstructs the past net-worth series from **manual snapshots**. The API handler groups snapshots
into per-`(owner_user_id, kind)` timelines and asks the engine to evaluate each item on a grid of
month-first dates; the handler owns aggregation (Σ per user/household), scoping and the projection
join. The engine only interpolates.

Public API (re-exported from `lib.rs`):
```rust
pub fn evaluate_timeline(&HistoryTimeline, grid_dates: &[NaiveDate]) -> Result<Vec<Vec<Decimal>>, EngineError>
pub fn amortized_segment_value(p_a: Decimal, p_b: Decimal, terms: Option<&LoanTerms>,
                               days_from_start: i64, days_total: i64) -> Decimal
pub fn anchored_cashflow_segment_value(v_a: Decimal, v_b: Decimal, cf: &[CashFlowEntry],
                               seg_start: NaiveDate, seg_end: NaiveDate, eval_date: NaiveDate,
                               days_from_start: i64, days_total: i64) -> Decimal   // v1.6.0, tier-2
pub fn add_months_signed(date: NaiveDate, delta: i32) -> NaiveDate  // month-first, signed (neg = past)
pub fn month_index_of(date: NaiveDate, anchor_month_first: NaiveDate) -> i32  // (y2-y1)*12 + (m2-m1)
// types: HistoryTimeline { dates, items }, HistoryItem { source_item_id, kind, observations, cashflow },
//        HistoryObservation { value, terms }, LoanTerms { apr_percent, monthly_payment },
//        HistoryItemKind { Asset, Liability }, CashFlowEntry { date, delta }
```

`HistoryTimeline.dates` are **strictly ascending** (non-ascending → `InvalidHistoryTimeline`); the
LAST date may be a "virtual today" observation appended by the caller — the engine neither knows
nor cares which are virtual. `HistoryItem.observations` is parallel to `dates` (`None` = item not
present in that snapshot; a shorter vec is treated as `None` for the missing indices).

Evaluation rules (per item, per grid point `g`):
- Before the first snapshot `s_1`: `0`, **except** the grid point in `s_1`'s own month
  (`month_first(s_1) ≤ g < s_1`) which "clamps" and evaluates at `s_1` (first visible point is the
  observed value, not a false 0).
- Within a segment `[s_a, s_{a+1}]`: observed at **both** ends → interpolate (**Asset** = linear in
  civil days; **Liability** = `amortized_segment_value`); observed at **one** end only → that
  observed value exactly at its own snapshot date, `0` elsewhere in the segment (items appear /
  disappear without inventing ramps); **neither** → `0`.
- Guarantees **endpoint exactness**: the value at every snapshot date equals the observed value.

Liability interpolation is a **residual-corrected French amortization** curve:
`i = apr/1200`, `u = 1+i`, `f = days_from_start/days_total`, `N = days_total / 30.436875`,
`x = f·N`; `theo(y) = P_a·u^y − M·(u^y−1)/i` (via `Decimal::checked_powd`), `theo_c = max(theo, 0)`;
result `= max( theo_c(x) + f·(P_b − theo_c(N)), 0 )`. The residual term makes `f=0 → P_a` and
`f=1 → P_b` **exact** regardless of `powd` approximation. Falls back to **linear** when `terms` is
`None`, `apr ≤ 0`, `M ≤ 0`, `M ≤ P_a·i` (payment doesn't cover interest), or any checked op fails.
Snapshot mutations are **not** projection-engine inputs — they never touch the projection cache.

### Cash-flow anchoring (tier-2, v1.6.0)
`HistoryItem` gained an optional `cashflow: Vec<CashFlowEntry>` field (`#[serde(default)]`). A
`CashFlowEntry { date, delta }` is a dated cash movement that **shapes** an asset's curve **within**
its segment **without ever contradicting the snapshots** — the anchored curve still passes exactly
through both endpoints. `delta` is already sign-normalized by the caller (**positive raises** the
asset value; account leg = `+amount`, savings-destination leg = `−amount`); the engine never
interprets signs or sources, it only sums `delta`.

`anchored_cashflow_segment_value` computes, for an **asset** segment `[seg_start, seg_end]` observed
at both ends:

```
v(t) = Va + C(a→t) + f(t)·(Vb − Va − C_total)
```

- `C(a→t)` = Σ of `delta` over the **half-open** interval `(seg_start, eval_date]` (a txn dated on
  `seg_start` belongs to the *previous* segment; one dated on `seg_end` **does** count).
- `C_total = C(a→b)` = Σ of `delta` over `(seg_start, seg_end]`.
- `f(t) = days_from_start / days_total`, linear in civil days — the **same** base as
  `interpolate_linear` (same `clamp`, same division).

Properties (unit-tested as P1–P5 in `history.rs`):
- **P1 / P2 — endpoint exactness for arbitrary cash-flow**: `v(seg_start) = Va` (empty `(a→a]`,
  residual term ×0) and `v(seg_end) = Vb` **exactly** (`C(a→b) = C_total` cancels the residual;
  `f = n/n = 1`, no residual division). Holds for deltas that don't sum to zero, a delta dated on
  `seg_end`, etc.
- **P3 — empty ⇒ identical to `interpolate_linear`**: with `cashflow` empty the formula degenerates
  to `Va + f·(Vb − Va)`; moreover the caller (`evaluate_item_at`) only takes the anchored branch
  when some entry falls in `(d_a, d_b]`, otherwise it calls `interpolate_linear` **verbatim** — so a
  timeline with an empty (default) `cashflow` field reproduces the previous history series **bit for
  bit** (P3b).
- Deposit into flat snapshots (`Va = Vb`) jumps just after the deposit date, then decays linearly
  back to `Va` by `seg_end` (the snapshot wins; the inflow is re-absorbed).

**Liabilities and one-sided items ignore cash-flow, deliberately**: only the `(Some, Some)` **Asset**
arm consults `cashflow`. Liabilities already model the principal with residual-corrected French
amortization — injecting the payment as a delta would double-count it — so they stay bit-for-bit
identical to the no-cash-flow curve; items observed at a single endpoint keep their appear/disappear
behavior. Implementation: `O(n)` linear scan over `cf` per evaluation point (no prefix sums, robust
to any input order), sub-ms at this scale, no `f64`.

## Runway (`runway.rs`)

Pure module (v2.2.0) that answers "how many months do the **liquid** assets cover the monthly
expense?" while compounding the assets' expected return and inflating the expense. Sole consumer:
`GET /v1/summary` → `financial_health.runway_months` / `runway_is_indefinite`
(`apps/api/src/handlers/summary.rs`). Public API in the block above; 13 unit tests in-module
(as of 2026-08-15, v2.3.0).

| Input | Meaning |
|---|---|
| `liquid_assets: &[(Decimal, Option<Decimal>)]` | One row per liquid asset: `(current_value, expected_annual_return_percent)`. The handler passes exactly the rows of `assets WHERE is_liquid = true` in the requested scope. |
| `monthly_expense: Decimal` | Total monthly expense to cover — in the handler, `expense_total_monthly_equivalent` (so it follows `savings_source`). |
| `annual_inflation_percent: Decimal` | `installation.annual_inflation_assumption_percent`, clamped to ≥ 0 by the handler. |
| `swr_pct: Decimal` (v2.3.0) | `installation.fire_settings.swr_pct` (in %) — the **same** safe-withdrawal rate the FIRE target uses (Jubilación tab), read via `installation_calendar_inflation_fire`. Only drives the infinite case. |
| `annual_expense_for_swr: Decimal` (v2.3.0) | The **annual** expense already grossed up for taxes by the handler: `gross_up_net_annual_fire(expense_total × 12, fire.tax_brackets, fire.taxes_enabled)` — the *same* gross-up as the FIRE target. With `taxes_enabled = false` it is plainly `12 × monthly_expense`. The engine never recomputes `12 × monthly_expense` itself. |

Model (each rule exists for a reason — do not "simplify" one away):

- **Nominal frame**: assets grow at their *nominal* expected return and the expense is inflated every
  month. The result is a count of months (frame-invariant), but mixing nominal returns with a
  constant expense would overstate the runway.
- **Withdraw-then-grow order**: each month pays the expense first and grows what is left — the same
  order as the simulation loop in `projection.rs` (negative cash flow drains before the multipliers
  apply), so both curves stay coherent.
- **Value-weighted multiplier**: `m = Σ vₐ·monthly_multiplier(rₐ) / Σ vₐ`, i.e. a **prorated drain**
  (every asset funds the expense in proportion to its weight). Slightly **conservative** versus the
  engine's real drain, which empties the lowest-return liquids first and therefore keeps the
  high-return ones longer. Deliberate: the KPI must not promise more than the simulation.
- **Negative rates compound**: inherited from `monthly_multiplier` (shared with the simulation via
  `pub(crate)`, so the runway uses *exactly* the engine's annual→monthly conversion). A negative
  expected return (−100 < r < 0) now decays the balance for real and **shortens** the runway;
  `r ≤ −100` clamps to factor 0. The expense-inflation argument is never negative here (the
  installation validates 0..50).
- **SWR threshold (the infinite case, v2.3.0)**: `Indefinite` ⟺ the grossed-up annual withdrawal does
  not exceed the SWR applied to the starting balance, `annual_expense_for_swr ≤ A·(swr_pct/100)`.
  Compared **without dividing** — `annual_expense_for_swr·100 ≤ A·swr_pct` — so the boundary is
  *exact* in `Decimal`. It is the liquidity "FIRE number": `A ≥ gross_expense / SWR`. `swr_pct ≤ 0`
  can never satisfy it (right side ≤ 0, left side > 0), so no separate guard is needed. Beware: the
  `100` de-percentages `swr_pct` and is unrelated to `MAX_RUNWAY_MONTHS`, even though `12·100 = 1200`.
- **Check order (contract)**: `NoExpenseBase` (expense ≤ 0) → `Months(0)` (balance ≤ 0) → SWR
  threshold → finite loop. `NoExpenseBase` must come **first**: with expense 0 the inequality
  `0 ≤ A·swr` is trivially true and would report an undefined runway as infinite.
- **The trigger is deliberately independent of return and inflation**: it looks only at `A`, the
  grossed expense and the SWR — the definition of SWR already assumes a portfolio whose real return
  sustains that withdrawal long-term. Return and inflation still govern the **finite** case (the loop
  below). Accepted consequence: a balance below the threshold with a huge return is no longer
  "infinite", and one exactly at the threshold with 0 % return is.
- **100-year cap is a floor, not a sentinel**: surviving `MAX_RUNWAY_MONTHS` (1.200) months without
  meeting the SWR threshold returns `Months(1200)` — read as "at least 100 years", not an exact
  measure and **not** `Indefinite` (the UI renders it «+100 años»). Still no epsilon and no closed
  form: `ln`-based closed forms suffer cancellation exactly at the `A·j → g` boundary; the monthly
  loop avoids it and costs microseconds.
- **Exact reduction to `A / g`** (when the SWR threshold is *not* met, i.e. the finite branch): with
  return and inflation 0, `m = m_inf = 1` and the final fractional month reconstructs `A/g` with a
  single division — bit-exact **inside the engine**, which is where the property lives.
  Since 3.8.0 the HTTP surface publishes `runway_months` rounded to **1 decimal**
  (`handlers/summary.rs`, aligned with `sim_kpis` in `handlers/projection.rs`, which already did),
  so the baseline tests assert `(A/g).round_dp(1)`: still no tolerance, just the published
  precision. Anything that needs the full value must call `liquid_runway_months` directly.

  Wire-side consequence worth knowing: a runway below `0,05` months now serializes as `"0.0"`
  instead of a long non-zero decimal. `SummaryView` no longer keys the Runway tile off a
  zero-check for exactly this reason — a runway of zero months is information, not missing data.
- Edge cases: `monthly_expense <= 0` → `NoExpenseBase` (not "infinite"); total balance ≤ 0 →
  `Months(0)`.

Worked values (engine-verified). Finite branch, 12.000 € liquid vs 1.200 €/month, SWR 3,5 % (all four
below the threshold, unchanged since v2.2.0): return 0 % / inflation 0 % → 10; 5 % / 0 % → 10,19;
0 % / 3 % → 9,89; 5 % / 3 % → 10,07 months. Threshold branch (v2.3.0): 240.000 € vs 700 €/month at
SWR 3,5 % with taxes off → `Indefinite` on the **exact** boundary (840.000 = 840.000); 1.000.000 € at
7 % vs 4.000 €/month at SWR 3,5 % → `Months(1200)` floor, since 48.000 > 35.000 (it was `Indefinite`
in v2.2.0, when the cap decided infinity); with the default ES brackets `gross_up(8.400) ≈ 10.481 €`,
raising the threshold to ≈ 299.457 € of liquid balance.

## Rendimiento neto (`net_return.rs`)

Pure module answering "what is my net worth expected to return in a year?". Sole consumer:
`GET /v1/summary` → `financial_health.net_return_nominal_annual_pct` /
`net_return_real_annual_pct` (`apps/api/src/handlers/summary.rs`). 9 unit tests in-module.

```
numerator = Σ vₐ·rₐ/100 − Σ pₗ·aprₗ/100        (euros per year)
nominal_pct = 100 · numerator / (Σ vₐ − Σ pₗ)
real_pct    = 100 · ((100 + nominal_pct)/(100 + inflation_pct) − 1)
```

| Input | Meaning |
|---|---|
| `assets: &[(Decimal, Option<Decimal>)]` | `(current_value, expected_annual_return_percent)` for **every** asset in the requested scope — not only the liquid ones. |
| `liabilities: &[(Decimal, Option<Decimal>)]` | `(principal, apr_percent)` for the **non-expired** liabilities (handler applies `payment_end_date IS NULL OR >= today`, same predicate as `total_liabilities`). |
| `annual_inflation_percent` | `installation.annual_inflation_assumption_percent` (clamped ≥ 0 by the installation layer). |

Rules, each load-bearing:

- **`None` rate = 0 %, never an exclusion.** A row without a configured rate still weighs in the
  denominator, so it dilutes. Dropping it would report the return of the *configured* subset
  while calling it the return of the portfolio.
- **Net worth is the denominator**, so leverage amplifies in both directions: 100.000 at 5 %
  against a 60.000 loan at 3 % is 8 % on 40.000 of net worth, not 5 %.
- **`net_worth ≤ 0` ⇒ `None`.** With a non-positive denominator the quotient flips sign or
  diverges; the API omits both fields and the UI hides the tile rather than print a number that
  reads backwards.
- **Real by dividing factors, not subtracting points** (Fisher): `n − i` drifts exactly where it
  matters. The API layer rounds to 4 decimals of percent for publication (`PCT_DP` in
  `summary.rs`); the engine stays exact, same discipline as `runway_months`.
- **Expectation, not realized return**: it reads the rates the user configured per asset, never
  history, and ignores contributions — it measures the portfolio, not the saving.
- **Known divergence with the simulation, narrowed in 4.2.0 but not closed**: this KPI charges
  `apr_percent` on **every** non-expired liability, unconditionally. The projection loop only
  accrues interest on liabilities whose model accrues (all but `fixed_payments`, #144) **and**
  that have an active payment plan, and only in mode A (B/C zero the TIN). Desde 4.7.0 (#121) el
  KPI usa el MISMO predicado (`liability_interest_accrues`): la fila que no devenga entra al
  denominador con coste 0. El único residuo de divergencia con la curva proyectada son los modos
  B/C (anulan el TIN en el engine; el KPI no mira `savings_source`) — declarado aquí y en el
  texto de ayuda de la métrica.

Worked example (engine-verified, `worked_example_matches_the_documented_figures`): 100.000 at 5 %
+ 50.000 with no rate, minus a 60.000 loan at 3 % APR, inflation 2 % → numerator 3.200 €/year over
90.000 € of net worth = **3,5556 %** nominal, **1,5251 %** real (the naive subtraction would say
1,5556 %).

## Notes for the API handler (projection.rs)
- Load `allocation_rules` from DB ordered by `priority ASC`, then map each `target_asset_id` → index in `assets[]` before building the engine input.
- Planning flows with `due_date`: placed in their calendar month. Flows without `due_date`: spread over 90 days from ref_date.
- Horizon derivation (`projection_horizon_months`): se resuelve **una** fecha de nacimiento — `users.birth_date` del usuario de sesión, y si es NULL la primera fila de `persons` con `birth_date` por `is_primary DESC, sort_index ASC`. Horizonte = `clamp(90 − edad, 5, 70)` años × 12. Sin fecha de nacimiento: fallback **360 meses (30 años)**. `?months=N` (12–840) lo sobreescribe. `horizon_basis` reporta la razón: `lifespan_90` | `fallback_no_demographics` | `months_override`. (No existe `projection_target_age` — eliminado en v1.0.6.)
- Response includes UI-layer fields computed in the handler (not in engine): `milestones` (next 3 net-worth thresholds, **nominal**), `milestones_real` (same thresholds crossed over the **deflated** net worth = euros de hoy; empty when inflation is 0 — the web reuses `milestones`. The web picks the set from the "Inflation Adjusted" toggle), `compound_outpaces_true_savings_month_index`, `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date`. Both milestone sets are computed over the full monthly series (`points_full`), not the decimated `points`, so `reached_month_index` keeps precision under `density=hybrid`. `deflate_points_to_today` mirrors the chart's visual deflation (`ProjectionNetWorthChart.baseSeries`) but at monthly resolution.
- **Retirement drawdown**: el handler pasa **siempre** `retirement_start_month: None` — la jubilación se dispara únicamente cuando `nw_prev` cruza el target FIRE (`fire_reached`, v1.0.6). A partir de ese mes el ingreso cae a `income_retirement_monthly` (suma de `budget_entries` con `persists_after_retirement = true`) y el gasto pasa a `expense_retirement_monthly` (excluye gastos con `ends_at_retirement`). `retirement_monthly_withdrawal` es siempre 0 — la caída de ingresos por sí sola drena la cartera. El target FIRE lo computa el **servidor** (`compute_fire_target_nw` → `jubilacion_target_net_worth` en el response): `neto = expense_retirement − income_retirement` (modo annual_expense) o `neto = income − income_retirement` (modo current_income); si `neto ≤ 0` **no hay target** (`None`, no `max(0,…)`); si no, `target = gross_up(neto × 12) / (SWR/100)`. El frontend duplica la fórmula solo para el preview en vivo del formulario (paridad garantizada por `apps/api/tests/fixtures/fire-parity.json`).

## Performance notes (handler ↔ engine boundary)
- `project_net_worth_series` is CPU-bound (840 months × N assets × `Decimal::powd`). The handler wraps it in `tokio::task::spawn_blocking` to avoid blocking the reactor.
- `compound_outpaces_true_savings_month` is a **second projection pass** with `planning_adj = 0` and `liability.monthly_payment = 0` so the marker compares `market_growth` against a clean `income − expense` baseline. Eliminating the double pass would change the indicator's semantics; instead the handler runs both projections in parallel with `tokio::join!(spawn_blocking, spawn_blocking)`.
- The gross-up of net-annual FIRE through tax brackets uses a **closed-form per-bracket solver** (no binary search). `gross = (net − r·prev_ceiling + K) / (1 − r)`, advancing one bracket at a time until the candidate fits. Old code used 90 iterations of binary search on `Decimal`. Desde v2.3.0 `gross_up_net_annual_fire` es `pub(crate)` (`apps/api/src/handlers/projection.rs`) y tiene **dos consumidores**: el target FIRE y el umbral SWR del runway en `summary.rs` (`annual_expense_gross`). Cualquier cambio en los tramos o en el solver mueve **ambos** números a la vez — es intencional: comparten definición fiscal por diseño.
- `build_installation_projection_input` returns a `BuiltProjection` struct that carries `input`, `monthly_net_regular`, `asset_id_name` (Vec<(Uuid, String)>) and `planning_rows`. The handler reuses those instead of issuing a second `SELECT id, name FROM assets` and a second `SELECT planning_flows` (deleted with Fase 2.3). Desde v2.2.0 también expone `effective_savings_source` + (desde 3.9.0) `savings_income_basis` / `savings_expense_basis` — que **sustituyen** al escalar `savings_source_months_with_data`: con ventanas configurables por lado no existe *un* número de meses — (fuente **tras** el fallback, serializadas en `ProjectionSeriesResponse`) y `debt_service_monthly` (cuotas de pasivos activos; **no** es input del engine, que amortiza los pasivos aparte), que consume `assets_projection_context` para los caps `months_expense`.
- Initial queries in `get_projection_series` (installation row, user birth_date, household birth_date) run concurrently via `tokio::try_join!`.
