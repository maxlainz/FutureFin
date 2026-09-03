# Projection Engine (crates/engine)

> Este doc describe el CÓMO (API pública y bucle). El QUÉ de cada magnitud — unidad, convención,
> por qué refleja (o no) la realidad española, y las divergencias conocidas con su issue — vive en
> [`financial-contracts.md`](financial-contracts.md) (auditoría 2026-08).

Pure Rust crate — no I/O, no DB, no async. Pure financial math (projection + history interpolation).
La API pública es **solo `Decimal`** (`ls crates/engine/src/*.rs` para la lista viva de módulos):
- `money.rs` — el trait `MoneyOps` (5.0.0 WP5.5): el contrato numérico del núcleo, con la única
  implementación que vive aquí, la de `Decimal`. Ver §Núcleo genérico y crate estocástico.
- `sim.rs` — los tipos del núcleo (`SimInput`/`SimOutput` y los gemelos `*G`) y las conversiones
  desde y hacia la superficie pública. Copias campo a campo, cero aritmética.
- `sim_core.rs` — **el bucle**, genérico sobre `MoneyOps`: fases, cascada, venta del mes,
  crecimiento, recurrencia de pasivos, factores y objetivo clásico.
- `projection.rs` — los tipos públicos, el calendario de amortización, el valor actual de una renta
  y los ENVOLTORIOS `Decimal` de todo lo anterior (`project_net_worth_series`,
  `first_month_allocation`, `fire_target_at_month_index`…). Sigue siendo el sujeto de este doc.
- `phases.rs` — el `PhasePlan` de 5.0.0: trigger, fases, pensión con fecha, regla de retirada y los
  ejes de §B.3/§B.7 (ver [ProjectionInput fields](#projectioninput-fields)).
- `withdrawal.rs` — las cuatro reglas de retirada de la fase jubilada (5.0.0 WP2, §Reglas de retirada).
- `target.rs` — el objetivo consciente del PLAN: pensión con fecha, perpetuidad neta y puente
  (5.0.0 WP3, §El objetivo consciente del PLAN).
- `solve.rs` — las inversas por bisección sobre el motor (5.0.0 WP3, §Solves).
- `history.rs` — pure interpolation of the **historical** net-worth series from manual snapshots
  (see [History interpolation](#history-interpolation-historyrs) below). Deps unchanged
  (`rust_decimal` feature `maths` already present for `powd`).
- `net_return.rs` — expected annual net return of net worth (`net_return_percentages`; see
  [Rendimiento neto](#rendimiento-neto-net_returnrs) below). Consumed by `GET /v1/summary`.
- `runway.rs` — liquidity runway with compounded return + inflation (v2.2.0; **SWR threshold for the
  infinite case** since v2.3.0 — `Indefinite` ⟺ the grossed-up annual withdrawal fits inside
  `swr_pct` × liquid balance **AND the portfolio's weighted expected return is > 0** (4.8.0, #128);
  the finite case drains sequentially, lowest-return first, like the simulation; see
  [Runway](#runway-runwayrs) below). Consumed by `GET /v1/summary`.

## Núcleo genérico y crate estocástico (5.0.0 WP5.5)

**El bucle es uno solo y está parametrizado por su tipo numérico.** Monte Carlo (WP6) necesita
correr la MISMA simulación miles de veces, y en `Decimal` cuesta ~12 ms por proyección de 840
meses. La salida no fue duplicar el bucle en coma flotante —dos bucles divergen en silencio al
primer cambio— sino hacerlo genérico:

```rust
pub trait MoneyOps: Copy + PartialOrd + PartialEq + Sized + Debug
    + Add<Output=Self> + Sub<Output=Self> + Mul<Output=Self> + Div<Output=Self> + Neg<Output=Self>
{
    fn zero() -> Self;  fn one() -> Self;  fn max_value() -> Self;
    fn from_decimal(Decimal) -> Self;  fn to_decimal(self) -> Decimal;
    fn from_u32(u32) -> Self;  fn from_i64(i64) -> Self;
    fn checked_add(self, Self) -> Option<Self>;
    fn checked_mul(self, Self) -> Option<Self>;
    fn checked_div(self, Self) -> Option<Self>;
    fn min(self, Self) -> Self;  fn max(self, Self) -> Self;  fn clamp(self, Self, Self) -> Self;
    fn is_zero(self) -> bool;  fn is_sign_negative(self) -> bool;
    fn total_cmp(&self, &Self) -> Ordering;
    fn powd_fraction(self, num: u32, den: u32) -> Self;   // la familia (1+p)^{k/12}
    fn gains_equal(a: Self, b: Self) -> bool;             // el cortocircuito uniforme/mixto de g
    fn sum_of(impl Iterator<Item = Self>) -> Self;        // el mismo plegado que `Iterator::sum`
}

pub fn simulate<M: MoneyOps>(input: &SimInput<M>) -> Result<SimOutput<M>, EngineError>
```

`project_net_worth_series` y `first_month_allocation` son ENVOLTORIOS: convierten
`ProjectionInput` a `SimInput<Decimal>` (copia campo a campo, **cero operaciones**; medido: 1,2 µs
sobre P9, contra ~12 ms de la proyección) y devuelven la salida movida sin copiar un número.

**Por qué la instanciación `Decimal` no puede mover un dígito**: no es equivalencia algebraica, es
que ejecuta la MISMA secuencia de llamadas. Tres detalles que lo hacen cierto y que un refactor
descuidado rompería:

| detalle | por qué |
|---|---|
| `min`/`max` delegan en los **inherentes** de `rust_decimal`, NO en `Ord` | el inherente devuelve `self` en el empate y `Ord::max` devuelve `other`: `x.max(ZERO)` con `x = 0.000000000000000000` da `"0"` por `Ord` y `"0.000000000000000000"` por el inherente. **El pin dorado hashea el `Display`.** |
| `clamp` es `Ord::clamp` (no `max(lo).min(hi)`) | dentro del intervalo devuelve `self` intacto, escala incluida |
| `powd_fraction(k, 12)` construye el exponente como `from_u32(k)/from_u32(12)` y llama a `powd` | `powd` enruta los exponentes enteros por `checked_powu` (potencia exacta); un producto acumulado los desviaría a `exp`/`ln` |

**El `f64` vive FUERA de este crate.** El freezer `crates_engine_src_has_no_f64_outside_comments`
(`lib.rs`) sigue intacto y **sin excepciones**: la única implementación de `MoneyOps` en
`crates/engine` es la de `Decimal`. La de coma flotante es `F64Money` y vive en
**`crates/engine-stochastic`** (regla del huérfano: el trait es público). Ese crate no tiene bucle
propio — instancia este —, y su contrato es que **de él no sale un euro**: publica magnitudes
estadísticas (probabilidad de éxito, percentiles, agotamiento por edad), nunca un KPI monetario.
El dinero de la app sale siempre del camino `Decimal`.

Sus políticas están declaradas una por una en el doc de `F64Money` (`checked_*` = `None` con
no-finito; `total_cmp` = `f64::total_cmp` porque `drain_order` ORDENA y `f64` no es `Ord`;
`gains_equal` con tolerancia `GAIN_RATIO_EQ_TOLERANCE = 1e-12`; `from_decimal` pierde los ~12
últimos de los 28 dígitos). La igualdad con tolerancia está en el TRAIT y no escondida en un
`PartialEq`: `PartialEq` para `F64Money` sigue siendo el `==` exacto de `f64`.

**El gancho de Monte Carlo** es `SimInput::growth_overrides: Option<Vec<Vec<M>>>` (`[k−1][i]`):
cuando trae la fila del mes, el paso de crecimiento usa esos factores en vez del multiplicador
hoisted por activo. `None` —lo único que produce la conversión desde `ProjectionInput`— deja el
bucle donde estaba. Una fila mal dimensionada se ignora en vez de panicar.

**La puerta de degeneración** (`crates/engine-stochastic/tests/degeneration.rs`) corre los 23 casos
de la batería del motor por los dos caminos y compara `net_worth` y `liquid_worth` mes a mes en
todo el horizonte más los índices discretos. Medido: **máximo 1,5e-7 € en 840 meses** (P9), y los
cuatro índices (`retirement_month_index`, `liquid_crossing_month_index`,
`assets_depleted_month_index`, `phase_transitions`) coinciden EXACTAMENTE en todos los casos. La
única fila con cota relativa es `P14_techo_numeric`, sintético (activo en el techo de
`NUMERIC(18,4)` al 20 % durante 70 años ⇒ patrimonio ~3,5e19 €), donde el espaciado de los `f64`
ya supera el euro: allí la cota es `1e-12` relativa y se mide 2,0e-14.

Hallazgo que esta puerta cazó y se arregló en el mismo WP: la venta mixta con techo deducía «¿se
vendió el techo entero?» comparando `gross_monthly >= gross_cap` — exacto en `Decimal`, filo de
navaja en coma flotante, y de esa rama cuelga qué es **recorte informativo** y qué es
**descubierto que resta patrimonio**. Ahora el booleano lo publica el paseo
(`MixedGrossDrawdown::cap_exhausted`), que es quien lo sabe. Valía 8.138 € en P15; los dos pines
dorados no se movieron.

## Public API

```rust
// Main projection: net_worth, liquid_worth and contributed_capital series (len = horizon_months + 1,
// index 0 = today). contributed_capital = Σ basis por activo (#120, puede decrecer).
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
    pub leftover: Decimal,             // lo que ninguna regla absorbió; fuera del balance, contado en unallocated_savings_total
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
// 4.8.0 (#127): con CERO activos ya NO hay atajo a ceros — la caja del mes 1 (ingreso − gasto −
// deuda) se calcula igual, porque `net_recurring_monthly`/`net_cash_monthly` de la proyección la
// leen de aquí; solo `per_asset` y la traza quedan vacías. El cruce del mes 1 usa la riqueza
// LÍQUIDA (#143), igual que el bucle.

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

// 5.0.0 WP3: con pensión CON FECHA el objetivo lo evalúa `fire_target_at_month_index_with_plan` /
// `PlanFireTarget` (`target.rs`), que con `plan.pension == None` LLAMA a la función de abajo —
// bit-identidad por construcción. Ver §El objetivo consciente del PLAN.
//
// Único helper para evaluar el target FIRE inflado en un `month_index` dado (0 = punto de
// partida, 12 = un año después). Lo consumen tanto el motor (para `fire_reached`) como el
// handler (para construir `fire_target_series`). Antes había una fórmula duplicada — el motor
// usaba `years = (k-1)/12` y el handler `years = month_index/12`, lo que generaba un off-by-one
// de un mes entre cuándo se disparaba la jubilación y la serie pintada en el chart.
// 4.8.0 (#142): suma el término finito de deuda (`ft.debt_payments_remaining`, ver FireTarget) —
// con él, el objetivo DEJA DE SER MONÓTONO: solo vale el escaneo lineal del cruce.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal>

// 4.8.0 (#142): la serie del término de deuda — para cada mes m, Σ de los pagos que quedan
// ESTRICTAMENTE después de m (cuota efectiva + extra + comisión, calendario real de cada pasivo,
// cap 840 meses) + la cola residual (principal vivo al final del plan, constante). El handler la
// pega en `FireTarget.debt_payments_remaining` DESPUÉS de construir las liabilities del engine —
// y simulate la RECONSTRUYE tras aplicar los overrides (un extra que acorta el plan cambia el
// término; olvidarlo dejaría el objetivo del escenario con la deuda del baseline).
pub fn debt_payments_remaining_series(
    liabilities: &[ProjectionLiabilityInput],
    ref_date: NaiveDate,
) -> Vec<Decimal>

// Liquidity runway (v2.2.0): months the liquid assets cover the monthly expense, draining them
// sequentially (lowest expected return first, like the simulation's drain — 4.8.0, #128),
// compounding each remaining balance and inflating the expense. See the Runway section below.
// NOT an infinity sentinel (v2.3.0): the finite loop's cap. Surviving it returns `Months(1200)`,
// a FLOOR ("at least 100 years"); only the SWR threshold + the positive-return gate (#128)
// yield `Indefinite`.
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
    pub annual_inflation_percent: Decimal, // 4.9.0 (#139): indexa el GASTO del bucle; [−2, 50]
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    pub allocation_rules: Vec<AllocationRule>,   // cascade, in priority order
    pub liabilities: Vec<ProjectionLiabilityInput>,   // see per-mode contract note below
    pub planning_monthly_cash_adjustment: Vec<Decimal>,
    pub phase_plan: PhasePlan,         // 5.0.0 WP1b — absorbe los 4 campos de jubilación de 4.15.0
    pub fire_target: Option<FireTarget>,
}

// 5.0.0 WP1b (`phases.rs`, re-exportado de `lib.rs`); WP2 implementó las reglas de retirada y WP3
// las dos fases pendientes (media jornada y pensión con fecha) más los ejes de §B.3/§B.7.
// Sustituye a `retirement_start_month`, `income_retirement_monthly`,
// `expense_retirement_monthly` y `retirement_monthly_withdrawal`, que el bucle y
// `first_month_allocation` interpretaban cada uno por su cuenta.
//
// **Los callers usan los CONSTRUCTORES, nunca el literal** (`grep -rn "PhasePlan {" apps/ crates/`
// solo encuentra la definición): por eso cada campo nuevo entra con default y ningún caller se
// rompe. `ensure_supported()` ya solo rechaza PARÁMETROS imposibles de una regla
// (`InvalidWithdrawalRule`); `UnsupportedPhase` no la produce nadie desde WP3.
pub struct PhasePlan {
    pub retirement_trigger: RetirementTrigger, // LiquidCrossing | AtMonth(u32) (1-based, `k >= s`)
    pub partial: Option<PartialPhase>,         // WP3 — media jornada, simulada
    pub pension: Option<PensionSchedule>,      // WP3 — simulada (`start_index` es 0-based, la rejilla del target)
    pub withdrawal: WithdrawalRule,            // 5.0.0 WP2 — las CUATRO se simulan (`withdrawal.rs`)
    pub spend_mode: SpendMode,                 // Ceiling | RuleIsSpend — coinciden bajo FixedReal
    pub income_retirement_monthly: Decimal,    // ingreso que persiste tras jubilarse (plano)
    pub expense_retirement_monthly: Decimal,   // gasto tras jubilarse (se indexa con f(k−1))
    pub extra_monthly_withdrawal: Decimal,     // el antiguo `retirement_monthly_withdrawal`
    // ---- WP3 (§B.3, §B.7, D17). Todos con default en los dos constructores ----
    pub target_basis: TargetBasis,             // default Perpetuity — §El objetivo consciente del PLAN
    pub bridge_discount_annual_pct: Decimal,   // default 0 ⇒ puente sin descuento
    pub crossing_is_reading_only: bool,        // default false — D17: el cruce NO jubila, solo se anota
    pub contribution_cap_monthly: Option<Decimal>, // default None — techo de lo que la cascada invierte
    pub contributions_stop_month: Option<u32>, // default None — desde ese mes, techo 0 (coast)
    pub income_pause: Option<IncomePause>,     // default None — P8.c
}
pub enum RetirementTrigger { LiquidCrossing, AtMonth(u32) }
pub enum SpendMode { Ceiling, RuleIsSpend }
pub enum WithdrawalRule { FixedReal, PercentOfBalance { pct }, Hybrid { start_pct, end_pct },
                          Guardrails { pct, band_pct, adjust_pct } }   // las 4 desde WP2 — §Reglas de retirada
pub enum ExpenseBasis { Retirement, Regular }
pub enum Phase { Accumulating, Partial, Retired }
pub enum TargetBasis { Perpetuity, BridgeToPension }
// WP3 llenó el enum. `code()` es el literal PÚBLICO de cada aviso (el que la API publica en
// `warnings[]`): vive en el motor para que no haya un `match` duplicado en `apps/api`.
pub enum EngineWarning { RetireAtAgeUnderfunded, CoastNotReachable, PartialPhaseCapitalShrinking }
impl EngineWarning { pub fn code(self) -> &'static str }
pub struct PartialPhase { pub start_month: u32, pub income_monthly: Decimal, pub expense_basis: ExpenseBasis }
pub struct PensionSchedule { pub start_index: u32, pub monthly_today: Decimal, pub indexed: bool,
                             pub fraction_while_partial: Decimal }
impl PensionSchedule { pub fn monthly_at(self, i: u32, inflation_factor: Decimal) -> Decimal }
// Ventana SEMIABIERTA `from_month ≤ k < from_month + months`; multiplica el ingreso GANADO de la
// fase, nunca el término de pensión con fecha (una excedencia no pausa la pensión pública).
pub struct IncomePause { pub from_month: u32, pub months: u32, pub income_fraction: Decimal }

impl PhasePlan {
    // Lo que 4.15.0 hacía: cruce, fixed_real/ceiling, sin parcial ni pensión con fecha, retirada
    // extra 0, y TODOS los ejes de WP3 apagados. Bit-idéntico campo a campo.
    pub fn classic(income_retirement_monthly: Decimal, expense_retirement_monthly: Decimal) -> Self
    // El antiguo `retirement_start_month = Some(k)`: `classic` + trigger forzado + retirada extra.
    pub fn forced_at(start_month: u32, income_retirement_monthly: Decimal,
                     expense_retirement_monthly: Decimal, extra_monthly_withdrawal: Decimal) -> Self
}

// 4.10.0 (#170): el objetivo se evalúa MES A MES SOBRE LA NECESIDAD. `base_amount` —una cifra ya
// grosseada, ya dividida por el SWR y con la pensión ya restada antes de inflar— se RETIRÓ en esa
// ola: inflaba el neto entero mientras el motor drena `gasto·f(k) − pensión`, y el objetivo se
// quedaba corto en `pensión·(f(k)−1)` al mes. Lo que viaja ahora son los INGREDIENTES.
pub struct FireTarget {
    pub need: FireNeed,                    // la NECESIDAD, no el resultado (#170)
    pub swr_pct: Decimal,                  // 3.5 = 3,5 %. <= 0 ⇒ sin objetivo (None en toda la serie)
    pub tax_brackets: Vec<TaxBracket>,     // la MISMA escala que el drenaje (#140)
    pub taxes_enabled: bool,
    pub taxable_gain_ratio: Decimal,       // g ∈ [0,1] — fracción gravable de cada euro bruto (#140 fase 2)
    pub annual_inflation_percent: Decimal, // 0 = target plano; > 0 = target móvil
    // 4.8.0 (#142): término FINITO de deuda — `debt_payments_remaining[m]` = Σ de los pagos de
    // cuota que quedan DESPUÉS del mes m (cuota + extra + comisión) + cola residual (principal
    // que el plan no llega a amortizar). Índice fuera de rango → last() (la cola es constante).
    // Lo construye el handler con `debt_payments_remaining_series`; vacío ⇒ término 0.
    // Emparejado con la base de cruce LÍQUIDA (#143): el objetivo exige cubrir la perpetuidad
    // (base) MÁS todos los euros de cuota pendientes, y a cambio el cruce compara contra la
    // riqueza líquida BRUTA (sin restar principal). Algebraicamente equivalente al par
    // «NW neto vs base + interés restante», pero medible con activos vendibles.
    pub debt_payments_remaining: Vec<Decimal>,
}

// La estructura de la necesidad NO es la misma en los tres modos FIRE (#170).
pub enum FireNeed {
    // `manual` y `current_income`: la cifra declarada en euros de hoy se indexa ENTERA.
    Indexed { annual_net_today: Decimal },
    // `annual_expense`: gasto de jubilación (se INDEXA) menos el ingreso que persiste (PLANO,
    // #139). Es la necesidad REAL que el drenaje ejecuta: `max(0, E·f(k) − I)·12`.
    ExpenseMinusPension { expense_monthly: Decimal, pension_monthly: Decimal },
}
```

### El objetivo consciente del PLAN (5.0.0 WP3, §B.3 de #207)

Con **pensión con fecha** la necesidad deja de ser una sola: hay una antes de `P` y otra desde `P`.
`crates/engine/src/target.rs` lo resuelve **sin tocar** `fire_target_at_month_index`, que sigue
siendo el objetivo de 4.15.0 y el que el pin dorado hashea.

```rust
// Objetivo de un PLAN en el índice 0-based `i`. Con `plan.pension == None` LLAMA a
// `fire_target_at_month_index(ft, i)` — bit-identidad por construcción, no por revisión.
pub fn fire_target_at_month_index_with_plan(
    ft: Option<&FireTarget>, plan: &PhasePlan, month_index: u32,
) -> Option<Decimal>

// El mismo objetivo, PREcomputado: O(P) al construirlo, O(1) por consulta. Es el que usa el bucle.
pub struct PlanFireTarget<'a> { /* … */ }
impl PlanFireTarget<'_> {
    pub fn new(ft: Option<&FireTarget>, plan: &PhasePlan) -> Self
    pub fn at(&self, month_index: u32) -> Option<Decimal>
    pub fn need_full_annual_at(&self, i: u32) -> Option<Decimal>  // 12·need_full_m(i), €/año
    pub fn pension_monthly_at(&self, i: u32) -> Decimal           // P_m(i), €/mes
    pub fn expense_monthly_at(&self, i: u32) -> Option<Decimal>   // E·f(i), €/mes (sin restar nada)
    pub fn pension_coverage_ratio(&self) -> Option<Decimal>       // P_m(P)/(E·f(P)), FRACCIÓN
    pub fn partial_gap_target(&self, plan: &PhasePlan, expense_regular: Decimal) -> Option<Decimal>
}
pub const MAX_BRIDGE_MONTHS: u32 = 1_200;  // más allá, el puente degrada a perpetuidad ÍNTEGRA
```

**Una unidad por término** (hallazgo B1 de la revisión adversarial: mezclar €/mes con €/año hace
que el puente salga 12 veces mal sin que nada falle). Con `f(i) = inflation_factor_at_month_index`,
`P = pension.start_index` (0-based, la rejilla del target) e `I_persist` = el ingreso plano que
persiste:

| símbolo | unidad | definición |
|---|---|---|
| `need_full_m(i)` | €/mes | `max(0, E·f(i) − I_persist)` |
| `P_m(i)` | €/mes | `0` si `i < P`; `monthly_today·f(i)` si `indexed`, plano si no |
| `need_net_m(i)` | €/mes | `max(0, E·f(i) − I_persist − P_m(i))` |

- **`TargetBasis::Perpetuity`** — `T(i) = gross_up(12·need(i))/SWR + deuda(i)`, con `need_full_m`
  mientras `i < P` (la pensión no se cuenta hasta que existe; R6, la lectura conservadora) y
  `need_net_m` desde `P`. Si `need_net_m(i) ≤ 0` ⇒ `T(i) = deuda(i)`: **nunca `None`** — un
  objetivo ausente ahí diría «no se jubila jamás» cuando la verdad es «se jubila ya» (hallazgo B3).
- **`TargetBasis::BridgeToPension`** — para `i < P`, el valor presente del puente más la
  perpetuidad de lo que la pensión NO cubra, con `d = bridge_discount_annual_pct/100`:

  ```text
  T(i) = Σ_{m=i}^{P−1} gross_up_monthly(need_full_m(m))·(1+d)^{−(m−i)/12}
       + [gross_up(12·need_net_m(P))/SWR]·(1+d)^{−(P−i)/12}
       + deuda(i)
  ```

  Desde `P` coincide término a término con la perpetuidad neta. Los **dos escenarios de D15 caen
  solos**: si la pensión cubre el 100 % del gasto el término perpetuo es 0 exacto y el objetivo es
  solo el puente; si cubre una parte, queda la perpetuidad sobre el resto.

**Cómo se computa, y por qué no es la suma llana.** `q(j) = inflation_factor_at_month_index(d, j)`
es el MISMO helper que la inflación (una sola implementación del factor, como manda #139), y
`(1+d)^{−(m−i)/12} = q(i)/q(m)`, así que el puente es `q(i)·Σ_{m≥i} G(m)/q(m)` — una **suma
sufijo**: `O(P)` una vez, `O(1)` por evaluación, contra el `O(P²)` de la suma directa (cientos de
miles de gross-ups a 840 meses). En `i = 0`, donde `q(0) = 1` exacto, la forma-cociente ES la suma
directa término a término. Nunca por producto acumulado: `powd` enruta los exponentes enteros por
`checked_powu` y un producto acumulado los desviaría a `exp`/`ln`.

Medido (release, `tests/timing.rs`): la tabla del puente cuesta **≈ 10 µs por mes de puente** —
2,4 ms con `P = 240`, ~8,4 ms con `P = 840`.

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

**Identidades del motor (4.12.1 — `surplus_cash` retirado, #175/#176)**. Viven exactamente aquí,
una vez; cualquier otra mención de estas tres magnitudes en este doc remite a este bloque:
```
net_worth[k]            = Σ activos(k) − Σ principales(k) − undrained(k)
liquid_worth[k]         = Σ activos líquidos(k)
contributed_capital[k]  = Σ basis_i(k)
```
`surplus_cash` murió como término de las tres. El sobrante que ninguna regla absorbe (paso 5 del
bucle) queda FUERA del balance: se cuenta aparte en `unallocated_savings_total` (ver
[Output](#output)), inalcanzable en producción con activos vivos (sumidero indestructible, #176).

**Matiz honesto sobre «efecto instantáneo cero»**: es exacto **en el balance** (los dos `−E` se
cancelan en la identidad del NW de arriba), y por eso el test que la pinea
usa un activo de rentabilidad **nula**. En la serie de un hogar cuyo activo marginal
compone a `g` mensual, el mes queda `E·g` por debajo: el euro sale de la caja **antes** del paso de
crecimiento y el principal baja **después**. Efecto colateral: `contributed_capital` también baja
en `E` en los meses con sobrante, porque la cascada reparte menos.

**El deflactado NO vive aquí.** El motor sigue simulando 100 % en nominal; `net_worth_real` y
`GET /v1/projection/deflate` son capa de presentación del handler — ver el bloque de
`deflator_at_month_index` en `.claude/api-routes.md` §Projection y la fila 3 de
`futurefin-failure-archaeology` §1 (el motor «real puro» sigue rechazado).


## Inflación y target FIRE móvil
- **El GASTO se indexa a la inflación de la instalación** (4.9.0, #139): en el mes `k` el bucle cobra `expense_base × f(k−1)` con el factor único `inflation_factor_at_month_index` — el MISMO eje `(k−1)/12` que el trigger del target, así que `f(1)=1` y el mes 1 cobra exactamente lo que el usuario tecleó. Ambas ramas (regular y jubilación) escalan por el mismo factor: la discontinuidad del cruce es la de siempre × `f(k*)`, sin saltos nuevos, y el gasto de jubilación declarado está en euros de HOY (la simulación lo actualiza sola). En B/C se indexa el escalar YA restado de cuotas declaradas (#142) — la cuota es nominal por contrato y el motor la cobra aparte sin inflar. Los techos `months_expense` heredan el gasto del mes, así que **crecen con la inflación** (pineado). **Los INGRESOS quedan planos a propósito** (decisión del owner: «las subidas hay que pelearlas») — consecuencia aritmética detectada y cuantificada: el objetivo resta la pensión ANTES de inflar y se queda corto en `I_ret·(g^y − 1)/SWR` (issue #170; se arregla en la Ola 6, con el gross-up ya dentro del engine).
- El rendimiento de activos (`expected_annual_return_percent`) es **nominal**, sin deflactar.
- El **target FIRE se reevalúa cada mes SOBRE LA NECESIDAD** (4.10.0, #170): `target(i) = gross_up(need(i)·12, tramos, g)/(swr/100) + debt_term(i)`, con la necesidad indexada según su estructura (`FireNeed`) — **no una base pre-calculada que se infla entera**; `base_amount` se retiró en esa ola. `debt_term(i)` es el término finito de deuda de #142 (ver `FireTarget.debt_payments_remaining`; 0 sin pasivos) y **no se infla** (las cuotas son nominales por contrato). El gross-up de la necesidad inflada NO es el gross-up inflado: la escala es afín y los tramos son NOMINALES (fiscal drag). Con pensión CON FECHA el objetivo lo evalúa el evaluador consciente del plan (§El objetivo consciente del PLAN); sin ella, `fire_target_at_month_index` sigue siendo la única fuente.
- `annual_inflation_percent = 0` degenera a una base plana — pero con deuda viva el objetivo completo es **estrictamente decreciente** (el término cae con cada cuota pagada).
- **El objetivo YA NO ES MONÓTONO** (4.8.0): base creciente + término decreciente. Ninguna optimización puede asumir monotonía (búsqueda binaria del cruce, salida temprana); el cruce se decide por escaneo lineal.

## SimAsset fields
- `expected_annual_return_percent`: **nominal** compound growth rate (7 = 7%/year). None = no compound growth.
- `is_liquid`: liquid assets are drained first when cash is negative; sorted by growth rate (lowest first).
- `purchase_price`: optional cost basis; seeds `basis[i]` (#120) and therefore `contributed_capital[0]`. La base baja proporcionalmente al valor drenado y la rentabilidad nunca la toca — el hueco `valor − base` es la plusvalía latente.

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
2. **Fase del mes** (5.0.0 WP3, §B.1), monótona `Accumulating → Partial → Retired`:
   - **Jubilación** por el **latch absorbente** (4.8.0, #141): `retired = retired || (fire_reached && !crossing_is_reading_only) || k >= retirement_trigger.forced_month()` — una vez jubilado, SIEMPRE jubilado, aunque el patrimonio caiga después por debajo del objetivo (antes el estado parpadeaba mes a mes con gastos crecientes e ingresos planos). `fire_reached` compara **`liquid_prev`** — la riqueza LÍQUIDA del mes anterior (`liquid_worth[k−1]`): Σ de los activos `is_liquid`, BRUTA, sin restar principal (#143; `surplus_cash` retirado de este término en 4.12.1/#175 — teorema: el cruce solo pudo irse MÁS TARDE con el cambio, nunca adelantarse; emparejada con el término de deuda del target) — contra `PlanFireTarget::at(k−1)`.
     **`crossing_is_reading_only` (D17)** desactiva el cruce como TRIGGER sin tirar el objetivo: `liquid_crossing_month_index` se sigue anotando, y quien jubila es solo `AtMonth(R)`. Es lo que permite a una estrategia por edad conservar el objetivo para el chart y para medir el infra-financiado. Con `false` (el default) la unión `cruce || mes forzado` es la de 4.15.0, tal cual, y `P10_jubilacion_forzada` sigue pineado.
     Si en el mes efectivo `liquid_prev < target(k−1)`, se emite `EngineWarning::RetireAtAgeUnderfunded` (se mira el OBJETIVO, no el trigger: si jubiló el cruce, la desigualdad no puede darse).
   - **Media jornada** (P7/D10) si el latch no cerró y `k ≥ partial.start_month`: el ingreso es `partial.income_monthly` (PLANO) y el gasto el que diga `expense_basis` (jubilación por defecto, regular si el perfil lo dice). `partial_retirement_month_index` solo se publica si la fase se pisó de verdad — una media jornada declarada DESPUÉS de la jubilación no ocurre y no se pinta.
   El ingreso y el gasto salen de la fase; **el gasto elegido se multiplica por `f(k−1) = inflation_factor_at_month_index(input.annual_inflation_percent, k−1)`** (4.9.0, #139) — el ingreso no.
3. **Pensión con fecha** (P2/D3/D8, WP3): desde `k−1 ≥ pension.start_index` se SUMA al ingreso, **en cualquier fase**, por `monthly_today·f(k−1)` si `indexed` y plana si no; durante `Partial`, × `fraction_while_partial`. Sin pensión con fecha aquí no se ejecuta ni una suma (bit-identidad). `pension_start_month_index = start_index + 1` (rejilla 0-based del target → mes 1-based del bucle), `None` si cae fuera del horizonte.
   **Pausa de ingresos** (P8.c): dentro de la ventana semiabierta de `income_pause`, el ingreso GANADO de la fase se multiplica por `income_fraction` — la pensión con fecha, que se suma después, NO se pausa.
4. `retirement_withdrawal` = `phase_plan.extra_monthly_withdrawal` if `in_retirement`, else 0.
5. `net_cash = income - expense - debt_service + planning_adj[k] - retirement_withdrawal`.
6. If `net_cash > 0` (surplus): **techo de aportación** primero (WP3, §B.7) — con `contribution_cap_monthly = Some(c)` (o `k ≥ contributions_stop_month`, que impone `c = 0`) la cascada solo ve `min(sobrante, c)` y el resto se publica en `disposable_cash(k)`: **no se invierte, no compone y no entra en el patrimonio**, mismo trato que `unallocated_savings_total`. Sin techo, el pool es el sobrante entero y no se ejecuta una operación de más. Después, **run the allocation cascade** over `allocation_rules` (see [AllocationRule fields](#allocationrule-fields)) — **también en jubilación** (4.12.1, #175): es la MISMA cascada del usuario, sin rama especial por estado; los techos de la fase #171 pasan de explicativos a vinculantes también aquí. `AllocationSkipReason::InRetirement` y el literal de wire `in_retirement` MURIERON con la rama que los producía. Lo que ninguna regla absorbe **NO entra en el NW**: se acumula en `unallocated_savings_total` (ver [Output](#output)) — inalcanzable en producción con activos vivos (sumidero indestructible, #176). Lo reinvertido en cada activo, jubilado o no, **sube su base de coste** (`basis[i] += alloc[i]`, `basis_declared[i] = true`) y abarata sus ventas futuras (#178, ver paso 6). `distribute_contributions` takes an optional trace sink (`Option<&mut Vec<RuleOutcome>>`): the loop passes `None` — it runs up to 840 times per request and nobody reads the trace there — while `first_month_allocation` passes `Some`. **One cascade implementation, not two**: a second one would diverge silently at the first cap change, and an explanation that disagrees with what the engine does is worse than no explanation. The cascade **cannot over-allocate**: `take` is bounded three times (rule intent, cap room, remaining cash) and the loop breaks when cash runs out.
7. **La venta del mes** (`execute_month_sale`). Desde 5.0.0 WP2 este paso ya NO es el `else`
   del anterior: corre SIEMPRE, después de la cascada, porque `spend_mode = rule_is_spend`
   vende también en meses de superávit (R7) — y en ese caso el orden importa: la cascada
   invierte la caja del mes PRIMERO y la regla vende DESPUÉS, sobre el saldo ya reinvertido.
   Hasta 4.15.0 las dos ramas eran excluyentes, así que bajar la venta detrás del reparto no
   mueve un dígito de ningún caso de 4.15.0 (`pins-4.15.json`, verde). Cuánto se vende lo
   decide la regla (§Reglas de retirada); cómo se vende no ha cambiado:
   **el escalón «caja primero» murió con `surplus_cash`** (4.12.1,
   #175) — el déficit entero se vende **BRUTO** (`need_assets_net = -net_cash`; 4.10.0/#140 fase 1
   — M1, dentro del bucle, jubilado o no), drenando
   ALL assets — liquids first, then illiquids, each group lowest-return first (tiebreak by input
   index; orden extraído en `drain_order`, implementación ÚNICA que comparten
   `drain_from_assets`, esta rama y el runway). La exención fiscal que tenía la caja no desaparece:
   la hereda el drenaje natural — la cuenta al 0 % drena primero (`drain_order`) y, si su base fue
   alimentada por la cascada, deriva `g = 0` (`b = v`). **La `g` es POR ACTIVO desde 4.12.0 (#178)**:
   si el activo declaró coste (`purchase_price` presente, 0 incluido) **O** su base viva fue
   alimentada por la propia cascada (`basis_declared[i]` — extensión B de #178, 4.12.1: el euro
   aportado ES el dato, aunque el activo no declarara `purchase_price`), `g_i = max(0, 1 − b_i/v_i)`
   derivada de su base viva — invariante al drenaje del propio mes (`b'/v_post = b/v_pre`),
   creciente con el crecimiento (`ρ_k = ρ₀·m^{−k}`) —; si ninguna de las dos aplica, el escalar `taxable_gain_ratio`.
   Con `g` uniforme sobre lo vendible, CORTOCIRCUITO al camino literal de 4.11.0
   (`gross_up_monthly` + `drain_from_assets` + `after_tax_monthly`) — bit-idéntico; con mezcla,
   `gross_up_mixed_monthly` (forma cerrada por tramos sobre la base agregada `Σ g_i·venta_i`;
   sin iteración) decide bruto Y reparto a la vez, y el descubierto sale NETO por construcción.
   La base de coste de cada activo BAJA con lo drenado (`b' = b·v_post/v_pre`, #120 — con
   `checked_mul` y reordenamiento a `b·(v_post/v_pre)` **solo** si el producto no cabe en un
   `Decimal`, issue #209: con un activo cerca del techo de `NUMERIC(18,4)` y rentabilidad alta el
   producto desbordaba y el motor PANICABA) y
   `undrained_cumulative` acumula el descubierto **NETO** (mide gasto que faltó, no ventas que
   no ocurrieron); se resta del net worth. El chequeo de agotamiento (#119) compara el BRUTO
   INTENTADO (el de la regla, no el de la necesidad) contra lo vendible. El OBJETIVO y el umbral SWR siguen con el escalar
   (perpetuidades — el reparto de regímenes vive en financial-contracts §2.4).
   (Erratum fixed 2026-08: this line used to say only "liquid assets", but `drain_from_assets`
   has always continued into illiquid assets once the liquids run dry.)
   **La fase parcial NO pasa por la regla de retirada**: las reglas se anclan en `L(R−1)`/`f(R−1)` —el patrimonio con el que se ENTRA en la jubilación— y en `Partial` ese ancla no existe todavía, así que un déficit de media jornada se vende como el de quien trabaja: necesidad fija, bruta, sin techo.
8. Apply compound growth (`× monthly_multiplier(rate)`) to each asset value — sin deflactar. `monthly_multiplier` = raíz 12ª del factor anual `1 + p/100`; `None` y `0` → factor 1; **las tasas negativas componen de verdad** (−50 % anual ⇒ ×0,5 en 12 meses); `p ≤ −100` se clampa a factor 0 (la capa API rechaza esos inputs con error tipado).
9. Assign each liability its `closing_principal` from step 1. No recomputation, no `min` — just the
   assignment.

## Reglas de retirada (5.0.0 WP2 — `withdrawal.rs`)

Quién decide **cuánto** se vende en la fase jubilada. Hasta 4.15.0 había una sola regla y no se
llamaba así: cada mes jubilado con déficit se vendía exactamente lo que la caja no cubría — eso es
`fixed_real`, sin techo. El estado vive en `WithdrawalPlanner` (latch de `hybrid`, multiplicador de
`guardrails`), FUERA del bucle: una regla con memoria que se recalcula desde cero cada mes no es la
regla.

**Convenciones de índice, y no son decorativas:**

| símbolo | qué es | de dónde sale en el bucle |
|---|---|---|
| `R` | primer mes JUBILADO, 1-based | `retirement_month_index` |
| `L(k−1)` | patrimonio LÍQUIDO al cierre del mes anterior | `liquid_prev`, el MISMO valor que consume el cruce |
| `L_R` | `L(R−1)`: el líquido con el que se entra en la jubilación | se ancla el primer mes jubilado |
| `f(i)` | `inflation_factor_at_month_index(annual_inflation_percent, i)` | el mes `k` usa `f(k−1)`, como el gasto |

**Los `pct` son BRUTOS de impuestos** (R9), igual que el SWR (`gross/SWR`): el techo topa la
**venta**, no los euros que llegan al bolsillo. Con impuestos encendidos, un techo del 4 % netea
menos del 4 % — eso es el contrato, no un error de unidad. (Pin: `P17_guardrails_taxes_es`, mes 1 =
2.333,33 brutos ⇒ 1.853,33 netos con la escala ES.)

| regla | permitido BRUTO del mes `k` |
|---|---|
| `fixed_real` | **sin techo**: el permitido ES la necesidad del mes (el drenaje de 4.15.0, bit a bit) |
| `percent_of_balance {pct}` | `pct/100 · L(k−1) / 12` |
| `hybrid {start,end}` | `start` hasta el latch, `end` después. Latch: primer mes jubilado con `end·L(k−1) ≥ start·L_R·f(k−1)/f(R−1)` — es decir, cuando la retirada del porcentaje final ya no es menor que la inicial actualizada al IPC. **Monótono**: no se reabre |
| `guardrails {pct,band,adjust}` | `W_R · mult · f(k−1)/f(R−1)`, con `W_R = pct/100·L_R/12`. Cada 12 meses desde `R` (`k = R+12, R+24, …`) se mide `ratio = 12·W_k/L(k−1)` contra `ratio₀ = pct/100`: `ratio > ratio₀·(1+band/100)` ⇒ `mult ·= 1 − adjust/100` (capital preservation); `ratio < ratio₀·(1−band/100)` ⇒ `mult ·= 1 + adjust/100` (prosperity). El ajuste multiplica la BASE `W_R`, no el `W` del año, para que la indexación siga funcionando |

**Guyton-Klinger 2006, y lo que NO está**: se implementan las reglas de *capital preservation* y
*prosperity*. **Quedan fuera, a propósito y declarado**: la *portfolio management rule* con su
ventana de 15 años (que apaga el recorte cuando quedan menos de 15 años de plan) y la *inflation
rule* (que salta la subida por IPC del año siguiente a un recorte). Las dos SUAVIZAN el modelo;
omitirlas deja una versión más reactiva, que es la dirección prudente. Y en el camino
DETERMINISTA con rentabilidad > SWR la prosperity dispara **todos los años** (*ratchet*): es lo que
la regla dice sobre un camino sin volatilidad, y es exactamente por lo que los guardarraíles solo
tienen sentido pleno con Monte Carlo (WP6).

**Los dos `spend_mode` (D5)**, con `need` = `max(0, −net_cash)` del mes:

- `Ceiling` — solo actúa en meses de déficit: `venta_bruta = min(need_gross, permitido)`.
- `RuleIsSpend` (R7) — la regla ES el gasto del patrimonio: se vende el **permitido** todos los
  meses jubilados, haya déficit o no. Con superávit, la cascada invierte la caja del mes PRIMERO y
  la venta ocurre DESPUÉS (paso 6 del bucle); lo que sobra sobre la necesidad se GASTA y no vuelve
  a la cartera (`withdrawal_excess`).
- Con `fixed_real` **los dos modos coinciden** — el permitido se define como la necesidad, así que
  no hay techo que recorte ni sobrante que gastar. Es la propiedad que mantiene 4.15.0 bit-idéntico
  bajo cualquiera de los dos, y tiene test propio.

**El techo BRUTO con `g` mixta** (#178) necesita la dirección contraria del gross-up: `gross_up_mixed_monthly`
resuelve «qué bruto netea este NETO», y aquí se conoce el bruto y hace falta el neto. Lo resuelve
`tax::mixed_drawdown_for_gross_cap` — **paseo EXACTO por los mismos quiebros**, nunca una bisección
(§2.23 de `futurefin-failure-archaeology`: si la función es lineal a trozos, no la busques,
recórrela). El neto `F(G) = G − tax(B(G))` tiene pendiente `1 − r·g_j` mientras se vacía el tramo
`j` bajo el tipo `r`, y sus quiebros son las fronteras de capacidad (cambia `g`) y los techos de
tramo fiscal (cambia `r`): se recorren en el orden de `drain_order` en ≤ `n + |tramos|` pasos, sin
tolerancias. `the_gross_walk_is_the_exact_inverse_of_the_net_walk` ata las dos direcciones.

La rama de déficit llama al paseo directo **solo cuando el techo de verdad recorta** la venta que
la necesidad pedía; en cualquier otro caso corre el camino literal de 4.15.0, operando a operando.

## Output
```rust
pub struct ProjectionOutput {
    pub net_worth: Vec<Decimal>,         // nominal, euros del momento, index 0..=horizon_months
    pub liquid_worth: Vec<Decimal>,      // 4.8.0 (#143): Σ activos is_liquid (BRUTA) — la base del cruce; surplus_cash retirado del término en 4.12.1 (#175)
    pub contributed_capital: Vec<Decimal>, // Σ basis por activo (nominal) — desde 4.10.0/#120 PUEDE DECRECER: vender baja la base (b' = b·v_post/v_pre); «cumulative» murió con la Ola 6; surplus_cash retirado del término en 4.12.1 (#175)
    pub per_asset_series: Vec<Vec<Decimal>>, // value per asset per month (nominal)
    pub assets_depleted_month_index: Option<u32>, // 4.6.0 (#119): primer mes con déficit ≥ TODO lo drenable
    pub uncovered_deficit_total: Decimal,         // 4.6.0 (#119): undrained_cumulative final
    pub unallocated_savings_total: Decimal,       // 4.12.1 (#175): ahorro que ninguna regla absorbió, acumulado — NO entra en net_worth ni en contributed_capital; "0" con activos vivos (sumidero indestructible #176)
    // --- 5.0.0 WP1b (§B.8): LECTURAS de fase. Ninguna cambia la aritmética; todas se derivan de
    //     valores que el bucle ya tenía. `pins-4.15.json` NO las hashea (sigue probando que las de
    //     arriba no se movieron); van en `pins-5.0-outputs.json`, fixture aparte y aditivo.
    pub retirement_month_index: Option<u32>,        // primer mes jubilado (1-based) — efectivo: min(cruce, forzado)
    pub liquid_crossing_month_index: Option<u32>,   // primer mes con líquido(k−1) ≥ target(k−1) — LECTURA, no gobierna
    pub phase_transitions: Vec<(Phase, u32)>,       // [(Accumulating,0)] (+ (Partial,x)) (+ (Retired,k)); monótona
    pub withdrawal: Vec<Decimal>,                   // retirada NETA del mes = after_tax(bruto vendido) (len horizon+1, [0] = 0)
    pub withdrawal_shortfall: Vec<Decimal>,         // recorte de la REGLA — informativo (D22/D24), NO es uncovered_deficit_total; 0 con fixed_real
    pub withdrawal_excess: Vec<Decimal>,            // sobrante de rule_is_spend sobre la necesidad — 0 en ceiling y con fixed_real
    pub pension_start_month_index: Option<u32>,     // WP3: `pension.start_index + 1` (1-based), None si cae fuera del horizonte
    pub partial_retirement_month_index: Option<u32>,// WP3: primer mes de media jornada — None si la fase no se pisó
    pub warnings: Vec<EngineWarning>,               // WP3: el bucle emite RetireAtAgeUnderfunded y PartialPhaseCapitalShrinking
    // --- 5.0.0 WP3 (§B.3, §B.7): lecturas de pensión, puente, media jornada y margen ---
    pub bridge_effective_withdrawal_pct: Option<Decimal>, // 100·12·need_full_m(R−1)/L(R−1) — % ANUAL; None sin pensión+puente
    pub pension_coverage_ratio: Option<Decimal>,    // P_m(P)/(E·f(P)) — FRACCIÓN (0,6 = 60 %); None sin pensión con fecha
    pub partial_gap_target: Option<Decimal>,        // gross_up(12·gap_m(X))/SWR — informativo; Some(0) = la media jornada se paga sola
    pub partial_phase_capital_growing: bool,        // true ⟺ HUBO fase parcial Y el líquido no bajó ni un mes en ella
    pub disposable_cash: Vec<Decimal>,              // caja que el techo de aportación dejó fuera de la cascada (len horizon+1, [0] = 0)
    pub disposable_cash_total: Decimal,             // Σ de la serie. "0" son cero euros, no «no aplica»
}
```

**Las lecturas de WP3 son `Option` por disciplina, no por comodidad** (norma de la casa: `null`
nunca es cero). `bridge_effective_withdrawal_pct` es `None` sin pensión con fecha, sin base puente,
sin objetivo, sin jubilación dentro del horizonte o con `L(R−1) ≤ 0`; `pension_coverage_ratio` lo es
sin pensión con fecha o con gasto no positivo en `P`. La única EXCEPCIÓN declarada es
`partial_phase_capital_growing`, que es un `bool`: sin fase parcial vale `false` («no hay fase que
crezca»), y quien necesite distinguir «no hubo» de «hubo y menguó» mira
`partial_retirement_month_index` o el aviso `PartialPhaseCapitalShrinking`.

**Identidad del mes con techo de aportación** (`sobrante > 0`):
`sobrante = Σ aportado + no_asignado + disposable_cash(k)`. La misma se refleja en
`FirstMonthAllocation`, que ganó un campo `disposable` para no romperla en el camino de lectura.

## Solves — las inversas del motor (5.0.0 WP3 — `solve.rs`)

«¿Qué valor de X hace que la simulación cumpla Y?», **biseccionando sobre el motor entero**, nunca
sobre una fórmula cerrada que lo aproxime (hallazgo M8 de la revisión adversarial: un capital
necesario descontado a una tasa escalar es un número plausible que ninguna simulación produce).

```rust
pub const MAX_SOLVE_ITERATIONS: u32 = 24;   // el PRESUPUESTO: 24 proyecciones, no un umbral

// Mínima aportación mensual constante (techo sobre lo que la cascada invierte) tal que
// `líquido(R−1) ≥ T(R−1)`. `Ok(None)` = no hay objetivo evaluable en R−1 — NO es «cero».
pub fn required_contribution_monthly(input: &ProjectionInput, target_month: u32)
    -> Result<Option<SolveResult>, EngineError>
pub struct SolveResult { pub contribution: Decimal, pub underfunded: bool,
                         pub search_ceiling: Decimal, pub iterations: u32,
                         pub required_capital_path: Vec<Decimal>, pub warnings: Vec<EngineWarning> }

// Primer mes a partir del cual se puede dejar de aportar y aun así llegar a T(R−1).
pub fn coast_fire_month_index(input: &ProjectionInput, target_month: u32)
    -> Result<Option<CoastSolve>, EngineError>
pub struct CoastSolve { pub coast_month_index: Option<u32>, pub coast_number: Option<Decimal>,
                        pub coast_path: Vec<Decimal>, pub iterations: u32,
                        pub warnings: Vec<EngineWarning> }

// P8.b y P8.c (what-if de MCP, D30).
pub fn max_extra_monthly_expense_keeping_date(input: &ProjectionInput)
    -> Result<Option<Decimal>, EngineError>
pub fn retirement_delay_months(input: &ProjectionInput, pause: IncomePause)
    -> Result<RetirementDelay, EngineError>
pub struct RetirementDelay { pub baseline_month_index: Option<u32>,
                             pub paused_month_index: Option<u32>, pub delay_months: Option<i64> }
```

- **El invariante de la bisección** es «un extremo verificado bueno, el otro verificado malo», y se
  devuelve el bueno. Es más fuerte que fiarse de la monotonía: aunque la función objetivo tuviera
  un tramo no monótono, el valor devuelto se ejecutó y cumplió. La monotonía aporta la
  MINIMALIDAD, no la validez. Las dos rendijas están declaradas en el doc de cada solve (cascada
  hacia un activo ilíquido; cruce que adelanta la jubilación sin `crossing_is_reading_only`).
- **La cota superior NO es el sobrante del mes 1** (R5 lo dejaba abierto; decidido en WP3 **con
  medición**): es `max(neto recurrente del mes 1, max_k sobrante(k))`, con el sobrante mes a mes
  leído del `disposable_cash` de la ejecución con techo 0 — la misma sonda que el solve ya hace, así
  que no cuesta una proyección extra. Con el sobrante del mes 1 la cota **no contiene la respuesta**:
  medido en P9, techo 500 €/mes deja `líquido(599)` en 91.444 € frente a los 725.197 € de la cascada
  real, y cualquier objetivo entre esas dos cifras se declararía infra-financiado siendo alcanzable
  — el rojo falso de D17. Regresión: `the_solve_ceiling_is_the_max_monthly_surplus_not_the_first_months_headroom`.
- **El «número coast»** es `coast_path[coast_month_index − 1]`: el líquido con el que el hogar
  ENTRA en el mes de corte. Serie simulada, no descuento cerrado.
- `max_extra_monthly_expense_keeping_date` suma el extra **solo a `expense_regular_monthly`** —el
  gasto de la fase de acumulación—, ni al de jubilación ni a la necesidad del objetivo: responde
  «¿cuánto margen tengo AHORA?», no «¿cuánto puedo subir mi nivel de vida para siempre?».
- Coste medido (release, P9 a 840 meses): **≈ 395 ms** una bisección completa de 24 iteraciones.
  Por eso el plan los calcula UNA vez y los guarda en la entrada de cache (M4).

Sobre las lecturas de 5.0.0: `withdrawal(k)` son los euros NETOS que salieron de los activos ese
mes — `after_tax(bruto vendido)` —, así que con `fixed_real` **es** el drenaje de 4.15.0 visto
desde la otra cara: el que ya alimentaba `uncovered_deficit_total`.

**Las TRES magnitudes son distintas y NO se suman entre sí** (B.1.5 del plan de #207, D22/D24 —
fue el hallazgo B2 de la revisión adversarial, que las tenía confundidas):

| magnitud | qué mide | ¿resta patrimonio? |
|---|---|---|
| `withdrawal` | lo que se retiró y se gastó, NETO | — (es la salida de caja) |
| `withdrawal_shortfall` | la necesidad que la REGLA no dejó retirar (`max(0, necesidad_neta − neto que el techo permitía)`) | **NO** — es un recorte de gasto, no un impago |
| `uncovered_deficit_total` | lo que los ACTIVOS no pudieron vender de la venta intentada | **SÍ**, como siempre (deuda implícita) |
| `withdrawal_excess` | lo que `rule_is_spend` vendió POR ENCIMA de la necesidad y se gastó | — (sale de la cartera vía `withdrawal`) |

En un mes de déficit **con la venta fundada al completo** se cumple exacto
`withdrawal + withdrawal_shortfall = necesidad_neta` (y, en `rule_is_spend` con techo ≥ necesidad,
`withdrawal − withdrawal_excess = necesidad_neta`). Cuando la cartera no llega, la diferencia se va
a `uncovered_deficit_total`, no al recorte: el recorte **no crece** con el agotamiento.

El handler no publica todavía todas estas lecturas:
`jubilacion_month_index` sigue derivándose en `handlers/projection.rs` (R8 es WP5).

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
- `EngineError::InvalidWithdrawalRule` — 5.0.0 WP2: la regla trae parámetros no simulables (`pct`,
  `start_pct`/`end_pct`, `band_pct` o `adjust_pct` ≤ 0; `adjust_pct` ≥ 100). La API los acota mucho
  antes (`handlers/retirement_profile.rs`), pero el motor es una función pura y su firma admite
  cualquier `Decimal`: rechaza tipado en vez de panicar o simular otra política.
- `EngineError::UnsupportedWithdrawalRule` — **ya no la produce nadie**: WP2 implementó las cuatro
  reglas. Sobrevive porque `apps/api` la mapea junto a `UnsupportedPhase` al mismo
  `engine_feature_unavailable`; se retirará con ella en WP3.
- `EngineError::UnsupportedPhase` — 5.0.0 WP1b: `phase_plan.partial` o `phase_plan.pension` presentes (WP3)

Las dos últimas las comprueban **`project_net_worth_series` y `first_month_allocation`** antes de
mirar nada más: la segunda resuelve el mes 1 igual que el bucle, así que no puede aceptar un plan
que el bucle rechaza.

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
expense?" while compounding the assets' expected return and inflating the expense. Consumers:
`GET /v1/summary` → `financial_health.runway_months` / `runway_is_indefinite`
(`apps/api/src/handlers/summary.rs`) y `sim_kpis` (`handlers/projection.rs`). **#178 (4.12.0)**:
cada líquido viaja como `(valor, rentabilidad %, base de coste declarada)` — el BUCLE FINITO
deriva la `g` de cada activo con coste declarado (misma pareja de vías que la rama de déficit:
uniforme ⇒ camino literal bit-idéntico; mezcla ⇒ `gross_up_mixed_monthly`), mientras el UMBRAL
SWR sigue con el escalar (perpetuidad — reparto de regímenes en financial-contracts §2.4). El
mes final fraccionario de la vía mixta se mide en NETO (no existe «el» bruto de un mes mixto);
la vía uniforme conserva la fracción BRUTA histórica. Public API in the block above; **16** unit tests in-module
as of 4.8.0 (recount: `grep -c '#\[test\]' crates/engine/src/runway.rs`).

| Input | Meaning |
|---|---|
| `liquid_assets: &[(Decimal, Option<Decimal>)]` | One row per liquid asset: `(current_value, expected_annual_return_percent)`. The handler passes exactly the rows of `assets WHERE is_liquid = true` in the requested scope. |
| `monthly_expense: Decimal` | Total monthly expense to cover — in the handler, `expense_total_monthly_equivalent` (so it follows `savings_source`). |
| `annual_inflation_percent: Decimal` | `installation.annual_inflation_assumption_percent` — rango [−2, 50] desde 4.9.0 (#146; el clamp ≥ 0 del handler se retiró): con inflación negativa el gasto del runway DECRECE mes a mes y el runway se alarga (12.000/1.000 a −2 % ⇒ 12,11 meses donde el clamp publicaba 12,0). |
| `swr_pct: Decimal` (v2.3.0) | `installation.fire_settings.swr_pct` (in %) — the **same** safe-withdrawal rate the FIRE target uses (Jubilación tab), read via `installation_calendar_inflation_fire`. Only drives the infinite case. |
| `annual_expense_for_swr: Decimal` (v2.3.0) | The **annual** expense already grossed up for taxes by the handler: `gross_up_net_annual_fire(expense_total × 12, fire.tax_brackets, fire.taxes_enabled)` — the *same* gross-up as the FIRE target. With `taxes_enabled = false` it is plainly `12 × monthly_expense`. The engine never recomputes `12 × monthly_expense` itself. |

Model (each rule exists for a reason — do not "simplify" one away):

- **Nominal frame**: assets grow at their *nominal* expected return and the expense is inflated every
  month. The result is a count of months (frame-invariant), but mixing nominal returns with a
  constant expense would overstate the runway.
- **Withdraw-then-grow order**: each month pays the expense first and grows what is left — the same
  order as the simulation loop in `projection.rs` (negative cash flow drains before the multipliers
  apply), so both curves stay coherent.
- **Sequential drain (4.8.0, #128)**: each month the expense is funded by emptying the assets in
  the SAME order as `drain_from_assets` in the real simulation — lowest expected return first
  (`None` counts as 0, ties by index) — and then each remaining balance grows by ITS own
  multiplier. Until 4.7.x a value-weighted single multiplier (prorated drain) was used,
  systematically **shorter**: prorating consumes the high-return assets from month 1, while the
  real drain lets them compound untouched. Single-asset portfolios are bit-identical under both
  models. Measured: 10.000 € at 0 % + 10.000 € at 10 % vs 1.000 €/month → 21,27 months (weighted
  gave 20,80); 150.000 € at 0 % + 50.000 € at 10 % vs 2.000 €/month → 130,96 (weighted gave
  111,39). A negative individual value never "funds" the expense (its take clamps to ≥ 0) — it
  only subtracts from the total, exactly as it did under the pooled model.
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
- **Positive-return gate (4.8.0, #128)**: the SWR threshold alone no longer declares `Indefinite` —
  the liquid portfolio's value-weighted expected return must also be **strictly positive**
  (`Σ vₐ·rₐ > 0`, `None` = 0; compared without dividing, equivalent to the weighted mean since
  `A > 0` is guaranteed upstream). The Trinity/Bengen rule was validated for invested portfolios
  with positive expected real return — never for cash parked at 0 %: 300.000 € at 0 % vs
  875 €/month meets the threshold by exact equality yet runs dry in 342,86 months, and that is now
  what gets published. A balance below the threshold with a huge return is still not "infinite"
  (the threshold still gates), and inflation still only governs the **finite** loop — the SWR
  definition already carries it inside the portfolio's real return.
- **100-year cap is a floor, not a sentinel**: surviving `MAX_RUNWAY_MONTHS` (1.200) months without
  meeting the SWR threshold returns `Months(1200)` — read as "at least 100 years", not an exact
  measure and **not** `Indefinite` (the UI renders it «+100 años»). Still no epsilon and no closed
  form: `ln`-based closed forms suffer cancellation exactly at the `A·j → g` boundary; the monthly
  loop avoids it and costs microseconds.
- **The finite loop sells GROSS (4.10.0, twin of #140)**: each month's need is
  `gross_up_monthly(inflated expense, brackets, enabled, taxable_gain_ratio)` — until 4.9.x the
  threshold demanded fiscal capital while the loop spent tax-free, the exact asymmetry of #140 in
  another card. With ES brackets the canonical 12.000/1.000 scenario drops from 12 to **9,5758**
  months (and back to 12 exact with `g = 0`). The gross-up runs INSIDE the loop on the
  already-inflated expense (`gross_up` is affine — D-1).
- **Exact reduction to `A / g`** (the finite branch, **taxes off**): with return and inflation 0
  every multiplier is 1 and the sequential drain removes exactly `g` from the total each month,
  so the final fractional month reconstructs `A/g` with a single division — bit-exact **inside
  the engine**, which is where the property lives. Con impuestos el divisor es el BRUTO y la
  división simple muere.
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
0 % / 3 % → 9,89; 5 % / 3 % → 10,07 months. Threshold + gate (4.8.0, #128): 240.000 € **without
return** vs 700 €/month at SWR 3,5 % with taxes off meets the boundary exactly (840.000 = 840.000)
but fails the gate → finite, `A/g = 342,857…` (published 342,9; it was `Indefinite` until 4.7.x);
the same balance at 2 % passes the gate → `Indefinite` on the exact boundary. 1.000.000 € at 7 % vs
4.000 €/month at SWR 3,5 % → `Months(1200)` floor, since 48.000 > 35.000; with the default ES
brackets `gross_up(8.400) ≈ 10.481 €`, raising the threshold to ≈ 299.457 € of liquid balance
(pinned at the API level with 270.000 € at 2 %: taxes on → finite ≈ 612,38 months; taxes off →
`Indefinite`).

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
| `annual_inflation_percent` | `installation.annual_inflation_assumption_percent` (rango [−2, 50] desde 4.9.0/#146 — sin clamp; con deflación el real queda por encima del nominal). |

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
- Horizon derivation (`projection_horizon_months`): se resuelve **una** fecha de nacimiento — `users.birth_date` del usuario de sesión, y si es NULL la primera fila de `persons` con `birth_date` por `is_primary DESC, sort_index ASC`. Horizonte = `clamp(edad_límite − edad, 5, 70)` años × 12, con `edad_límite = horizon_lifespan_age` (85..=105, default 90 — configurable desde 4.9.0/#149 y **mudada de `installation.fire_settings` a `users.retirement_profile` en 5.0.0**; el clamp [5, 70] no se tocó, así que el eje solo muerde si `edad ≥ edad_límite − 70`). Sin fecha de nacimiento: fallback **360 meses (30 años)**. `?months=N` (12–840) lo sobreescribe. `horizon_basis` reporta la razón: `lifespan_age` (hasta 4.8.0 `lifespan_90` — un número congelado en un literal publicado) | `fallback_no_demographics` | `months_override`, con `horizon_lifespan_age` ecoado al lado. El «margen al final» NO estrena campo: es `points[último].net_worth` (+ `final_net_worth_real` en euros de hoy, paridad con simulate); «no llegó» ⟺ `assets_depleted_month_index != null` o `uncovered_deficit_total > 0`. (No existe `projection_target_age` — eliminado en v1.0.6.)
- Response includes UI-layer fields computed in the handler (not in engine): `milestones` (next 3 net-worth thresholds, **nominal**), `milestones_real` (same thresholds crossed over the **deflated** net worth = euros de hoy; empty when inflation is 0 — the web reuses `milestones`. The web picks the set from the "Inflation Adjusted" toggle), `compound_outpaces_true_savings_month_index`, `anchor_date_ymd`, `show_age_mode`, `use_age_on_x_axis`, `viewer_birth_date`. Both milestone sets are computed over the full monthly series (`points_full`), not the decimated `points`, so `reached_month_index` keeps precision under `density=hybrid`. `deflate_points_to_today` mirrors the chart's visual deflation (`ProjectionNetWorthChart.baseSeries`) but at monthly resolution.
- **Retirement drawdown** (corregido 2026-09-03: **este bullet decía «el handler pasa SIEMPRE
  `PhasePlan::classic(...)`, sin mes forzado», y dejó de ser cierto con WP5**). El plan lo construye
  el handler **desde el perfil del usuario** (`users.retirement_profile`), y decide dos cosas:
  - **el disparador** — `RetirementTrigger::LiquidCrossing` en `asap`/`pension_bridge`; con una
    estrategia por EDAD (`retire_at_age`, `coast`) pasa `AtMonth(R)` **y**
    `crossing_is_reading_only = true`, así que el cruce se sigue evaluando y publicando
    (`liquid_crossing_month_index`) pero **no jubila**. El literal que la respuesta ecoa es
    `retirement_trigger`;
  - **la base del objetivo, la regla de retirada, la pensión con fecha y la fase parcial**, todas
    del perfil.
  A partir del mes efectivo el ingreso cae a `income_retirement_monthly` (suma de `budget_entries`
  con `persists_after_retirement = true`) y el gasto pasa a `expense_retirement_monthly` (excluye
  gastos con `ends_at_retirement`). `extra_monthly_withdrawal` (el antiguo
  `retirement_monthly_withdrawal`) sigue siendo siempre 0 — la caída de ingresos por sí sola drena
  la cartera. La necesidad FIRE la construye el **servidor** (`compute_fire_need` — **no
  `compute_fire_target_nw`, que se renombró en 4.10.0/#170** — → `jubilacion_target_net_worth` en el
  response): `neto = expense_retirement − income_retirement` (modo annual_expense) o
  `neto = income − income_retirement` (modo current_income); si `neto ≤ 0` **no hay target** (`None`,
  no `max(0,…)`); si no, el objetivo se evalúa por mes sobre esa necesidad (ver §Inflación y target
  FIRE móvil). El frontend duplica la fórmula solo para el preview en vivo del formulario (paridad
  garantizada por `apps/api/tests/fixtures/fire-parity.json`).

## Performance notes (handler ↔ engine boundary)
- `project_net_worth_series` is CPU-bound (840 months × N assets × `Decimal::powd`). The handler wraps it in `tokio::task::spawn_blocking` to avoid blocking the reactor.
- `compound_outpaces_true_savings_month` is a **second projection pass** with `planning_adj = 0` and `liability.monthly_payment = 0` so the marker compares `market_growth` against a clean `income − expense` baseline. Eliminating the double pass would change the indicator's semantics; instead the handler runs both projections in parallel with `tokio::join!(spawn_blocking, spawn_blocking)`.
- The gross-up of net-annual FIRE through tax brackets uses a **closed-form per-bracket solver** (no binary search). `gross = (net − r·prev_ceiling + K) / (1 − r)`, advancing one bracket at a time until the candidate fits. Old code used 90 iterations of binary search on `Decimal`. Desde la Ola 6 (#140) vive en el ENGINE (`crates/engine/src/tax.rs`, `pub`, con el eje `taxable_gain_ratio` — la validez por tramo es `g·G ≤ techo`) y tiene **cuatro consumidores**: el target FIRE (evaluado POR MES desde #170), el drenaje bruto del bucle, y los dos umbrales SWR del runway (summary + simulate) — cuyo bucle finito también vende bruto desde esta ola. Cualquier cambio en los tramos o en el solver mueve TODOS a la vez — es intencional: una sola definición fiscal.
- `build_installation_projection_input` returns a `BuiltProjection` struct that carries `input`, `monthly_net_regular`, `asset_id_name` (Vec<(Uuid, String)>) and `planning_rows`. The handler reuses those instead of issuing a second `SELECT id, name FROM assets` and a second `SELECT planning_flows` (deleted with Fase 2.3). Desde v2.2.0 también expone `effective_savings_source` + (desde 3.9.0) `savings_income_basis` / `savings_expense_basis` — que **sustituyen** al escalar `savings_source_months_with_data`: con ventanas configurables por lado no existe *un* número de meses — (fuente **tras** el fallback, serializadas en `ProjectionSeriesResponse`) y `debt_service_monthly` (cuotas de pasivos activos; **no** es input del engine, que amortiza los pasivos aparte), que consume `assets_projection_context` para los caps `months_expense`.
- Initial queries in `get_projection_series` (installation row, user birth_date, household birth_date) run concurrently via `tokio::try_join!`.
