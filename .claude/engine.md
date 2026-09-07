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
- `target.rs` — el **número FIRE clásico**: UNA base (perpetuidad sobre el gasto de jubilación
  indexado), sin restar la pensión con fecha y sin disparar nada (5.0.0 E4, §El número FIRE
  clásico).
- `solve.rs` — las inversas por bisección sobre el motor (5.0.0 WP3, podadas en E4: §Solves).
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

**La puerta de degeneración** (`crates/engine-stochastic/tests/degeneration.rs`) corre los 25 casos
de la batería del motor por los dos caminos y compara `net_worth` y `liquid_worth` mes a mes en
todo el horizonte más las decisiones discretas. Medido: **máximo 1,5e-7 € en 840 meses** (P9), y
las **seis** decisiones discretas (`retirement_month_index`, `liquid_crossing_month_index`,
`assets_depleted_month_index`, `phase_transitions` y —desde E1— `failure_month_index` y
`failure_kind`) coinciden EXACTAMENTE en todos los casos. Las dos últimas no admiten la holgura de
±1 mes que se les tolera a los índices: el veredicto de un camino es lo que Monte Carlo CUENTA, y
un mes de holgura ahí es un mes de holgura en la fecha que la app publica. La
única fila con cota relativa es `P14_techo_numeric`, sintético (activo en el techo de
`NUMERIC(18,4)` al 20 % durante 70 años ⇒ patrimonio ~3,5e19 €), donde el espaciado de los `f64`
ya supera el euro: allí la cota es `1e-12` relativa y se mide 2,0e-14.

Hallazgos que esta puerta cazó, **todos de la misma familia**: un booleano publicado colgando de
una comparación exacta entre dos cantidades que los dos tipos calculan por caminos distintos.

1. La venta mixta con techo deducía «¿se vendió el techo entero?» comparando
   `gross_monthly >= gross_cap`. Ahora el booleano lo publica el paseo
   (`MixedGrossDrawdown::cap_exhausted`), que es quien lo sabe. Valía 8.138 € en P15.
2. El **agotamiento** se decidía con `venta_bruta >= drenable` ANTES de vender. En el aterrizaje
   exacto —el capital que iguala al euro el objetivo del puente— `Decimal` decía `None` y `f64`
   decía `Some(120)`, y `f64` es el tipo sobre el que corre cada camino de Monte Carlo: el mismo
   plan salía «arruinado» en el fan chart y «perfecto» en la línea. Ahora se mide DESPUÉS de
   vender, sobre los saldos (§Agotamiento).
3. `partial_phase_capital_growing` compara el líquido de cierre de dos meses consecutivos, y en
   una serie PLANA los dos valores son iguales en `Decimal` y difieren en un ulp en `f64`. El `<`
   lo decide ahora el tipo (`MoneyOps::strictly_below`, tolerancia relativa `1e-9` declarada en el
   crate estocástico), igual que el `==` de las `g`. Eran 5 de 2.901 entradas del corpus
   diferencial.

Los dos pines dorados no se movieron con ninguno de los tres.

### Monte Carlo (5.0.0 WP6)

`project_percentile_bands(input, volatilities, McConfig) -> McOutcome` corre `paths` caminos del
MISMO bucle con los factores de crecimiento sorteados. **Un shock de mercado común por mes** (D11):
un solo `z_k ~ N(0,1)` que todos los activos viven a la vez, `f_ik = d_i·exp(σ_i·z_k − σ_i²/2)` con
`σ_i = annual_volatility_percent/100/√12`, `m_i` la raíz doceava del propio motor
(`monthly_growth_multiplier`, no una copia) y `d_i = m_i·exp(σ_i²/2)` la **deriva**.

**La rentabilidad declarada es COMPUESTA (CAGR)** — decisión M8 del modelo v2 (owner, 2026-09-06):
lo que el usuario escribe es el crecimiento geométrico, el número que publican los fondos. Por eso
el sorteo no centra la MEDIA en `m_i` sino la **MEDIANA**: la corrección de Itô se come la deriva
justo hasta dejar `mediana(f_ik) = m_i` **exacta**, y `E[f_ik] = d_i = m_i·exp(σ_m,i²/2)` es la
**prima de varianza** (con σ anual 15 %: +0,094 % al mes, +11,9 % a 10 años). Consecuencia
publicable: **la línea determinista es la central de la banda**, no un techo.

**La conversión CAGR → media aritmética vive en UN SOLO SITIO**: `PathEngine::new`
(`crates/engine-stochastic/src/mc.rs`), sobre la σ **MENSUAL** — sumarla a la tasa anual
(`CAGR + σ_anual²/2`) es la fórmula equivocada y 12 veces mayor de lo que el factor mensual
necesita. Ni `crates/engine` ni `deterministic_growth_multipliers`/`simulate_f64` la aplican, y por
eso la **puerta de degeneración no cambió de valor** con este cambio: no pasa por `PathEngine`.
`σ_i = 0` ⇒ `d_i = m_i` y `f_ik = m_i`, las dos por **rama explícita**. La identidad «mediana del
patrimonio = línea determinista» es exacta solo **sin flujos** (`mc_median_is_the_deterministic_line`
la mide: mediana 197.313 € frente a 196.715 € deterministas, +0,30 %, con 2.500 caminos); con
aportaciones o retiradas la cascada, el drenaje y la fiscalidad no son lineales y la mediana se
separa unos pocos puntos porcentuales (±2–4 % a 20–35 años).

Semilla estable por usuario (D23, `seed_for(installation_id, user_id)` = FNV-1a +
finalizador de splitmix64), un flujo ChaCha8 propio por camino (ampliar la muestra no reescribe la
que había), normales por Box–Muller y percentiles por **rango más cercano** (siempre un valor
observado, nunca una interpolación). `McOutcome` publica bandas **puntuales** p10/p50/p90 de
`net_worth` y `liquid_worth` (la p50 NO es un camino y no cumple ninguna identidad contable),
`success_probability` con su intervalo de Wilson (`wilson_low`, `half_width_pp`), `failures_by_kind`,
fallo acumulado por edad (`cumulative_failure_by_age`) y las dos lecturas de cobertura (D24, B2).

**`McOutcome` v2 (E9): éxito = CERO fallos F1/F2/F3 en el camino** —
`success_probability` = caminos con `failure_month_index.is_none()` / N, la MISMA fuente que
clasifica el bucle del motor (F1 `PortfolioDepleted`, F2 `InitialRateExceeded`, F3
`RuleBelowNeed`; F2 y F3 solo pueden firmar en `R`). Ya NO depende de si el hogar se jubila: con la
API v2 todo plan se sortea con
`RetirementTrigger::AtMonth` —un mes forzado que resuelve el solver externo
(`solve_mc::valid_retirement_month`) antes de llegar aquí—, así que «no jubilarse nunca» dejó de
ser un desenlace posible del sorteo. Los campos que D22/D24 usaban para separar esa pregunta
(`never_retired_probability`, `success_given_retired`) y la infra-financiación de una edad fija
(`underfunded_probability`, publicado `None` desde que E1 retiró `RetireAtAgeUnderfunded` del
motor) **se retiraron de `McOutcome`**: esa lectura es ahora `1 − éxito(R)` del solver estocástico.

`failures_by_kind: [u32; 3]` cuenta el PRIMER motivo de cada camino fallido, en el orden
`KIND_PORTFOLIO_DEPLETED` / `KIND_INITIAL_RATE_EXCEEDED` / `KIND_RULE_BELOW_NEED` —los MISMOS
índices que `SuccessAt::by_kind` de `solve_mc`, para que las dos capas no diverjan— y suma
exactamente `paths − paths·success_probability`. `wilson_low`/`half_width_pp` reutilizan
`solve_mc::SuccessAt::new` (Wilson nunca se reimplementa en `mc.rs`): con `failures == 0`,
`wilson_low` colapsa a la forma cerrada `n/(n+z²)` — **estrictamente menor que 1**, nunca «100 %
seguro» solo porque la muestra no vio ningún fallo.

`cumulative_failure_by_age` **sustituye a `depletion_probability_by_age`** (que solo contaba F1):
fracción ACUMULADA de caminos con `failure_month_index ≤ mes`, cada `FAILURE_STEP_MONTHS` (60,
renombrada desde `DEPLETION_STEP_MONTHS`) meses desde el ANCLA. El ancla es el mes de jubilación
FORZADO del plan (`retirement_trigger.forced_month()`); si el `ProjectionInput` todavía trae
`RetirementTrigger::LiquidCrossing` (llamantes/tests legacy que no han migrado al mes forzado de la
v2), el ancla es la jubilación efectiva del camino DETERMINISTA como ANTES —y, a falta de ella, la
mediana de los sorteados—. La última fila SIEMPRE es el horizonte, y coincide al bit con
`1 − success_probability`.

**Las dos lecturas de cobertura cuentan la necesidad que la CARTERA no pudo fundar**, no solo la
que la regla rechazó. `months_below_need_p50` cuenta los meses con `recorte + descubierto > 0` (sin
cambios en E9). `withdrawal_to_need_ratio_p50` se **corrigió en E9 (bug B2)**:
`Σ max(0, w − excess) / Σ max(0, w + s + u − excess)`, cada término CLAMPADO a `≥ 0` MES A MES antes
de sumar. Bajo `rule_is_spend` (D5), `withdrawal` incluye el EXCESO sobre la necesidad
(`withdrawal_excess`, D24) —la regla ES el gasto y vende `permitido` aunque sobre—, y antes del fix
ese exceso contaba en el numerador Y el denominador sin descontarlo, inflando la cobertura: medido
en el mismo hogar y la misma semilla, **0,9888 → 0,9793**. La identidad del motor
`w + s + u − excess = need_net` (`crates/engine/tests/fuzz_invariants.rs`) es la que exige el clamp
MES A MES: `need_net` puede ser NEGATIVO desde el mes en que la pensión supera el gasto. **De este
crate no sale un euro**: todo lo publicado es estadístico. Lo que el modelo NO representa —colas
gruesas, autocorrelación, correlación imperfecta entre activos (con un `z` común es exactamente 1),
bootstrap histórico, volatilidad de IPC/ingresos/gasto, rebalanceo— está escrito en el doc del
módulo `mc`, no en un comentario suelto.

**F3 se juzga SOLO en `R`, el primer mes jubilado** (decisión C10 del owner, 2026-09-07) — la
misma marca que F2, y por la misma razón: los dos preguntan por la FECHA. Después de `R` el único
motivo vivo es F1, y el recorte que la regla haga más adelante viaja como LECTURA
(`withdrawal_shortfall`), nunca como fracaso.

**Por qué se movió, con la medición que lo condenó.** Mes a mes, F3 era un problema de BARRERA y no
una medición del plan: el permitido sigue a `L(k−1)`, que en Monte Carlo pasea con ~17 % de
volatilidad frente a una deriva de ~0,8 %/año, así que sobre cientos de meses la probabilidad de
cruzarla alguna vez tiende a 1 por la varianza. `PercentOfBalance`/`Hybrid`/`Guardrails` con
`spend_mode = rule_is_spend` NUNCA disparan F1 (no pueden agotar la cartera, por construcción), así
que la barrera era el ÚNICO veredicto posible y decidía sola: un hogar de prueba con la necesidad
pegada al 4 % del capital inicial fallaba por F3 en el **94,9 %** de los caminos, y sobre la demo
sintética con «3,5 % del saldo» el capital necesario hoy salía **2,52 M€** (620 k€ con
`fixed_real`), con **67 fallos de 2.500 caminos, todos F3** y el primero siempre antes de la
pensión (mediana: mes 293). Con F3 solo en `R`, el mismo hogar pide **860 k€** y el hogar de prueba
tiene éxito 1,0 con la cobertura intacta (70 meses de recorte, ratio 0,9793 — cifras BYTE a byte
las de antes: lo que se movió es el veredicto, no la simulación).
`mc_percent_of_balance_never_ruins_but_cuts_the_spending` mide esa mitad;
`mc_f3_is_a_property_of_the_plan_not_of_the_draw` mide la otra —con `R` fijo, `L(R−1)` es el mismo
en todos los caminos, así que F3 es determinista: o fallan todos o no falla ninguno—. **Retirar F3
del todo NO era la alternativa**: colapsaría al suelo de F2 y el modo porcentual sería infalible.

Sigue en pie la separación que el modelo v2 hace visible: «la cartera no se agota» y «el plan tiene
éxito» son preguntas distintas, y bajo una regla que por diseño no puede arruinar a nadie el
recorte se mide en `months_below_need_p50` / `withdrawal_to_need_ratio_p50`, no en el éxito.

**El colchón de caja se retiró en 5.0.0 antes de publicarse** (decisión del propietario,
2026-09-06): la caja es un activo más y las reglas de ahorro fijan cuánto se guarda — no hay un
mecanismo aparte que la rellene. No queda ni un tipo, ni un campo, ni un test suyo en
`crates/engine` ni en `crates/engine-stochastic`.

### Los solves estocásticos (5.0.0 E6 — `crates/engine-stochastic/src/solve_mc.rs`)

Donde `mc` pregunta «¿cómo de ancha es la banda?», `solve_mc` pregunta **«¿cuándo me puedo
jubilar?»** (decisión M1 del owner, «definición A»). **Misma doctrina que `crates/engine/src/solve.rs`**
—bisección sobre el motor entero, extremo VERIFICADO, presupuesto de iteraciones— con una
diferencia que cambia el vocabulario: cada evaluación de la función objetivo no es una proyección
sino un **sorteo de N caminos**, y su respuesta no es un booleano sino una **proporción con barra
de error**.

```rust
pub fn retiring_at(input: &ProjectionInput, month: u32) -> ProjectionInput   // AtMonth(k) + cruce como lectura

pub fn success_at_month(input, vols: &[Option<f64>], mc: &McConfig, month: u32) -> Result<SuccessAt, McError>
pub fn success_by_retirement_month(input, vols, mc, grid: &[u32]) -> Result<Vec<SuccessAt>, McError>
pub struct SuccessAt { pub month: u32, pub paths: u32, pub failures: u32, pub success: f64,
                       pub wilson_low: f64, pub half_width_pp: f64,
                       pub rule_of_three_upper: Option<f64>, pub by_kind: [u32; 3] }
impl SuccessAt { pub fn new(month, paths, failures, by_kind) -> Self; pub fn meets(&self, threshold_pct: u32) -> bool }

pub fn valid_retirement_month(input, vols, search: &McConfig, confirm: &McConfig,
                              threshold_pct: u32, k_min: u32) -> Result<RetirementDateSolve, McError>
pub struct RetirementDateSolve { pub month: Option<u32>, pub success: f64, pub wilson_low: f64,
                                 pub half_width_pp: f64, pub rule_of_three_upper: Option<f64>,
                                 pub predecessor_success: Option<f64>, pub date_is_approximate: bool,
                                 pub draws_search: u32, pub draws_confirm: u32,
                                 pub failures_by_kind: [u32; 3], pub best_effort: Option<(u32, f64)> }
```

- **Un camino falla ⟺ `SimOutput::failure_month_index.is_some()`.** Los tres motivos (F1/F2/F3) los
  clasifica el bucle del motor; aquí solo se CUENTAN (`by_kind`, en el orden
  `KIND_PORTFOLIO_DEPLETED` / `KIND_INITIAL_RATE_EXCEEDED` / `KIND_RULE_BELOW_NEED`). Sin
  `PhasePlan::initial_rate` no existe F2 y ningún camino puede fallar por tasa inicial. **F2 y F3
  se deciden en `R`** (C10), así que con `R` fijo son deterministas respecto al sorteo —`L(R−1)` es
  el mismo en todos los caminos cuando `R = 1`— y lo único que la volatilidad decide después es F1.
- **El escenario** es siempre `retiring_at`: `retirement_trigger = AtMonth(k)` **y**
  `crossing_is_reading_only = true`. Sin lo segundo el motor conserva la unión `cruce || k ≥ forzado`
  y un camino afortunado se jubilaría antes de `k` — `éxito(k)` mediría `min(cruce, k)`.
- **Umbral (C3)**: `u < 100 ⇒ wilson_low ≥ u/100`; `u = 100 ⇒ failures == 0`, y ahí se publica la
  cota de la regla de tres (`3/N`). El rango 80–100 lo valida el perfil, no el motor: aquí un
  umbral fuera de rango **no se recorta**, simplemente no lo cumple nadie.
- **Wilson, y por qué no la normal.** Con `p̂ = 1` la aproximación normal da una barra de error
  EXACTAMENTE cero («100 % seguro con 2.500 caminos»). Wilson no degenera y además colapsa a una
  forma cerrada que conviene tener escrita: **`wilson_low = n/(n + z²)`** con `z = 1,96`
  (`2500/2503,8416 = 0,998466`, barra **0,1534 pp**; `500/503,8416 = 0,992376`). De ahí sale la
  cota que hay que tener presente al elegir presupuestos: **un umbral `u < 100` es inalcanzable con
  menos de `z²·u/(1−u)` caminos** — 73 para el 95 %, 381 para el 99 %. Los 500/2.500 del plan
  cubren el rango entero. `half_width_pp` es la distancia del estimador puntual a la cota INFERIOR
  (el lado que decide), no media anchura: el intervalo es asimétrico.
- **Números aleatorios comunes.** `path_rng(seed, p)` no depende ni de `k` ni de `paths`, así que
  el camino `p` vive el mismo mercado en todas las evaluaciones y **los 500 de la búsqueda son un
  prefijo bit a bit de los 2.500 de la confirmación**. Es lo que hace comparables `éxito(k)` y
  `éxito(k+1)` y lo que permite que la confirmación desmienta a la búsqueda sin ser otra muestra.
  Regresión: `common_random_numbers_make_the_confirmation_a_superset`.

**Las cinco fases de `valid_retirement_month`:**

| fase | qué hace | presupuesto |
|---|---|---|
| A | bracket de 60 meses sobre `[k_min, H]`, **cerrando siempre en `H`**: el primer mes de la rejilla que cumple | ≤ 15 sorteos de `search` |
| B | refinado ANUAL dentro del bracket, barrido de abajo arriba | ≤ 4 |
| C | bisección MENSUAL, helper único `bisect_month(lo_fails, hi_ok, …)`, invariante «`lo` falla, `hi` cumple, se devuelve `hi`» | ≤ 6 |
| D | confirmación con `confirm`; si no cumple, avanza mes a mes ≤ 12 veces y, si aun así no cierra, devuelve el que más cerca quedó con `date_is_approximate = true` | 1 + ≤ 12 |
| E | auditoría de `k−1` con `confirm` → `predecessor_success` (`None` en el suelo: no hay predecesor, **nunca un 0**) | ≤ 1 |

- **`k_min` lo pone el LLAMANTE**: `1` sin puente, `max(1, P − 12·bridge_max_years)` con puente
  (C2). **Es el único sitio donde `bridge_max_years` acota la FECHA** — el tope de tasa inicial que
  el puente levanta es cosa del motor (`InitialRateGate::bridge`). Un `k_min > H` devuelve
  `month: None` sin sortear nada.
- **Sin fecha en el horizonte ⇒ `month: None`, jamás un 0** (un 0 se leería como «ya puedes»), y se
  publica `best_effort: (mes, éxito)` con la mejor observación —elegida por `wilson_low`, empate al
  mes más temprano— además de `failures_by_kind`, que dice POR QUÉ no la hay.
- **Lo que se garantiza es «un mes VERIFICADO que cumple», NO el mínimo demostrable.** Aquí la
  monotonía ni siquiera se supone: un «Próximo» es un flujo en un mes absoluto, una fase parcial con
  base de gasto regular se encarece al alargarse, y con la inflación por encima del crecimiento neto
  el éxito **decrece** con `k` en tramos enteros. `predecessor_success` es el único dato honesto
  sobre la minimalidad. Regresión: `a_non_monotone_success_curve_still_returns_a_verified_month`.

**Coste medido** (release, P9 a 840 meses, `tests/timing_mc.rs::the_date_solve_costs_what_the_plan_says`):

```text
  un sorteo de éxito(k):   500 caminos ≈ 100–110 ms      2.500 caminos ≈ 445–465 ms
  cotas derivadas:  típico (12+2) ≈ 2,1–2,2 s      peor caso (25+14) ≈ 8,7–9,2 s   (plan: ≤ 3,5 s / ≤ 10 s)
  solve real A→E:   12–14 sorteos de 500 + 2 de 2.500  ⇒  1,8–1,9 s
```

El test IMPRIME estos números en cada ejecución en vez de afirmarlos: son de una máquina concreta y
un rango es lo honesto. Lo que no cambia es la FORMA — el coste de una fecha es
`draws_search · t(500) + draws_confirm · t(2.500)`, y los dos contadores se publican en
`RetirementDateSolve`.

La maquinaria del sorteo (`PathEngine`) se construye **una vez por presupuesto** y se sostiene todo
el solve: entre evaluaciones solo se reescribe `sim.phase_plan.retirement_trigger`. Por eso
`PathEngine` es `pub(crate)` — no es API pública del crate y `lib.rs` no lo reexporta.

#### Capital necesario (5.0.0 E7 — `crates/engine-stochastic/src/needed_capital.rs`)

La pregunta simétrica de la fecha (decisión M9 del owner, corrección C4 del panel y **decisión C9
del owner, 2026-09-07**): fijado el mes de jubilación, **¿cuánto capital hay que TENER a esa edad?**.
Se responde igual —bisección sobre el motor entero, extremo VERIFICADO, presupuesto de iteraciones—
pero moviendo el CAPITAL en vez de la FECHA: se escala el patrimonio LÍQUIDO por un factor `λ` hasta
que el plan cumple el umbral.

```text
  λ*        = mín{λ : éxito_condicionado(escalar_líquido(λ), k) ≥ umbral}
  needed(k) = liquid_worth[k−1] de project_net_worth_series(retiring_at(scale_liquid_assets(input, λ*), k))
```

- **CONDICIONADA a llegar (C9), y esto es lo que hace la cifra legible.** Para el nodo `k` la
  acumulación hasta `k−1` **no se sortea**: los factores de los meses `1..k−1` son los deterministas
  del motor (`monthly_growth_multiplier`, los mismos que `deterministic_growth_multipliers`) y el
  sorteo arranca en `k` con jubilación forzada en `k`, con los mismos números aleatorios comunes
  (`PathEngine::set_stochastic_from_month`; el RNG **se consume también** en los meses del prefijo,
  para que el camino `p` vea en el mes `k` el mismo `z` en cualquier nodo). Consecuencia: todos los
  caminos llegan al cierre de `k−1` con el MISMO líquido, la puerta F2 se evalúa una vez sobre él y
  el importe publicado **es** `X*` —lo que hay que tener—, no la mediana de una nube de llegadas.
  Sortear también la acumulación (lo de antes de C9) arrastraba su dispersión (**×3,19 a 30 años**
  con σ 17 %) y el sobrecoste de Wilson sobre ella (**×1,24**): en la batería P9 la curva salía
  monótona creciente de **2,9 M€ a 93,5 M€** en euros de hoy entre el mes 1 y el 840, y condicionada
  se queda en 2,4–2,9 M€ bajando a 737 k€. **`k = 1` no se mueve ni un euro** (no hay prefijo que
  fijar). **El coste no cambia**: 15,62 s → 15,68 s en la curva de 14 nodos de P9 (release) — el
  bucle recorre el horizonte entero igual, solo que sin shock.
- **La FECHA NO es condicionada**: `valid_retirement_month` sigue siendo la definición A (cada camino
  con su acumulación, sorteada desde el mes 1). Por eso la curva **no tiene por qué cruzar** la
  trayectoria del patrimonio en la fecha del plan: son dos preguntas.

```rust
pub fn scale_liquid_assets(input: &ProjectionInput, lambda: f64) -> ProjectionInput

pub fn needed_liquid_at_month(input, vols: &[Option<f64>], search: &McConfig, confirm: &McConfig,
                              threshold_pct: u32, k: u32) -> Result<NeededCapital, McError>
pub fn needed_capital_today(input, vols, search, confirm, threshold_pct) -> Result<NeededCapital, McError>
pub fn needed_capital_curve(input, vols, mc: &McConfig, threshold_pct: u32, grid: &[u32])
                              -> Result<Vec<NeededCapital>, McError>

// El eje que la definición condicionada necesita (C9). `stochastic_from_month <= 1` == la de siempre.
pub fn success_at_month_from(input, vols, mc: &McConfig, month: u32, stochastic_from_month: u32)
                              -> Result<SuccessAt, McError>          // solve_mc
pub fn run_path_from(input, vols, config: &McConfig, path_index: u32, stochastic_from_month: u32)
                              -> Result<SimOutput<F64Money>, McError> // mc

pub struct NeededCapital { pub month: u32, pub lambda: Option<f64>,
                           pub amount_nominal: Option<Decimal>, pub amount_today: Option<Decimal>,
                           pub absent_reason: Option<&'static str>,
                           pub success_at_lambda: Option<SuccessAt>,
                           pub capital_is_approximate: bool,
                           pub draws_search: u32, pub draws_confirm: u32 }
```

- **Qué escala `λ` y qué no.** Solo los activos con `is_liquid == true` (#143): la vivienda no es el
  stock que el drenaje vende y multiplicarla movería el patrimonio publicado sin mover un euro de la
  capacidad de jubilarse. Y de cada activo líquido se escalan **el valor Y la base de coste**
  (`purchase_price`): mover el valor dejando la base quieta subiría la `g_i = 1 − b_i/v_i` que
  gobierna el gross-up del drenaje y fabricaría una **plusvalía fantasma** que encarece el capital
  necesario por un artefacto del método. Con las dos, `g_i` es invariante exacta y el neto de una
  liquidación escala **exactamente** por `λ` (regresión con números a mano:
  `scaling_moves_the_basis_with_the_value_so_no_phantom_gain_appears`, 88.000 → 176.000 € frente a
  los 168.000 € del contrafactual sin base). Ingresos, gastos, deuda, «Próximos» y las reglas de la
  cascada **no se tocan**.
- **El importe es el líquido REAL de la trayectoria escalada, no `λ*·L_det(k−1)`.** Se paga UNA
  proyección `Decimal` de más por cifra publicada (~12,6 ms en P9) y a cambio: (a) es exacto para
  todo `k` —el producto solo coincide en `k = 1`, porque los flujos que no escalan (ahorro, gasto,
  deuda, «Próximos») no han intervenido todavía—; (b) **los nodos tardíos no se rompen**: en un
  hogar cuyo camino ACTUAL se agota antes del horizonte (P9 hacia el mes 800, ingreso plano contra
  gasto indexado, #139) el producto valdría `λ·0 = 0 €` y el nodo saldría como ausencia aunque el
  hogar escalado sí tenga cartera ahí. Regresión:
  `the_curve_uses_the_scaled_liquid_so_late_nodes_are_not_zero`.
- **Hoy = `k = 1`.** `needed_capital_today` es un envoltorio literal de `needed_liquid_at_month(…, 1)`,
  no una segunda definición: la cifra que Jubilación, Resumen y Proyección enseñan sale de la MISMA
  bisección que la curva. En `k = 1` el importe coincide con `λ*·L(0)` —el mes 0 es el estado
  inicial— y no hay nada que deflactar (el factor en el índice 0 es 1).
- **Fases y presupuesto.** (A) bracket sobre `λ` desde 1: duplicando ≤ 12 veces si no cumple,
  halvando ≤ 8 si ya cumple; (B) bisección ≤ 12 pasos con `search` (500), extremo alto siempre
  verificado; (C) confirmación con `confirm` (2.500) y, si desmiente a la búsqueda, avances de
  **+2 %** hasta 6 veces — si aun así no cierra se publica el que más cerca quedó con
  `capital_is_approximate = true`. La **curva** usa un solo presupuesto (`search`), **no confirma**
  (`draws_confirm == 0`, `capital_is_approximate` siempre `false`) y **arranca cada nodo en el `λ*`
  del anterior** (warm start, 8 pasos de bisección en vez de 12); el primer nodo va en frío.
- **Rejilla del llamante**, en su orden y con sus repeticiones (la API pasa cada 60 meses ∪ `{k*}`):
  este crate no sabe de fechas de nacimiento y no se inventa un muestreo.
- **Redondeo a cientos HACIA ARRIBA** (D4 enmendado), los dos importes por separado: un capital
  necesario redondeado a la baja quedaría por debajo del umbral que promete. `amount_today =
  amount_nominal / inflation_factor_at_month_index(π, k−1)` — el MISMO factor del motor,
  `(1 + π/100)^((k−1)/12)`, no una copia.
- **Ausencias, nunca un 0 €**: `no_liquid_assets` (el hogar no tiene activos líquidos —se decide
  sin sortear, escalar cero es cero— o el hogar ESCALADO llega a `k−1` sin líquido),
  `threshold_unreachable` (ni `2^12` veces la cartera cumple), `month_beyond_horizon` y
  **`already_covered`** (ni `1/2^8` veces la cartera lo INCUMPLE: los ocho halvings cumplen todos,
  no hay extremo malo y `λ*` no existe en la rejilla ⇒ **no hace falta capital adicional hoy**).
  Las tres primeras son «este método no puede medirlo»; la cuarta es una RESPUESTA. Hasta la
  corrección de 5.0.0 ese caso devolvía el ÚLTIMO halving como si fuera `λ*` y la curva publicaba
  `liquid_worth[k−1]` de un hogar escalado a casi cero — o sea el ahorro acumulado de la nómina:
  en la demo, cinco nodos CRECIENTES tras la fecha (485.800 → 2.902.400 €) leídos como «a los 86
  necesitas 2,9 M€». El warm start lo componía nodo a nodo (`λ` heredado ÷ `2^8` otra vez), y por
  eso hoy tiene suelo: **`WARM_LAMBDA_FLOOR = 1`**, el `λ` del hogar real.
- **Caveat de la cascada, declarado y no resuelto**: escalar supone que el reparto se mantiene
  **proporcional**, y eso vale mientras ninguna regla toque su tope. Con un `AllocationCap::Amount`
  un `λ` mayor **llena el tope antes** y desvía el resto a otro destino con otra rentabilidad y otra
  fiscalidad; los topes `MonthsExpense`/`IncomeMultiple` no escalan en absoluto porque se definen
  sobre el gasto o el ingreso. Reescalar los topes sería inventarse una regla que el usuario no
  configuró.
- **La monotonía tampoco se supone.** «Más capital ⇒ más éxito» casi siempre, pero con topes por
  importe un `λ` mayor redirige aportaciones, y la medición es muestral. Lo que se garantiza es **un
  `λ` VERIFICADO que cumple**, no el mínimo demostrable — la misma frase que gobierna `solve.rs` y
  `solve_mc`.
- **Coste.** Cada `λ` es un sorteo completo y además **reconstruye** el `PathEngine`: cambia la
  ENTRADA, no solo el trigger, así que el atajo de `solve_mc` no aplica (el sobrecoste es la
  conversión de la entrada y el buffer `meses × activos`, despreciable frente a los caminos).
  Medido en release sobre P9 a 840 meses
  (`tests/timing_mc.rs::the_needed_capital_solve_costs_what_the_plan_says`):

  ```text
    capital necesario HOY · umbral 95 ⇒ λ* 37,75 · 2.906.800 € · 19×500 + 1×2.500 ⇒ 2,5–2,8 s
    capital necesario HOY · umbral 80 ⇒ λ* 20,60 · 1.586.100 € · 18×500 + 1×2.500 ⇒ 2,4–2,8 s
    curva de 14 nodos (cada 60 meses + horizonte) · 149 sorteos de 500        ⇒ 16,1 s
  ```

  El plan pedía ≤ 3 s para la cifra de hoy (se cumple) y ≈ 12 s para la curva (**16,1 s medidos**:
  19 sorteos del nodo frío + 10 por cada uno de los 13 calientes, más una proyección `Decimal` por
  nodo). La curva es nivel 2 y se calcula en segundo plano; `WARM_LAMBDA_BISECTION_DRAWS` se queda
  en 8 — bajarlo a 6 cuadraría el número, y ajustar un presupuesto para que cuadre un número es
  exactamente lo que esta casa no hace. En P9 los fallos son todos F1 (`by_kind = [51, 0, 0]`): con
  70 años de horizonte, gasto indexado al 2,5 % y pensión plana, quien manda es la supervivencia de
  la cartera, no la puerta de tasa inicial.

  **La forma de la curva de P9, ya sin el artefacto**: `λ*` BAJA con la edad (37,75 en el mes 1 →
  30,70 en el 840) y el importe en euros de hoy SUBE (2,91 M€ → 93,5 M€), porque un hogar cuyo gasto
  indexado acaba superando su ingreso plano necesita un colchón cada vez mayor para llegar a esa
  edad sin agotarse. Con el producto `λ*·L_det(k−1)` la curva salía casi plana (4–5 M€) y el nodo
  del horizonte, ausente: las dos cosas eran el artefacto, no el hogar.

#### Aportación mínima, mes de coast, inicio de la jornada reducida (5.0.0 E8 — `strategy_solves.rs`)

Las tres preguntas que una ESTRATEGIA concreta añade a «¿cuándo me puedo jubilar?» (M10/M11/M12),
las tres con el mismo criterio —el umbral de éxito, `SuccessAt::meets`— y la misma doctrina
(bisección sobre el motor entero, extremo VERIFICADO, presupuesto de iteraciones).

```rust
pub fn minimum_extra_contribution(input, vols, search: &McConfig, confirm: &McConfig,
                                  threshold_pct: u32, r: u32) -> Result<ContributionSolve, McError>
pub struct ContributionSolve { pub month: u32, pub extra_monthly: Option<Decimal>, pub underfunded: bool,
                               pub search_ceiling: Decimal, pub success_at_solution: Option<SuccessAt>,
                               pub draws_search: u32, pub draws_confirm: u32 }
impl ContributionSolve { pub fn warning(&self) -> Option<StrategySolveWarning> }

pub fn coast_stop_month(input, vols, search, confirm, threshold_pct, r: u32) -> Result<CoastSolve, McError>
pub struct CoastSolve { pub retirement_month: u32, pub stop_month: Option<u32>,
                        pub freed_saving_monthly: Option<Decimal>, pub success_at_solution: Option<SuccessAt>,
                        pub warnings: Vec<StrategySolveWarning>, pub draws_search: u32, pub draws_confirm: u32 }

pub fn earliest_partial_start(input, vols, search, confirm, threshold_pct) -> Result<PartialSolve, McError>
pub struct PartialSolve { pub start_month: Option<u32>, pub phase_success: Option<SuccessAt>,
                          pub full_retirement: Option<RetirementDateSolve>,
                          pub warnings: Vec<StrategySolveWarning>, pub draws_search: u32, pub draws_confirm: u32 }

pub enum StrategySolveWarning { CoastNotReachable, PartialNeverStarts,
                                PartialNeverFullyRetires, RetireAtAgeUnderfunded }
impl StrategySolveWarning { pub fn code(self) -> &'static str }   // contrato de cable

// Los tres escenarios, cada uno una mutación y en un solo sitio:
pub fn contributing_extra(input, extra: Decimal, r: u32) -> ProjectionInput
pub fn stopping_at(input, stop: u32) -> ProjectionInput
pub fn partial_starting_at(input, start_month: u32) -> ProjectionInput
```

- **Una bisección, escrita UNA vez.** `bisect(lo_fails, hi_ok, max_draws, midpoint, meets)` es
  genérica sobre el eje: el mes lo parte `month_mid`, el importe `amount_mid`, y las dos guardas
  devuelven `None` cuando ya no queda candidato interior. `solve_mc::bisect_month` **no** se
  reutiliza porque es privado de ese módulo; lo que se reutiliza es su invariante, ahora escrito una
  sola vez para los tres ejes.
- **Cada evaluación RECONSTRUYE el `PathEngine`, y aquí no hay atajo.** Los tres ejes viven en el
  `SimInput` convertido (no en `retirement_trigger`), así que cada candidato es una entrada distinta
  y se pasa por `success_at_month`. El sobrecoste —conversión + buffer `meses × activos`— está por
  debajo del 1 % de un sorteo de 500 caminos.
- **De aquí SÍ salen euros, y son `Decimal` del camino EXACTO.** `extra_monthly` y `search_ceiling`
  los construye este módulo (suelo de 100 €, sobrante del mes 1 de `first_month_allocation`,
  doblajes y medias exactas); `freed_saving_monthly` sale de una ejecución DETERMINISTA
  (`run_stopping_at`). El sorteo decide **qué escenario cumple**, nunca **cuánto vale** — por eso la
  regla del crate («ninguna salida se publica como KPI monetario derivado de `f64`») sigue intacta y
  D4 no gana ninguna excepción.

**1 · `minimum_extra_contribution` (M12).** El menor extra mensual `c` **PLANO EN NOMINAL**
(supuesto S2, #139: en este motor los ingresos no se indexan) tal que
`success_at_month(input + c, …, r).meets(umbral)`. Se inyecta en `planning_monthly_cash_adjustment`
—cuya rejilla es **0-based**: índice `i` ⇒ mes `i+1` del bucle—, en los índices `0..=r−2`, o sea los
meses del bucle `1..=r−1`: se aporta mientras se trabaja y se deja de aportar al jubilarse. Va por
«Próximos» y no por `income_regular_monthly` a propósito: subir el ingreso cambiaría también
`ordinary_need` (`gasto − ingreso`) y con ella la puerta de tasa inicial, que es justo el criterio
que se está midiendo.

| fase | qué hace | presupuesto |
|---|---|---|
| 0 | sonda de `c = 0`: si el plan ya cumple, la respuesta es `Some(0)` — una respuesta, no una ausencia | 1 de `search` |
| A | extremo alto desde `max(100 €, sobrante del mes 1)`, **DOBLANDO** | ≤ 1 + 12 |
| B | bisección sobre el importe, «`lo` falla, `hi` cumple, se devuelve `hi`» | ≤ 12 |
| C | redondeo **a decenas hacia arriba** y CONFIRMACIÓN con 2.500; si no cumple, +5 % (con suelo de 10 €) hasta 6 veces | 1 + ≤ 6 |

Se **redondea antes de confirmar**: publicar una cifra distinta de la medida convertiría un solve
verificado en uno decorativo. El techo se **descubre doblando** y no se lee de
`solve.rs::search_ceiling`: aquella cota es el máximo sobrante del horizonte y es correcta para un
solve que pone TECHO a lo que la cascada invierte, pero aquí la incógnita es **dinero nuevo** que la
caja actual no acota — con esa cota se diría «no llegas» a un hogar sin sobrante que solo necesita
encontrar 3.000 €/mes. `underfunded = true` ⟺ ni `search_ceiling` cumple, y entonces
`extra_monthly: None` — **jamás un `Some(0)`**, que diría lo contrario.

**2 · `coast_stop_month` (M10 modo A + corrección C8).** El **PRIMER** `C` (no el último) tal que,
cortando las aportaciones desde ese mes y jubilándose igual en `r`, el plan sigue cumpliendo: la
pregunta es «¿desde cuándo puedo dejar de ahorrar?». Sonda alta `C = r` —el corte es INCLUSIVO
(`k ≥ C` ⇒ techo 0), así que `C = r` es «aportar durante toda la acumulación», el mejor plan de
coast que existe—: si falla, `stop_month: None` + `CoastNotReachable` y **un solo sorteo**. Sonda
baja `C = 1` ⇒ `Some(1)` («puedes dejar de aportar ya»). Si no, bisección ≤ 12 y confirmación
avanzando `C` hacia `r`.

`freed_saving_monthly` es `disposable_cash[C]` de una ejecución determinista con el corte —el mes
`C`, **no** `C+1`, porque el corte es inclusivo y `C` es el primer mes sin aportación—. **Supuesto
S4: el ahorro liberado es caja DISPONIBLE y no se reinvierte**; el pool que llega a la cascada es 0
y el sobrante entero sale del balance. Regresión:
`the_freed_saving_of_coast_is_disposable_and_is_not_reinvested`, que además comprueba la identidad
que lo cierra — los euros liberados son **exactamente** los que le faltan a la cartera frente a la
ejecución sin corte.

**3 · `earliest_partial_start` (M11 modo «en cuanto pueda»).** El menor `S` tal que **la FASE no
falla**, medido con `retirement_trigger = AtMonth(H+1)`: un plan que nunca se jubila del todo, para
que el candidato se juzgue por la fase y no por lo que venga después. Durante `Phase::Partial` el
motor solo puede fallar por **F1** (F3 y F2 se evalúan en el primer mes
jubilado, que no llega — supuesto S1), así que el criterio dice literalmente «la media jornada no se
come la cartera». Sondas `S = 1` y `S = H` (si la alta falla ⇒ `PartialNeverStarts`), bisección
≤ 12, confirmación, y luego **UNA** llamada a `valid_retirement_month` con la fase desde `S*` y
`k_min = S*+1`; si esa fecha vuelve `month: None` ⇒ `PartialNeverFullyRetires`.

**UNA capa anidada, no un producto** — la propiedad que hace viable el solve y que pinea
`the_partial_phase_solve_is_one_nested_layer_not_a_product`: el coste es `bisección_de_S + UNA
fecha`, no `bisección_de_S × fecha` (que serían ~14 × ~39 ≈ 550 sorteos). Los contadores de
`PartialSolve` son **los propios**, sin los de la fecha anidada: un total que esconde qué capa gastó
qué no sirve para presupuestar. Sin `PhasePlan::partial` declarado, `start_month: None` **sin aviso**
y sin sorteos («no hay pregunta que responder», la convención de `solve.rs`) — pero el `McConfig` se
valida igual.

**Coste medido** (release, P9 sin inflación a 840 meses,
`tests/timing_mc.rs::the_three_strategy_solves_cost_what_the_plan_says`; un sorteo ≈ 79–93 ms con
500 caminos y ≈ 380–390 ms con 2.500):

```text
  aportación mínima · R = 480 (ya cumple)  ⇒ 0 €/mes           ·  1×500 + 1×2.500  ⇒ 0,47 s
  aportación mínima · R = 240 (hay que buscar) ⇒ 1.210 €/mes   · 16×500 + 1×2.500  ⇒ 1,86 s   (plan ≤ 3 s)
  coast · R = 480 ⇒ primer C = 343, libera 1.600 €/mes         · 11×500 + 1×2.500  ⇒ 1,23 s   (plan ≤ 2 s)
  jornada reducida ⇒ primer S = 241, sin jubilación total      · 23×500 + 1×2.500  ⇒ 2,42 s   (plan ≤ 5 s)
                     (12+1 propios · 11+0 de la ÚNICA fecha)
```

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

// 5.0.0 E4: `fire_target_at_month_index_with_plan` / `PlanFireTarget` (`target.rs`) LLAMAN a la
// función de abajo para CUALQUIER plan — bit-identidad por construcción, no por revisión. El plan
// dejó de entrar en el número. Ver §El número FIRE clásico.
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
    pub crossing_is_reading_only: bool,        // default false — D17: el cruce NO jubila, solo se anota
    pub contribution_cap_monthly: Option<Decimal>, // default None — techo de lo que la cascada invierte
    pub contributions_stop_month: Option<u32>, // default None — desde ese mes, techo 0 (coast)
    pub income_pause: Option<IncomePause>,     // default None — P8.c
    pub initial_rate: Option<InitialRateGate>, // E1 — default None ⇒ SIN puerta (pins 4.15 intactos)
}
pub enum RetirementTrigger { LiquidCrossing, AtMonth(u32) }
pub enum SpendMode { Ceiling, RuleIsSpend }
pub enum WithdrawalRule { FixedReal, PercentOfBalance { pct }, Hybrid { start_pct, end_pct },
                          Guardrails { pct, band_pct, adjust_pct } }   // las 4 desde WP2 — §Reglas de retirada
pub enum ExpenseBasis { Retirement, Regular }
pub enum Phase { Accumulating, Partial, Retired }
// WP3 llenó el enum. `code()` es el literal PÚBLICO de cada aviso (el que la API publica en
// `warnings[]`): vive en el motor para que no haya un `match` duplicado en `apps/api`.
pub enum EngineWarning { PartialPhaseCapitalShrinking } // E1 retiró RetireAtAgeUnderfunded; E4, CoastNotReachable
impl EngineWarning { pub fn code(self) -> &'static str }
// 5.0.0 E1 — puerta de tasa inicial y veredicto de UN camino (§2.5 de financial-contracts.md).
pub struct InitialRateGate { pub swr_pct: Decimal, pub bridge: Option<BridgeCap> }
pub struct BridgeCap { pub max_pct: Decimal, pub max_years: u32 }
pub enum PathFailure { PortfolioDepleted, InitialRateExceeded, RuleBelowNeed }
impl PathFailure { pub fn code(self) -> &'static str } // portfolio_depleted | initial_rate_exceeded | rule_below_need
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

### El número FIRE clásico (5.0.0 E4)

`crates/engine/src/target.rs` publica **una lectura informativa**, no una decisión: el número que la
literatura FIRE llama «25× tu gasto». La fecha de jubilación la resuelve el umbral de éxito sobre
miles de caminos (`crates/engine-stochastic`), no un cruce contra este número.

```rust
// Objetivo de un PLAN en el índice 0-based `i`. Desde E4 devuelve EXACTAMENTE lo que devuelve
// `fire_target_at_month_index(ft, i)` para CUALQUIER plan, porque llama a la misma función del
// núcleo (`sim_core::fire_target_at_index_g`). El `plan` ya no se lee: se conserva en la firma
// porque la lectura sigue siendo «el objetivo de ESTE plan» para quien la consume.
pub fn fire_target_at_month_index_with_plan(
    ft: Option<&FireTarget>, plan: &PhasePlan, month_index: u32,
) -> Option<Decimal>

// La cara para recorrer una serie entera: construir una vez, consultar `at(i)` mes a mes.
pub struct PlanFireTarget<'a> { /* … */ }
impl PlanFireTarget<'_> {
    pub fn new(ft: Option<&FireTarget>, plan: &PhasePlan) -> Self
    pub fn at(&self, month_index: u32) -> Option<Decimal>
}
```

Con `f(i) = inflation_factor_at_month_index` e `I_persist` el ingreso PLANO que persiste tras
jubilarse (la pensión SIN fecha de 4.15.0, dentro de `FireNeed`):

```text
T(i) = gross_up(12 · max(0, E·f(i) − I_persist)) / (SWR/100) + deuda(i)
```

La rejilla es **0-based** (el bucle evalúa su mes `k` contra el índice `i = k−1`). `None` = no hay
objetivo (sin objetivo declarado, sin SWR positivo o sin necesidad HOY) — **nunca «cero»**.

**Lo que E4 retiró, y por qué** (decisión M4 del modelo v2, owner 2026-09-06):

- **La pensión CON FECHA ya no se resta.** Es un flujo de caja que el bucle cobra mes a mes, no un
  descuento sobre un stock. La base que la restaba desde `P` tenía un **acantilado de
  construcción**: con una pensión que cubriera el gasto entero, `need_net(i) ≤ 0` dejaba
  `T(i) = deuda(i)` —0 € sin deuda— y el objetivo se desplomaba de 600.000 € a 0 € entre dos meses
  consecutivos. Medido en la batería: `P19_pension_perpetuity_covering` cruzaba en el mes **121**
  (el siguiente al de la pensión) y hoy cruza en el **306**, cuando la acumulación llega de verdad.
- **`TargetBasis`, la base `BridgeToPension`, `bridge_discount_annual_pct` y su tabla sufijo `O(P)`,
  `MAX_BRIDGE_MONTHS` y `EngineError::BridgeDiscountOverflow`.** El puente sobrevive, pero como lo
  que la corrección C2 dice que es: un **tope de tasa inicial con fecha límite** (`BridgeCap`,
  evaluado por el bucle en el mes `R`), no una forma de dimensionar un objetivo. Con la tabla se fue
  también su violación de contrato LATENTE (más allá de `MAX_BRIDGE_MONTHS` la degradación podía
  bajar el objetivo un 77 %) y su coste medido de ~10 µs por mes de puente.
- **Las tres lecturas derivadas** `bridge_effective_withdrawal_pct`, `pension_coverage_ratio` y
  `partial_gap_target`, más los helpers que las alimentaban (`need_full_annual_at`,
  `expense_monthly_at`, `pension_monthly_at`): las tres capitalizaban una necesidad al SWR para
  decir algo sobre una fase, y esa pregunta la contesta hoy el umbral de éxito.

**El cruce sigue vivo como LECTURA**: `líquido(k−1) ≥ T(k−1)` se anota en
`liquid_crossing_month_index` y `RetirementTrigger::LiquidCrossing` sigue siendo el default de
`PhasePlan::classic` — P1–P13 de `pins-4.15.json` lo hashean.

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
- El **target FIRE se reevalúa cada mes SOBRE LA NECESIDAD** (4.10.0, #170): `target(i) = gross_up(need(i)·12, tramos, g)/(swr/100) + debt_term(i)`, con la necesidad indexada según su estructura (`FireNeed`) — **no una base pre-calculada que se infla entera**; `base_amount` se retiró en esa ola. `debt_term(i)` es el término finito de deuda de #142 (ver `FireTarget.debt_payments_remaining`; 0 sin pasivos) y **no se infla** (las cuotas son nominales por contrato). El gross-up de la necesidad inflada NO es el gross-up inflado: la escala es afín y los tramos son NOMINALES (fiscal drag). Desde E4 de 5.0.0 hay **una sola fuente**, `fire_target_at_month_index`, y el evaluador del plan la llama tal cual (§El número FIRE clásico): la pensión CON FECHA ya no entra en el objetivo.
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
     En el mes efectivo `R` (el primero jubilado) se evalúa la **puerta de tasa inicial** (5.0.0 E1, C1): con `phase_plan.initial_rate = Some(gate)`, si `12·ordinaria(R) > tope/100 · liquid_prev` el camino FALLA en `R` con `PathFailure::InitialRateExceeded`. El tope es `gate.bridge.max_pct` cuando hay puente Y pensión con fecha dentro de la ventana (`P − R ≤ 12·max_years`, `P = start_index + 1` en meses del bucle) y `gate.swr_pct` si no; se evalúa con `withdrawal::monthly_allowance` (la MISMA función que topa las reglas por saldo) y se compara con `MoneyOps::strictly_below`. `initial_rate: None` —el default de los dos constructores— **no ejecuta ninguna comparación**, y por eso `pins-4.15.json` no se mueve. Aquí vivía `EngineWarning::RetireAtAgeUnderfunded`, **retirado en E1**: «me jubilo por edad y no llego» dejó de ser un booleano contra un objetivo descontado y pasó a ser `1 − éxito(R)` en el crate estocástico.
   - **Media jornada** (P7/D10) si el latch no cerró y `k ≥ partial.start_month`: el ingreso es `partial.income_monthly` (PLANO) y el gasto el que diga `expense_basis` (jubilación por defecto, regular si el perfil lo dice). `partial_retirement_month_index` solo se publica si la fase se pisó de verdad — una media jornada declarada DESPUÉS de la jubilación no ocurre y no se pinta.
   El ingreso y el gasto salen de la fase; **el gasto elegido se multiplica por `f(k−1) = inflation_factor_at_month_index(input.annual_inflation_percent, k−1)`** (4.9.0, #139) — el ingreso no.
3. **Pensión con fecha** (P2/D3/D8, WP3): desde `k−1 ≥ pension.start_index` se SUMA al ingreso, **en cualquier fase**, por `monthly_today·f(k−1)` si `indexed` y plana si no; durante `Partial`, × `fraction_while_partial`. Sin pensión con fecha aquí no se ejecuta ni una suma (bit-identidad). `pension_start_month_index = start_index + 1` (rejilla 0-based del target → mes 1-based del bucle), `None` si cae fuera del horizonte.
   **Pausa de ingresos** (P8.c): dentro de la ventana semiabierta de `income_pause`, el ingreso GANADO de la fase se multiplica por `income_fraction` — la pensión con fecha, que se suma después, NO se pausa.
4. `retirement_withdrawal` = `phase_plan.extra_monthly_withdrawal` if `in_retirement`, else 0.
5. `net_cash = income - expense - debt_service + planning_adj[k] - retirement_withdrawal`.
   **Y, al lado, la necesidad ORDINARIA** (5.0.0 E1): `ordinary_need = max(0, expense + retirement_withdrawal − income)` — el gasto de vivir, con la pensión con fecha y las rentas persistentes ya restadas, **sin servicio de deuda y sin `planning_adj`**. Es la magnitud que juzgan la puerta de tasa inicial (F2) y la regla por saldo (F3); `net_cash` sigue siendo lo que decide cuánto se VENDE. Se publica en `SimOutput::ordinary_need` (serie, `len = horizon+1`, `[0] = 0`) y **no** en `ProjectionOutput`: la consume el crate estocástico, no la API.
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
    pub assets_depleted_month_index: Option<u32>, // 4.6.0 (#119) + 5.0.0: primer mes que dejó lo vendible a CERO **y** con alguna venta sin fundar desde él
    pub uncovered_deficit_total: Decimal,         // 4.6.0 (#119): undrained_cumulative final — operando LITERAL de 4.15.0, puede traer cola de ±1e-24; quien publica clampa
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
    pub unmet_need: Vec<Decimal>,                   // 5.0.0 (revisión D20): necesidad que la CARTERA no fundó — incremento mensual del descubierto, clampado a 0
    pub pension_start_month_index: Option<u32>,     // WP3: `pension.start_index + 1` (1-based), None si cae fuera del horizonte
    pub partial_retirement_month_index: Option<u32>,// WP3: primer mes de media jornada — None si la fase no se pisó
    pub warnings: Vec<EngineWarning>,               // desde E4 solo PartialPhaseCapitalShrinking (E1 retiró RetireAtAgeUnderfunded; E4, CoastNotReachable)
    // --- 5.0.0 WP3 (§B.3, §B.7): lecturas de fase y margen. E4 retiró las TRES del objetivo
    //     (bridge_effective_withdrawal_pct, pension_coverage_ratio, partial_gap_target) ---
    pub partial_phase_capital_growing: bool,        // true ⟺ HUBO fase parcial Y el líquido no bajó ni un mes en ella
    pub disposable_cash: Vec<Decimal>,              // caja que el techo de aportación dejó fuera de la cascada (len horizon+1, [0] = 0)
    pub disposable_cash_total: Decimal,             // Σ de la serie. "0" son cero euros, no «no aplica»
    // --- 5.0.0 E1: EL VEREDICTO DE ESTE CAMINO. Latches monótonos, fijados a la vez ---
    pub failure_month_index: Option<u32>,          // primer mes (1-based) en que el camino falló; None = aguanta
    pub failure_kind: Option<PathFailure>,         // por qué. Prioridad F1 > F2 > F3 dentro del mes;
                                                   // F2 y F3 SOLO pueden firmar en R (C10), después solo F1
}
```

**`failure_*` es el veredicto de UN camino, no el del PLAN.** El plan se juzga con la proporción de
caminos sin fallo contra el umbral del perfil (`crates/engine-stochastic`); sobre la línea
determinista estos dos campos describen un escenario. Y **no son `assets_depleted_month_index`**:
aquel marca el mes en que la cartera se vació (con confirmación posterior) en CUALQUIER fase; estos
solo miran meses `Retired`/`Partial` (durante `Partial`, solo F1) y marcan el primer mes en que la
necesidad se quedó sin fundar, la tasa inicial se pasó del tope o la regla se quedó por debajo del
gasto ordinario. En la batería: `P1` agota en el mes 20 y **no** falla (nunca se jubila); `P23`
agota en el 16 y falla en el 17. La semántica completa —los tres predicados, la prioridad, el
supuesto S1 y por qué F1 lleva `unfunded_sale` además de `unmet_need > 0`— vive en
[`financial-contracts.md`](financial-contracts.md) §2.5.

`SimOutput` publica además `ordinary_need: Vec<M>`, que `ProjectionOutput` **no** refleja (la
consume el crate estocástico; la API no la publica).

**Las lecturas de fase son `Option` por disciplina, no por comodidad** (norma de la casa: `null`
nunca es cero): `pension_start_month_index` y `partial_retirement_month_index` son `None` cuando la
fase no existe o cae fuera del horizonte. La única EXCEPCIÓN declarada es
`partial_phase_capital_growing`, que es un `bool`: sin fase parcial vale `false` («no hay fase que
crezca»), y quien necesite distinguir «no hubo» de «hubo y menguó» mira
`partial_retirement_month_index` o el aviso `PartialPhaseCapitalShrinking`.

**Identidad del mes con techo de aportación** (`sobrante > 0`):
`sobrante = Σ aportado + no_asignado + disposable_cash(k)`. La misma se refleja en
`FirstMonthAllocation`, que ganó un campo `disposable` para no romperla en el camino de lectura.

## Solves — las inversas del motor (5.0.0 WP3, podadas en E4 — `solve.rs`)

«¿Qué valor de X hace que la simulación cumpla Y?», **biseccionando sobre el motor entero**, nunca
sobre una fórmula cerrada que lo aproxime (hallazgo M8 de la revisión adversarial: un capital
necesario descontado a una tasa escalar es un número plausible que ninguna simulación produce).

**E4 se llevó de aquí los dos solves que preguntaban «¿llego a `T(R−1)`?»** —
`required_contribution_monthly` y `coast_fire_month_index`, con `SolveResult`, `CoastSolve` y el
aviso `CoastNotReachable`—: su criterio murió con el objetivo como decisión (M4). Las preguntas
siguen vivas y se responden contra el **umbral de éxito** en `crates/engine-stochastic`
(`solve_mc.rs`). Lo que queda son las dos inversas que no miran ningún objetivo, más los dos
motores de escenario, que se hicieron **públicos** para que el crate estocástico los reutilice en
vez de copiarlos.

```rust
pub const MAX_SOLVE_ITERATIONS: u32 = 24;   // el PRESUPUESTO: 24 proyecciones, no un umbral

// Los dos motores de escenario: mutan UN eje del PhasePlan y vuelven a simular. Son la plantilla
// de bisección que reutilizan los solves estocásticos.
pub fn run_with_cap(input: &ProjectionInput, cap: Decimal) -> Result<ProjectionOutput, EngineError>
pub fn run_stopping_at(input: &ProjectionInput, stop: u32) -> Result<ProjectionOutput, EngineError>

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
  real, y cualquier respuesta entre esas dos cifras saldría recortada. `search_ceiling` sigue en
  `solve.rs` (privada) y hoy la usa `max_extra_monthly_expense_keeping_date`. Regresión:
  `the_solve_ceiling_is_the_max_monthly_surplus_not_the_first_months_headroom`.
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
| `unmet_need` / `uncovered_deficit_total` | lo que los ACTIVOS no pudieron vender de la venta intentada (serie mensual / acumulado) | **SÍ**, como siempre (deuda implícita) |
| `withdrawal_excess` | lo que `rule_is_spend` vendió POR ENCIMA de la necesidad y se gastó | — (sale de la cartera vía `withdrawal`) |

**La identidad del mes, exacta y siempre** (la comprueba `tests/fuzz_invariants.rs` sobre 1.500
hogares aleatorios):

```text
withdrawal + withdrawal_shortfall + unmet_need − withdrawal_excess = necesidad_neta
```

El sobrante RESTA porque es gasto discrecional que ya está dentro de `withdrawal`. La serie
`unmet_need` es la que faltaba: sin ella el reparto solo cerraba cuando la venta se fundaba al
completo, y cualquier cociente de cobertura mentía justo en el caso que importa —la cartera
agotada—, porque con `fixed_real` el recorte es cero por construcción.

**Y el techo ata en las DOS vías.** Hasta la segunda revisión adversarial, la vía mixta decidía si
el techo ataba comparando contra `dd.gross_monthly`, que el paseo ya había recortado a la
capacidad: un techo POR ENCIMA de lo vendible se descartaba en silencio y su rechazo se
contabilizaba como descubierto — 916 € de patrimonio en el caso mínimo de la revisión, con la
venta byte a byte idéntica a la de la vía escalar. Ahora se decide contra lo que la NECESIDAD pide,
y el neto de un techo que la cartera no puede fundar se tasa con la `g` **marginal** (la del último
tramo con material, extendido): es la generalización exacta de la vía escalar — con todas las `g`
iguales devuelve `after_tax_monthly(techo, g)` dígito a dígito.

El handler no publica todavía todas estas lecturas:
`jubilacion_month_index` sigue derivándose en `handlers/projection.rs` (R8 es WP5).

Sobre los dos campos de 4.6.0 (#119): la definición del mes de agotamiento vive en el bucle. Desde
el pase de correcciones de la revisión D20 son **dos condiciones**: (1) primer mes cuya venta dejó
lo vendible a cero —medido DESPUÉS de vender, sobre los saldos, no comparando la venta con la
capacidad antes— y (2) alguna venta sin fundar en ese mes o después. Sin la segunda, un puente que
se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto posterior se publicaba
como «cartera agotada» con `uncovered_deficit_total = 0`. El texto histórico decía que el caso
exacto usa `>=` («la cartera se vacía este mes»), no «primer mes con descubierto», que daría
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
  - **la regla de retirada, la pensión con fecha y la fase parcial**, todas del perfil. (La «base
    del objetivo» era el cuarto eje hasta E4, que la retiró: hay una sola base y es una lectura.)
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
