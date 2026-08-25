//! Monthly projection: regular budget (no derived liability rows) + active debt service +
//! asset contributions / drain / compound growth. Ajustes opcionales por mes desde «Próximos»
//! (`planning_monthly_cash_adjustment`) suman al flujo de caja recurrente del mes (ingreso (+)
//! / gasto (−)) antes del reparto a activos o el drenaje.
//!
//! El reparto del sobrante mensual se hace mediante una **cascada de reglas**
//! ([`AllocationRule`]) ejecutadas en orden ascendente. Cada regla consume parte del sobrante
//! para un activo destino hasta su tope opcional; lo que queda pasa a la siguiente regla.

use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("horizon_months must be >= 1")]
    InvalidHorizon,
    #[error("planning_monthly_cash_adjustment must have length horizon_months")]
    InvalidPlanningAdjustments,
    #[error("allocation_rules contains an out-of-bounds target_index")]
    InvalidAllocationRuleTarget,
    #[error("history timeline dates must be strictly ascending")]
    InvalidHistoryTimeline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimAsset {
    pub id: Uuid,
    pub value: Decimal,
    pub purchase_price: Option<Decimal>,
    pub is_liquid: bool,
    /// Expected annual return % (e.g. 7 for 7%). None → no compound growth (factor 1).
    pub expected_annual_return_percent: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AllocationKind {
    /// Fixed € amount per month (capped at remaining surplus).
    Fixed,
    /// Percentage (0..=100) of the surplus *remaining at this step* of the cascade.
    Percent,
    /// Whatever surplus remains. Exactly one such rule is enforced by the handler.
    Remainder,
}

/// Optional ceiling on the destination asset. When the asset's current value reaches the
/// resolved ceiling, the rule contributes 0 and the cascade continues with the next rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AllocationCap {
    /// Absolute € target value for the destination asset.
    Amount(Decimal),
    /// Cap = N × (monthly expense + debt service). N is stored here.
    MonthsExpense(Decimal),
    /// Cap = N × monthly income. N is stored here.
    IncomeMultiple(Decimal),
}

#[derive(Debug, Clone)]
pub struct AllocationRule {
    /// Index into [`ProjectionInput::assets`]. The handler resolves UUIDs into indices.
    pub target_index: usize,
    pub kind: AllocationKind,
    /// `Fixed` → €/mes. `Percent` → 0..=100. `Remainder` → ignored (use `None`).
    pub amount: Option<Decimal>,
    pub cap: Option<AllocationCap>,
}

#[derive(Debug, Clone)]
pub struct ProjectionLiabilityInput {
    pub principal: Decimal,
    pub monthly_payment: Decimal,
    pub payment_end: Option<NaiveDate>,
}

/// Target FIRE evaluado mes a mes. `base_amount` es el patrimonio necesario en euros de hoy
/// (gross-up de impuestos ya aplicado); el target del mes `k` es
/// `base_amount × (1 + annual_inflation_percent/100)^((k-1)/12)`, lo que preserva el poder
/// adquisitivo del usuario en el momento de la jubilación. `annual_inflation_percent = 0`
/// degenera a un target plano (mismo valor en todos los meses).
#[derive(Debug, Clone)]
pub struct FireTarget {
    pub base_amount: Decimal,
    pub annual_inflation_percent: Decimal,
}

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    /// Civil "today" de la instalación (inicio del mes simulado para el índice 1).
    pub ref_date: NaiveDate,
    pub horizon_months: u32,
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    /// Cascada de reglas, en orden ascendente de prioridad (índice 0 = primera).
    pub allocation_rules: Vec<AllocationRule>,
    pub liabilities: Vec<ProjectionLiabilityInput>,
    /// Signed cash from planning flows per simulated month (`len == horizon_months`): index `i`
    /// pairs with month `i+1` (calendar month `add_months(month_first_calendar(ref_date), i)`).
    pub planning_monthly_cash_adjustment: Vec<Decimal>,
    /// Month index (1-based, same indexing as the simulation loop) at which the retirement
    /// drawdown phase begins. `None` = no drawdown modelled in this projection.
    pub retirement_start_month: Option<u32>,
    /// Income from sources that persist after retirement (e.g. rental, pension).
    /// Replaces `income_regular_monthly` from `retirement_start_month` onward.
    /// Typically `0` when all income stops at retirement (the default).
    pub income_retirement_monthly: Decimal,
    /// Expenses from sources that continue after retirement (i.e. entries where
    /// `ends_at_retirement = false`). Replaces `expense_regular_monthly` from
    /// `retirement_start_month` onward. Equal to `expense_regular_monthly` when
    /// no expense ends at retirement.
    pub expense_retirement_monthly: Decimal,
    /// Optional additional monthly draw from assets on top of the income/expense budget.
    /// The handler typically passes `0` — income reduction via `income_retirement_monthly`
    /// is the primary drain mechanism in the new model.
    pub retirement_monthly_withdrawal: Decimal,
    /// Target FIRE móvil. Cuando está presente, las aportaciones se detienen en el primer mes
    /// donde el patrimonio cruza el target inflado correspondiente al mes en curso.
    pub fire_target: Option<FireTarget>,
}

#[derive(Debug, Clone)]
pub struct ProjectionOutput {
    /// Month index 0..=horizon_months inclusive. Serie nominal: el patrimonio se expresa en euros
    /// del momento. La comparación con la inflación se hace contra el target FIRE móvil
    /// (`FireTarget`), no deflactando esta serie.
    pub net_worth: Vec<Decimal>,
    /// Cumulative contributed basis (nominal).
    pub contributed_capital: Vec<Decimal>,
    /// Per-asset value series: `per_asset_series[asset_index][month_index]`.
    /// Length == `assets.len()`; inner length == `horizon_months + 1` (months 0..=horizon).
    /// Nominal, igual que `net_worth`.
    pub per_asset_series: Vec<Vec<Decimal>>,
}

/// Primero-de-mes de una fecha (día 1 del mismo mes). Compartido con `history.rs`.
pub(crate) fn month_first_calendar(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

fn add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(Months::new(n))
        .unwrap_or(d)
}

fn month_window(month_first: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = month_first;
    let next_first = add_months(month_first, 1);
    let end = next_first.pred_opt().unwrap_or(start);
    (start, end)
}

/// Factor de crecimiento **mensual** equivalente a una tasa anual nominal (raíz 12ª del factor
/// anual). Tasas ausentes o exactamente 0 se tratan como crecimiento 0 (factor 1). Las tasas
/// **negativas componen de verdad** (−50 % anual ⇒ factor mensual ≈ 0,9439, ×0,5 a los 12 meses);
/// una tasa ≤ −100 % se clampa a factor 0 (pérdida total: el factor anual 1 + p/100 sería ≤ 0 y
/// no tiene raíz 12ª real). La capa API rechaza inputs ≤ −100 con error tipado; el clamp protege
/// frente a valores absurdos ya persistidos.
///
/// `pub(crate)` porque `runway.rs` lo comparte: el runway debe usar EXACTAMENTE la misma
/// conversión anual→mensual que la simulación, o divergiría del chart de proyección. Nota: para
/// la inflación del gasto del runway el argumento nunca es negativo (la instalación valida
/// 0..50), así que este cambio solo afecta al retorno esperado de los activos.
pub(crate) fn monthly_multiplier(annual_percent: Option<Decimal>) -> Decimal {
    let Some(p) = annual_percent else {
        return Decimal::ONE;
    };
    if p.is_zero() {
        return Decimal::ONE;
    }
    let annual_factor = Decimal::ONE + p / Decimal::from(100);
    if annual_factor <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    annual_factor.powd(Decimal::ONE / Decimal::from(12))
}

/// Target FIRE en el `month_index` indicado (0 = punto de partida, 12 = un año después, etc.),
/// con inflación anual compuesta capitalizada en pasos de 1/12 de año. `month_index = 0` devuelve
/// el `base_amount`. Devuelve `None` cuando no hay target o su base es ≤ 0.
///
/// Es la **única fuente de verdad**: tanto el motor (para decidir `fire_reached`) como el
/// handler de la API (para construir `fire_target_series`) la consumen, evitando off-by-one
/// entre la serie y el cruce.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal> {
    let ft = ft?;
    if ft.base_amount <= Decimal::ZERO {
        return None;
    }
    if ft.annual_inflation_percent <= Decimal::ZERO || month_index == 0 {
        return Some(ft.base_amount);
    }
    let years = Decimal::from(month_index) / Decimal::from(12u32);
    let factor = (Decimal::ONE + ft.annual_inflation_percent / Decimal::from(100u32)).powd(years);
    Some(ft.base_amount * factor)
}

fn drain_from_assets(
    values: &mut [Decimal],
    liquid: &[bool],
    rates: &[Option<Decimal>],
    mut need: Decimal,
) -> Decimal {
    if need <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let n = values.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| {
        let li = liquid[i];
        let lj = liquid[j];
        match (li, lj) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => rates[i]
                .unwrap_or(Decimal::ZERO)
                .cmp(&rates[j].unwrap_or(Decimal::ZERO))
                .then_with(|| i.cmp(&j)),
        }
    });
    for idx in order {
        if need <= Decimal::ZERO {
            break;
        }
        let take = values[idx].min(need);
        values[idx] -= take;
        need -= take;
    }
    need
}

/// Resolve a rule's cap into an absolute € ceiling for the destination asset.
/// Returns `None` for an uncapped rule.
fn resolve_cap_ceiling(
    cap: Option<AllocationCap>,
    monthly_expense_with_debt: Decimal,
    monthly_income: Decimal,
) -> Option<Decimal> {
    match cap {
        None => None,
        Some(AllocationCap::Amount(v)) => Some(v.max(Decimal::ZERO)),
        Some(AllocationCap::MonthsExpense(n)) => {
            Some((n.max(Decimal::ZERO) * monthly_expense_with_debt).max(Decimal::ZERO))
        }
        Some(AllocationCap::IncomeMultiple(n)) => {
            Some((n.max(Decimal::ZERO) * monthly_income).max(Decimal::ZERO))
        }
    }
}

/// Por qué una regla de la cascada no recibió (o recibió menos de lo que pedía).
///
/// Las cuatro razones tienen **remedios distintos**, y por eso no se colapsan: `NoCash` es «no te
/// sobra dinero» (toca ingresos o gastos), `NotReached` es «las reglas de arriba se lo comieron»
/// (tocan prioridades o topes), `CapFull` es «el activo destino ya está en su techo» y `ZeroAmount`
/// es una regla configurada a cero. Un `skipped_reason` ausente con importe cero invitaría a
/// inventarse la causa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationSkipReason {
    /// La caja del mes era ≤ 0: ninguna regla llegó a evaluarse.
    NoCash,
    /// La cascada agotó la caja antes de llegar a esta regla.
    NotReached,
    /// El activo destino ya alcanzó el techo del cap.
    CapFull,
    /// La regla resolvió un importe de 0 (fijo a 0, o porcentaje 0).
    ZeroAmount,
    /// `target_index` fuera de rango. Inalcanzable desde el handler, que valida antes; el bucle
    /// principal lo tolera en silencio y aquí se hace explícito.
    InvalidTarget,
}

/// Traza de UNA regla en la cascada de un mes. `amount_intent` es lo que la regla pidió y
/// `amount_resolved` lo que se llevó tras aplicar cap y caja disponible: cuando difieren sin haber
/// `skipped_reason`, la regla fue **recortada**, que no es un salto pero es justo lo que se quiere
/// ver al preguntar «¿por qué mi cartera recibe menos de lo que puse?».
#[derive(Debug, Clone)]
pub struct RuleOutcome {
    /// Posición de la regla en `ProjectionInput::allocation_rules`. El engine no conoce UUIDs: el
    /// handler mapea el índice a su identidad.
    pub rule_index: usize,
    pub target_index: usize,
    pub amount_intent: Decimal,
    pub amount_resolved: Decimal,
    /// Techo absoluto resuelto del cap (`None` = sin cap).
    pub cap_ceiling: Option<Decimal>,
    /// Espacio que quedaba bajo el techo al evaluar la regla (`None` = sin cap).
    pub cap_room: Option<Decimal>,
    pub skipped_reason: Option<AllocationSkipReason>,
}

/// Cascade-distribute a positive `pool` across assets following the ordered `rules`.
///
/// For each rule (in order):
/// - resolve the destination asset's cap_room (`ceiling − current_value`); rule is skipped if 0.
/// - compute the rule's intent:
///   - `Fixed`     → `min(amount, remaining)`
///   - `Percent`   → `remaining × amount / 100` (over what's left at this step)
///   - `Remainder` → `remaining`
/// - take `min(intent, cap_room?, remaining)`, add to `alloc[target]`, subtract from `remaining`.
///
/// Returns `(alloc, leftover)`: `alloc[i]` ≥ 0 added to asset `i`; `leftover` is the pool that
/// no rule absorbed (caller routes it to `surplus_cash`).
///
/// `trace` es un **sumidero opcional**: con `None` no se asigna nada y el coste es idéntico al de
/// antes de existir — importa porque el bucle de proyección llama a esta función hasta 840 veces
/// por request y nadie lee la traza ahí. Con `Some`, se emite un [`RuleOutcome`] por regla,
/// incluidas las que no reciben nada. Una sola implementación de la cascada: dos divergirían en
/// silencio al primer cambio de caps, y una explicación que no coincide con lo que el motor hace es
/// peor que no tener explicación.
fn distribute_contributions(
    pool: Decimal,
    rules: &[AllocationRule],
    values: &[Decimal],
    monthly_expense_with_debt: Decimal,
    monthly_income: Decimal,
    mut trace: Option<&mut Vec<RuleOutcome>>,
) -> (Vec<Decimal>, Decimal) {
    let n = values.len();
    let mut alloc = vec![Decimal::ZERO; n];
    if pool <= Decimal::ZERO || n == 0 {
        if let Some(t) = trace.as_deref_mut() {
            for (rule_index, rule) in rules.iter().enumerate() {
                t.push(RuleOutcome {
                    rule_index,
                    target_index: rule.target_index,
                    amount_intent: Decimal::ZERO,
                    amount_resolved: Decimal::ZERO,
                    cap_ceiling: None,
                    cap_room: None,
                    skipped_reason: Some(AllocationSkipReason::NoCash),
                });
            }
        }
        return (alloc, pool.max(Decimal::ZERO));
    }
    let mut remaining = pool;
    // Live view of asset values for cap calculations as the cascade progresses (so multiple
    // rules into the same asset respect a shared ceiling).
    let mut live_values: Vec<Decimal> = values.to_vec();

    for (rule_index, rule) in rules.iter().enumerate() {
        // Emite la traza de una regla que no llegó a repartir y sigue.
        macro_rules! skip {
            ($reason:expr, $intent:expr, $ceiling:expr, $room:expr) => {{
                if let Some(t) = trace.as_deref_mut() {
                    t.push(RuleOutcome {
                        rule_index,
                        target_index: rule.target_index,
                        amount_intent: $intent,
                        amount_resolved: Decimal::ZERO,
                        cap_ceiling: $ceiling,
                        cap_room: $room,
                        skipped_reason: Some($reason),
                    });
                }
            }};
        }

        if remaining <= Decimal::ZERO {
            // La caja se agotó: esta regla y todas las siguientes quedan sin evaluar. Se emiten
            // igualmente — omitirlas reproduciría el hueco de observabilidad que la traza cierra.
            if let Some(t) = trace.as_deref_mut() {
                for (i, r) in rules.iter().enumerate().skip(rule_index) {
                    t.push(RuleOutcome {
                        rule_index: i,
                        target_index: r.target_index,
                        amount_intent: Decimal::ZERO,
                        amount_resolved: Decimal::ZERO,
                        cap_ceiling: None,
                        cap_room: None,
                        skipped_reason: Some(AllocationSkipReason::NotReached),
                    });
                }
            }
            break;
        }
        let target = rule.target_index;
        if target >= n {
            skip!(
                AllocationSkipReason::InvalidTarget,
                Decimal::ZERO,
                None,
                None
            );
            continue;
        }
        let ceiling = resolve_cap_ceiling(rule.cap, monthly_expense_with_debt, monthly_income);
        let cap_room = ceiling.map(|c| (c - live_values[target]).max(Decimal::ZERO));
        if let Some(room) = cap_room {
            if room <= Decimal::ZERO {
                skip!(AllocationSkipReason::CapFull, Decimal::ZERO, ceiling, cap_room);
                continue;
            }
        }
        let intent = match rule.kind {
            AllocationKind::Fixed => rule.amount.unwrap_or(Decimal::ZERO).max(Decimal::ZERO),
            AllocationKind::Percent => {
                let pct = rule.amount.unwrap_or(Decimal::ZERO).max(Decimal::ZERO);
                (remaining * pct) / Decimal::from(100)
            }
            AllocationKind::Remainder => remaining,
        };
        let mut take = intent.min(remaining);
        if let Some(room) = cap_room {
            take = take.min(room);
        }
        if take <= Decimal::ZERO {
            skip!(AllocationSkipReason::ZeroAmount, intent, ceiling, cap_room);
            continue;
        }
        alloc[target] += take;
        live_values[target] += take;
        remaining -= take;
        if let Some(t) = trace.as_deref_mut() {
            t.push(RuleOutcome {
                rule_index,
                target_index: target,
                amount_intent: intent,
                amount_resolved: take,
                cap_ceiling: ceiling,
                cap_room,
                skipped_reason: None,
            });
        }
    }

    (alloc, remaining.max(Decimal::ZERO))
}

/// Resolución completa de la cascada del **primer mes**: lo que se reparte, de dónde sale y qué
/// queda sin repartir.
///
/// Existe porque `first_month_per_asset_contribution_nominals` devolvía solo `per_asset` y tiraba
/// tanto el `leftover` —que la cascada ya calculaba— como la base. Sin esa base era imposible
/// explicar por qué la aportación del mes 1 no cuadra con el neto recurrente del summary: la
/// diferencia es `planning_component`, el tramo de los planning flows sin fecha que caen en el mes
/// en curso (repartidos a `importe/90` por día natural), y eso hace que el número **cambie cada
/// día**. Ese desajuste se leyó como una sobreasignación de la cascada, que no lo era.
///
/// Identidades garantizadas: `base_cash = recurring_net + planning_component` y
/// `Σ per_asset + leftover = base_cash` cuando `base_cash > 0` (con `base_cash ≤ 0` no se reparte
/// nada y `leftover` es 0).
#[derive(Debug, Clone)]
pub struct FirstMonthAllocation {
    /// Aporte nominal por activo, en el orden de `ProjectionInput::assets`.
    pub per_asset: Vec<Decimal>,
    /// La caja del mes que la cascada reparte de verdad (`net_cash_month` del engine).
    pub base_cash: Decimal,
    /// `income − expense − debt_service`: la parte **estable**, la que una persona quiere decir
    /// cuando dice «mi aportación mensual».
    pub recurring_net: Decimal,
    /// `planning_adjustment[0] − retirement_withdrawal`: la parte **transitoria** del mes en curso.
    pub planning_component: Decimal,
    /// Cuota de los pasivos activos ya descontada de `recurring_net`.
    pub debt_service: Decimal,
    /// Lo que ninguna regla absorbió y acaba en `surplus_cash`.
    pub leftover: Decimal,
    /// Traza regla a regla, en el orden de `ProjectionInput::allocation_rules`.
    pub rules: Vec<RuleOutcome>,
}

/// Igual que [`first_month_allocation`] pero devolviendo solo los aportes por activo. Se mantiene
/// como wrapper por compatibilidad: es lo que consume `GET /v1/assets`.
pub fn first_month_per_asset_contribution_nominals(
    input: &ProjectionInput,
) -> Result<Vec<Decimal>, EngineError> {
    first_month_allocation(input).map(|a| a.per_asset)
}

/// Nominal contributions routed to each asset in the **first simulated month** (calendar month de
/// `ref_date`): cascada de reglas sobre el sobrante del mes, más la base de la que sale y la traza
/// de cada regla. Cero si el superávit es ≤ 0.
pub fn first_month_allocation(
    input: &ProjectionInput,
) -> Result<FirstMonthAllocation, EngineError> {
    if input.horizon_months < 1 {
        return Err(EngineError::InvalidHorizon);
    }
    if input.planning_monthly_cash_adjustment.len() != input.horizon_months as usize {
        return Err(EngineError::InvalidPlanningAdjustments);
    }
    let n = input.assets.len();
    let mut out = vec![Decimal::ZERO; n];
    for r in &input.allocation_rules {
        if r.target_index >= n {
            return Err(EngineError::InvalidAllocationRuleTarget);
        }
    }
    if n == 0 {
        return Ok(FirstMonthAllocation {
            per_asset: out,
            base_cash: Decimal::ZERO,
            recurring_net: Decimal::ZERO,
            planning_component: Decimal::ZERO,
            debt_service: Decimal::ZERO,
            leftover: Decimal::ZERO,
            rules: Vec::new(),
        });
    }

    let values: Vec<Decimal> = input.assets.iter().map(|a| a.value).collect();
    let principals: Vec<Decimal> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(Decimal::ZERO))
        .collect();

    let start_month_first = month_first_calendar(input.ref_date);
    let month_first = add_months(start_month_first, 0);
    let (m_start, _m_end) = month_window(month_first);

    let mut debt_service = Decimal::ZERO;
    for (i, liab) in input.liabilities.iter().enumerate() {
        let active = match liab.payment_end {
            None => liab.monthly_payment > Decimal::ZERO,
            Some(end) => end >= m_start && liab.monthly_payment > Decimal::ZERO,
        };
        if !active {
            continue;
        }
        let pay = liab
            .monthly_payment
            .min(principals.get(i).copied().unwrap_or(Decimal::ZERO));
        debt_service += pay;
    }

    let planning_adj = input.planning_monthly_cash_adjustment[0];

    // Estado del mes 1, resuelto EXACTAMENTE como lo hace el bucle de simulación.
    //
    // Antes esta función solo miraba `retirement_start_month` y siempre usaba el ingreso y el
    // gasto regulares, ignorando `fire_target`. En un hogar que ya está por encima de su número
    // FIRE eso publicaba una aportación **con el signo contrario a la realidad**: el bucle
    // detecta el cruce en el mes 1 (`nw_prev ≥ target(0)`), conmuta a ingreso de jubilación y
    // drena de los activos, mientras `/v1/assets` y `/v1/allocation-rules/resolution` seguían
    // diciendo «aportas 2.000 €/mes» y explicando regla a regla una cascada que no se ejecuta
    // jamás. No es un caso patológico: es el estado final del público al que sirve la app.
    //
    // El mes 0 no tiene sobrante acumulado ni caja pendiente, así que el patrimonio de partida
    // es Σ activos − Σ principales, igual que el primer punto de `net_worth`.
    let nw_month_zero: Decimal = values.iter().copied().sum::<Decimal>()
        - principals.iter().copied().sum::<Decimal>();
    let fire_reached = fire_target_at_month_index(input.fire_target.as_ref(), 0)
        .is_some_and(|t| nw_month_zero >= t);
    let in_retirement = fire_reached || input.retirement_start_month.is_some_and(|s| 1 >= s);
    let income = if in_retirement {
        input.income_retirement_monthly
    } else {
        input.income_regular_monthly
    };
    let expense = if in_retirement {
        input.expense_retirement_monthly
    } else {
        input.expense_regular_monthly
    };
    let retirement_withdrawal = if in_retirement {
        input.retirement_monthly_withdrawal
    } else {
        Decimal::ZERO
    };

    let recurring_net = income - expense - debt_service;
    let planning_component = planning_adj - retirement_withdrawal;
    let net_cash_month = recurring_net + planning_component;

    let mut rules_trace: Vec<RuleOutcome> = Vec::new();
    let (alloc, leftover) = distribute_contributions(
        net_cash_month,
        &input.allocation_rules,
        &values,
        input.expense_regular_monthly + debt_service,
        input.income_regular_monthly,
        Some(&mut rules_trace),
    );
    // `distribute_contributions` ya devuelve ceros y `leftover = max(pool, 0)` con caja ≤ 0, así
    // que no hace falta el corte temprano de antes: el resultado es idéntico.
    for i in 0..n {
        out[i] = alloc[i];
    }
    Ok(FirstMonthAllocation {
        per_asset: out,
        base_cash: net_cash_month,
        recurring_net,
        planning_component,
        debt_service,
        leftover: if net_cash_month > Decimal::ZERO {
            leftover
        } else {
            Decimal::ZERO
        },
        rules: rules_trace,
    })
}

pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError> {
    if input.horizon_months < 1 {
        return Err(EngineError::InvalidHorizon);
    }
    if input.planning_monthly_cash_adjustment.len() != input.horizon_months as usize {
        return Err(EngineError::InvalidPlanningAdjustments);
    }
    let n = input.assets.len();
    for r in &input.allocation_rules {
        if r.target_index >= n {
            return Err(EngineError::InvalidAllocationRuleTarget);
        }
    }

    let mut values: Vec<Decimal> = input.assets.iter().map(|a| a.value).collect();
    let liquid: Vec<bool> = input.assets.iter().map(|a| a.is_liquid).collect();
    let rates: Vec<Option<Decimal>> = input
        .assets
        .iter()
        .map(|a| a.expected_annual_return_percent)
        .collect();

    let mut principals: Vec<Decimal> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(Decimal::ZERO))
        .collect();

    let start_month_first = month_first_calendar(input.ref_date);

    let mut net_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut contrib_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut per_asset_series: Vec<Vec<Decimal>> = input
        .assets
        .iter()
        .map(|_| Vec::with_capacity(input.horizon_months as usize + 1))
        .collect();

    // Coste histórico ya invertido (precio de compra) antes del primer mes simulado.
    let initial_contributed_basis: Decimal = input
        .assets
        .iter()
        .filter_map(|a| a.purchase_price)
        .filter(|p| *p > Decimal::ZERO)
        .sum();

    let mut contributed_cumulative = initial_contributed_basis;
    let mut undrained_cumulative = Decimal::ZERO;
    // Monthly savings not routed when remainder weights sum to zero.
    let mut surplus_cash = Decimal::ZERO;

    let nw_fn = |vals: &[Decimal], pr: &[Decimal], und: Decimal, surplus: Decimal| -> Decimal {
        let ta: Decimal = vals.iter().copied().sum();
        let tl: Decimal = pr.iter().copied().sum();
        ta + surplus - tl - und
    };

    net_series.push(nw_fn(
        &values,
        &principals,
        undrained_cumulative,
        surplus_cash,
    ));
    contrib_series.push(contributed_cumulative);
    for (i, s) in per_asset_series.iter_mut().enumerate() {
        s.push(values[i]);
    }

    for k in 1..=input.horizon_months {
        let month_first = add_months(start_month_first, k - 1);
        let (m_start, _m_end) = month_window(month_first);

        let mut debt_service = Decimal::ZERO;
        for (i, liab) in input.liabilities.iter().enumerate() {
            let active = match liab.payment_end {
                None => liab.monthly_payment > Decimal::ZERO,
                Some(end) => end >= m_start && liab.monthly_payment > Decimal::ZERO,
            };
            if !active {
                continue;
            }
            let pay = liab
                .monthly_payment
                .min(principals.get(i).copied().unwrap_or(Decimal::ZERO));
            debt_service += pay;
        }

        let planning_adj = input.planning_monthly_cash_adjustment[(k - 1) as usize];

        let nw_prev = nw_fn(&values, &principals, undrained_cumulative, surplus_cash);
        // `nw_prev` es el patrimonio al cierre del mes k-1; lo comparamos contra el target
        // correspondiente a ese mismo punto del eje temporal.
        let fire_reached = fire_target_at_month_index(input.fire_target.as_ref(), k - 1)
            .map_or(false, |t| nw_prev >= t);
        let in_retirement =
            fire_reached || input.retirement_start_month.map_or(false, |s| k >= s);
        let income = if in_retirement {
            input.income_retirement_monthly
        } else {
            input.income_regular_monthly
        };
        let expense = if in_retirement {
            input.expense_retirement_monthly
        } else {
            input.expense_regular_monthly
        };

        let retirement_withdrawal = if in_retirement {
            input.retirement_monthly_withdrawal
        } else {
            Decimal::ZERO
        };

        let net_cash_month = income
            - expense
            - debt_service
            + planning_adj
            - retirement_withdrawal;

        if net_cash_month <= Decimal::ZERO {
            let mut need = -net_cash_month;
            let from_surplus = surplus_cash.min(need);
            surplus_cash -= from_surplus;
            need -= from_surplus;
            if need > Decimal::ZERO {
                let und = drain_from_assets(&mut values, &liquid, &rates, need);
                undrained_cumulative += und;
            }
        } else if in_retirement {
            // In retirement any surplus stays as cash buffer; no new contributions are made.
            surplus_cash += net_cash_month;
        } else {
            // `None`: el bucle corre hasta 840 veces por request y nadie lee la traza aquí.
            let (alloc, leftover) = distribute_contributions(
                net_cash_month,
                &input.allocation_rules,
                &values,
                expense + debt_service,
                income,
                None,
            );
            for i in 0..values.len() {
                values[i] += alloc[i];
                contributed_cumulative += alloc[i];
            }
            if leftover > Decimal::ZERO {
                surplus_cash += leftover;
                contributed_cumulative += leftover;
            }
        }

        for i in 0..values.len() {
            let m = monthly_multiplier(rates[i]);
            values[i] *= m;
        }

        for (i, liab) in input.liabilities.iter().enumerate() {
            if i >= principals.len() {
                break;
            }
            let active = match liab.payment_end {
                None => liab.monthly_payment > Decimal::ZERO,
                Some(end) => end >= m_start && liab.monthly_payment > Decimal::ZERO,
            };
            if active {
                let pay = liab
                    .monthly_payment
                    .min(principals[i])
                    .max(Decimal::ZERO);
                principals[i] -= pay;
            }
        }

        let nw = nw_fn(
            &values,
            &principals,
            undrained_cumulative,
            surplus_cash,
        );
        net_series.push(nw);
        contrib_series.push(contributed_cumulative);
        for (i, s) in per_asset_series.iter_mut().enumerate() {
            s.push(values[i]);
        }
    }

    Ok(ProjectionOutput {
        net_worth: net_series,
        contributed_capital: contrib_series,
        per_asset_series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn mk_asset(id: u128, value: Decimal, liquid: bool, rate: Option<Decimal>) -> SimAsset {
        SimAsset {
            id: Uuid::from_u128(id),
            value,
            purchase_price: None,
            is_liquid: liquid,
            expected_annual_return_percent: rate,
        }
    }

    fn rule_remainder(target: usize) -> AllocationRule {
        AllocationRule {
            target_index: target,
            kind: AllocationKind::Remainder,
            amount: None,
            cap: None,
        }
    }

    fn rule_fixed(target: usize, amount: Decimal, cap: Option<AllocationCap>) -> AllocationRule {
        AllocationRule {
            target_index: target,
            kind: AllocationKind::Fixed,
            amount: Some(amount),
            cap,
        }
    }

    fn rule_percent(target: usize, pct: Decimal, cap: Option<AllocationCap>) -> AllocationRule {
        AllocationRule {
            target_index: target,
            kind: AllocationKind::Percent,
            amount: Some(pct),
            cap,
        }
    }

    fn base_input(
        horizon: u32,
        income: Decimal,
        expense: Decimal,
        assets: Vec<SimAsset>,
        rules: Vec<AllocationRule>,
    ) -> ProjectionInput {
        ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: horizon,
            income_regular_monthly: income,
            expense_regular_monthly: expense,
            assets,
            allocation_rules: rules,
            liabilities: vec![],
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            expense_retirement_monthly: expense,
            retirement_monthly_withdrawal: Decimal::ZERO,
            fire_target: None,
        }
    }

    #[test]
    fn monthly_multiplier_none_and_zero_are_flat() {
        assert_eq!(monthly_multiplier(None), Decimal::ONE);
        assert_eq!(monthly_multiplier(Some(Decimal::ZERO)), Decimal::ONE);
    }

    /// −50 % anual ⇒ factor anual 0,5 ⇒ factor mensual 0,5^(1/12) ≈ 0,94387. El test fija la
    /// propiedad definitoria: componer el factor 12 veces reconstruye 0,5 (tolerancia 1e−9 por
    /// el powd de Decimal).
    #[test]
    fn negative_return_composes_downward() {
        let m = monthly_multiplier(Some(Decimal::from(-50)));
        assert!(m < Decimal::ONE && m > Decimal::ZERO);
        let annual = m.powd(Decimal::from(12));
        let expected = Decimal::new(5, 1); // 0.5
        assert!(
            (annual - expected).abs() < Decimal::new(1, 9),
            "0.94387…^12 debe reconstruir 0.5, obtenido {annual}"
        );
    }

    /// El factor anual 1 + p/100 es ≤ 0 a partir de −100 %: sin raíz 12ª real, se clampa a
    /// pérdida total (factor 0). La capa API rechaza estos valores; el clamp cubre datos ya
    /// persistidos.
    #[test]
    fn minus_100_or_less_clamps_to_zero_factor() {
        assert_eq!(monthly_multiplier(Some(Decimal::from(-100))), Decimal::ZERO);
        assert_eq!(monthly_multiplier(Some(Decimal::from(-150))), Decimal::ZERO);
    }

    /// Las tasas positivas conservan exactamente la fórmula previa al cambio:
    /// 10 % anual ⇒ 1,1^(1/12) = 1,0079741… (valor capturado antes del refactor).
    #[test]
    fn positive_rates_unchanged() {
        let m = monthly_multiplier(Some(Decimal::from(10)));
        assert_eq!(m.round_dp(7), Decimal::new(1_007_974_1, 7));
    }

    /// Nivel simulación: un activo de 10.000 € al −50 % anual, sin flujos, termina el año en
    /// ≈ 5.000 € (antes del fix quedaba plano en 10.000).
    #[test]
    fn negative_asset_return_decays_value_in_simulation() {
        let a = mk_asset(1, Decimal::from(10_000), true, Some(Decimal::from(-50)));
        let inp = base_input(12, Decimal::ZERO, Decimal::ZERO, vec![a], vec![]);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(10_000));
        let final_nw = out.net_worth[12];
        assert!(
            (final_nw - Decimal::from(5_000)).abs() < Decimal::new(1, 2),
            "esperado ≈ 5000 tras 12 meses a −50 % anual, obtenido {final_nw}"
        );
    }

    #[test]
    fn no_rules_routes_surplus_to_cash() {
        // Sin reglas: el sobrante queda como caja (surplus_cash) y entra al NW.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(3, Decimal::from(3000), Decimal::from(1000), vec![a], vec![]);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::ZERO);
        assert_eq!(out.net_worth[1], Decimal::from(2000));
        assert_eq!(out.net_worth[2], Decimal::from(4000));
        assert_eq!(out.net_worth[3], Decimal::from(6000));
    }

    #[test]
    fn single_remainder_rule_routes_full_surplus_to_target() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_remainder(0)],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1000));
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.per_asset_series[0][1], Decimal::from(1000));
    }

    #[test]
    fn fixed_amount_greater_than_surplus_clips_to_remaining() {
        // Regla pide 1500€ pero solo hay 1000€ → recibe 1000€ y resto = 0.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_fixed(0, Decimal::from(1500), None)],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1000));
    }

    #[test]
    fn cascade_fixed_then_remainder_splits_correctly() {
        // 1: fija 200 a A; 2: resto a B. Sobrante 1000 → A=200, B=800.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_fixed(0, Decimal::from(200), None),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(200));
        assert_eq!(nom[1], Decimal::from(800));
    }

    #[test]
    fn percent_rule_applies_to_remaining_at_step() {
        // Sobrante 1000. R1 fija 200 → quedan 800. R2 percent 50% → 400 a B. R3 remainder → 400 a C.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let c = mk_asset(3, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a, b, c],
            vec![
                rule_fixed(0, Decimal::from(200), None),
                rule_percent(1, Decimal::from(50), None),
                rule_remainder(2),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(200));
        assert_eq!(nom[1], Decimal::from(400));
        assert_eq!(nom[2], Decimal::from(400));
    }

    #[test]
    fn cap_amount_skips_rule_when_asset_at_ceiling() {
        // A ya está a 1000 con cap 1000 → su regla aporta 0. Toda la pasta va a B.
        let a = mk_asset(1, Decimal::from(1000), true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_percent(0, Decimal::from(100), Some(AllocationCap::Amount(Decimal::from(1000)))),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::ZERO);
        assert_eq!(nom[1], Decimal::from(1000));
    }

    #[test]
    fn cap_amount_clips_partial_room() {
        // A a 500, cap 1000 → 500 de room. Regla pide todo (remainder) pero clipa a 500. B remainder.
        let a = mk_asset(1, Decimal::from(500), true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(2000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_percent(0, Decimal::from(100), Some(AllocationCap::Amount(Decimal::from(1000)))),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(500));
        assert_eq!(nom[1], Decimal::from(1500));
    }

    #[test]
    fn cap_months_expense_resolves_against_monthly_total() {
        // Gasto 600€/mes. Cap = 2 meses = 1200. A vacío → 1200 de room.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(2000),
            Decimal::from(600),
            vec![a, b],
            vec![
                rule_percent(0, Decimal::from(100), Some(AllocationCap::MonthsExpense(Decimal::from(2)))),
                rule_remainder(1),
            ],
        );
        // Sobrante = 2000 - 600 = 1400. A se llena a 1200 (cap), B recibe 200.
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1200));
        assert_eq!(nom[1], Decimal::from(200));
    }

    #[test]
    fn cap_income_multiple_resolves_against_monthly_income() {
        // Ingreso 2000€/mes. Cap = 0.5× = 1000.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let b = mk_asset(2, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(2000),
            Decimal::ZERO,
            vec![a, b],
            vec![
                rule_percent(
                    0,
                    Decimal::from(100),
                    Some(AllocationCap::IncomeMultiple(Decimal::new(5, 1))), // 0.5
                ),
                rule_remainder(1),
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1000));
        assert_eq!(nom[1], Decimal::from(1000));
    }

    #[test]
    fn multi_rules_same_asset_share_ceiling() {
        // R1: fija 300 a A (cap 500). R2: remainder a A (cap 500).
        // Sobrante 1000. R1 pone 300, A=300, room=200. R2 quiere todo (700) pero room=200 → 200.
        // Quedan 500 sin asignar → surplus_cash. Sin regla remainder a otro, ese 500 va a cash.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![
                rule_fixed(0, Decimal::from(300), Some(AllocationCap::Amount(Decimal::from(500)))),
                AllocationRule {
                    target_index: 0,
                    kind: AllocationKind::Remainder,
                    amount: None,
                    cap: Some(AllocationCap::Amount(Decimal::from(500))),
                },
            ],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(500));
        let out = project_net_worth_series(&inp).unwrap();
        // A llega a 500. Sobran 500 → surplus_cash. NW = 500 + 500 = 1000.
        assert_eq!(out.net_worth[1], Decimal::from(1000));
    }

    #[test]
    fn surplus_zero_yields_zero_contributions() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::from(1000),
            vec![a],
            vec![rule_remainder(0)],
        );
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::ZERO);
    }

    #[test]
    fn invalid_target_index_errors() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_remainder(5)], // out of bounds
        );
        match project_net_worth_series(&inp) {
            Err(EngineError::InvalidAllocationRuleTarget) => {}
            other => panic!("expected InvalidAllocationRuleTarget, got {other:?}"),
        }
    }

    #[test]
    fn planning_adjustment_boosts_first_month_contribution_nominals() {
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let mut inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![rule_remainder(0)],
        );
        inp.planning_monthly_cash_adjustment = vec![Decimal::from(250)];
        let nom = first_month_per_asset_contribution_nominals(&inp).unwrap();
        assert_eq!(nom[0], Decimal::from(1250));
    }

    // -----------------------------------------------------------------------
    // FirstMonthAllocation: identidades y traza de la cascada
    // -----------------------------------------------------------------------

    #[test]
    fn first_month_allocation_identities_hold_with_and_without_planning() {
        // Ingreso 3000, gasto 1000, cuota 450 → recurrente 1550. Reglas: fijo 150 con cap de 6
        // meses de gasto, 40 % y sumidero.
        let assets = vec![
            mk_asset(1, Decimal::from(704), true, None),
            mk_asset(2, Decimal::from(50_000), true, None),
            mk_asset(3, Decimal::from(100), false, None),
        ];
        let rules = vec![
            rule_fixed(0, Decimal::from(150), Some(AllocationCap::MonthsExpense(Decimal::from(6)))),
            rule_percent(1, Decimal::from(40), None),
            rule_remainder(2),
        ];
        let mut inp = base_input(1, Decimal::from(3000), Decimal::from(1000), assets, rules);
        inp.liabilities = vec![ProjectionLiabilityInput {
            principal: Decimal::from(100_000),
            monthly_payment: Decimal::from(450),
            payment_end: Some(NaiveDate::from_ymd_opt(2040, 1, 1).unwrap()),
        }];

        // (a) Sin componente de planning: base == neto recurrente.
        let a = first_month_allocation(&inp).unwrap();
        assert_eq!(a.debt_service, Decimal::from(450));
        assert_eq!(a.recurring_net, Decimal::from(1550));
        assert_eq!(a.planning_component, Decimal::ZERO);
        assert_eq!(a.base_cash, a.recurring_net + a.planning_component);
        assert_eq!(
            a.per_asset.iter().sum::<Decimal>() + a.leftover,
            a.base_cash,
            "Σ per_asset + leftover debe cuadrar con la caja repartida"
        );
        // 150 fijo; 40 % de lo que queda (1400) = 560; sumidero se lleva el resto.
        assert_eq!(a.per_asset[0], Decimal::from(150));
        assert_eq!(a.per_asset[1], Decimal::from(560));
        assert_eq!(a.per_asset[2], Decimal::from(840));
        assert_eq!(a.leftover, Decimal::ZERO);
        assert!(a.rules.iter().all(|r| r.skipped_reason.is_none()));

        // (b) Con el tramo transitorio de planning: la base sube exactamente ese importe, y es lo
        // que hace que la «aportación mensual» del mes 1 no cuadre con el neto recurrente.
        inp.planning_monthly_cash_adjustment = vec![Decimal::from(193)];
        let b = first_month_allocation(&inp).unwrap();
        assert_eq!(b.planning_component, Decimal::from(193));
        assert_eq!(b.recurring_net, a.recurring_net, "la parte estable no se mueve");
        assert_eq!(b.base_cash, Decimal::from(1743));
        assert_eq!(b.base_cash, b.recurring_net + b.planning_component);
        assert_eq!(b.per_asset.iter().sum::<Decimal>() + b.leftover, b.base_cash);
        // El wrapper de compatibilidad devuelve exactamente `per_asset`.
        assert_eq!(first_month_per_asset_contribution_nominals(&inp).unwrap(), b.per_asset);
    }

    #[test]
    fn first_month_allocation_traces_every_skip_reason() {
        // Activo 0 ya en su techo (cap_full), activo 1 con regla a cero, activo 2 fijo que agota la
        // caja, activo 3 que ya no se alcanza.
        let assets = vec![
            mk_asset(1, Decimal::from(1000), true, None),
            mk_asset(2, Decimal::ZERO, true, None),
            mk_asset(3, Decimal::ZERO, true, None),
            mk_asset(4, Decimal::ZERO, true, None),
        ];
        let rules = vec![
            rule_fixed(0, Decimal::from(100), Some(AllocationCap::Amount(Decimal::from(1000)))),
            rule_fixed(1, Decimal::ZERO, None),
            rule_fixed(2, Decimal::from(500), None),
            rule_remainder(3),
        ];
        let inp = base_input(1, Decimal::from(500), Decimal::ZERO, assets, rules);
        let a = first_month_allocation(&inp).unwrap();

        assert_eq!(a.rules.len(), 4, "se emite una traza por regla, también las saltadas");
        assert_eq!(a.rules[0].skipped_reason, Some(AllocationSkipReason::CapFull));
        assert_eq!(a.rules[0].cap_ceiling, Some(Decimal::from(1000)));
        assert_eq!(a.rules[0].cap_room, Some(Decimal::ZERO));
        assert_eq!(a.rules[1].skipped_reason, Some(AllocationSkipReason::ZeroAmount));
        assert_eq!(a.rules[2].skipped_reason, None);
        assert_eq!(a.rules[2].amount_resolved, Decimal::from(500));
        // La caja se agotó en la regla 2: la 3 nunca llegó a evaluarse. `NotReached` y `NoCash` son
        // diagnósticos distintos («las de arriba se lo comieron» vs «no te sobra dinero») y por eso
        // no se colapsan.
        assert_eq!(a.rules[3].skipped_reason, Some(AllocationSkipReason::NotReached));
        assert_eq!(a.per_asset.iter().sum::<Decimal>() + a.leftover, a.base_cash);
    }

    #[test]
    fn first_month_allocation_reports_no_cash_for_every_rule() {
        let assets = vec![mk_asset(1, Decimal::ZERO, true, None)];
        let rules = vec![rule_fixed(0, Decimal::from(100), None), rule_remainder(0)];
        // Gasto por encima del ingreso: no hay caja que repartir.
        let inp = base_input(1, Decimal::from(500), Decimal::from(900), assets, rules);
        let a = first_month_allocation(&inp).unwrap();
        assert_eq!(a.base_cash, Decimal::from(-400));
        assert_eq!(a.recurring_net, Decimal::from(-400));
        assert_eq!(a.per_asset[0], Decimal::ZERO);
        assert_eq!(a.leftover, Decimal::ZERO, "con caja negativa no hay sobrante");
        assert_eq!(a.rules.len(), 2);
        assert!(a
            .rules
            .iter()
            .all(|r| r.skipped_reason == Some(AllocationSkipReason::NoCash)));
    }

    #[test]
    fn first_month_allocation_reports_a_capped_rule_as_trimmed_not_skipped() {
        // La regla pide 500 pero solo caben 200 bajo el techo: no es un salto, es un recorte.
        let assets = vec![
            mk_asset(1, Decimal::from(800), true, None),
            mk_asset(2, Decimal::ZERO, true, None),
        ];
        let rules = vec![
            rule_fixed(0, Decimal::from(500), Some(AllocationCap::Amount(Decimal::from(1000)))),
            rule_remainder(1),
        ];
        let inp = base_input(1, Decimal::from(1000), Decimal::ZERO, assets, rules);
        let a = first_month_allocation(&inp).unwrap();
        assert_eq!(a.rules[0].skipped_reason, None, "recortada no es saltada");
        assert_eq!(a.rules[0].amount_intent, Decimal::from(500));
        assert_eq!(a.rules[0].amount_resolved, Decimal::from(200));
        assert_eq!(a.rules[0].cap_room, Some(Decimal::from(200)));
        assert_eq!(a.per_asset[0], Decimal::from(200));
        assert_eq!(a.per_asset[1], Decimal::from(800));
    }

    #[test]
    fn contributed_capital_month_zero_includes_purchase_prices() {
        let mut inp = base_input(
            2,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(99),
                value: Decimal::from(100_000),
                purchase_price: Some(Decimal::from(80_000)),
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![],
        );
        inp.planning_monthly_cash_adjustment = vec![Decimal::ZERO; 2];
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(80_000));
        assert_eq!(out.contributed_capital[1], Decimal::from(80_000));
        assert_eq!(out.contributed_capital[2], Decimal::from(80_000));
    }

    #[test]
    fn contributed_capital_tracks_purchase_basis_plus_routed_surplus() {
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(11),
                value: Decimal::ZERO,
                purchase_price: Some(Decimal::from(1200)),
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![rule_remainder(0)],
        );
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(1200));
        assert_eq!(out.contributed_capital[1], Decimal::from(2200));
    }

    #[test]
    fn retirement_withdrawal_drains_assets_after_start_month() {
        let mut inp = base_input(
            4,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(10),
                value: Decimal::from(12_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![],
        );
        inp.retirement_start_month = Some(3);
        inp.retirement_monthly_withdrawal = Decimal::from(1_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(12_000));
        assert_eq!(out.net_worth[1], Decimal::from(12_000));
        assert_eq!(out.net_worth[2], Decimal::from(12_000));
        assert_eq!(out.net_worth[3], Decimal::from(11_000));
        assert_eq!(out.net_worth[4], Decimal::from(10_000));
    }

    #[test]
    fn retirement_income_drops_to_income_retirement_monthly_after_start_month() {
        let mut inp = base_input(
            5,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![SimAsset {
                id: Uuid::from_u128(40),
                value: Decimal::from(10_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            vec![rule_remainder(0)],
        );
        inp.retirement_start_month = Some(3);
        inp.income_retirement_monthly = Decimal::from(500);
        inp.expense_retirement_monthly = Decimal::from(2_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(10_000));
        assert_eq!(out.net_worth[1], Decimal::from(11_000));
        assert_eq!(out.net_worth[2], Decimal::from(12_000));
        assert_eq!(out.net_worth[3], Decimal::from(10_500));
        assert_eq!(out.net_worth[4], Decimal::from(9_000));
        assert_eq!(out.net_worth[5], Decimal::from(7_500));
    }

    #[test]
    fn fire_target_with_inflation_does_not_trigger_early_drain() {
        let mut inp = base_input(
            240,
            Decimal::from(1_000),
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(99),
                value: Decimal::ZERO,
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(7)),
            }],
            vec![rule_remainder(0)],
        );
        inp.planning_monthly_cash_adjustment = vec![Decimal::ZERO; 240];
        inp.fire_target = Some(FireTarget {
            base_amount: Decimal::from(50_000),
            annual_inflation_percent: Decimal::from(10),
        });
        let out = project_net_worth_series(&inp).unwrap();
        let cross_idx = out
            .net_worth
            .iter()
            .enumerate()
            .find(|(k, v)| {
                let target = fire_target_at_month_index(inp.fire_target.as_ref(), *k as u32)
                    .unwrap_or(Decimal::ZERO);
                **v >= target && target > Decimal::ZERO
            })
            .map(|(k, _)| k)
            .expect("debe cruzar el target móvil en el horizonte");
        for i in 1..=cross_idx {
            assert!(
                out.net_worth[i] >= out.net_worth[i - 1] - Decimal::new(1, 2),
                "serie cayó en mes {i} (prev={}, curr={}) — fire_reached prematuro",
                out.net_worth[i - 1],
                out.net_worth[i]
            );
        }
    }

    #[test]
    fn asset_growth_is_nominal_independent_of_currency_value() {
        let inp = base_input(
            12,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![SimAsset {
                id: Uuid::from_u128(40),
                value: Decimal::from(50_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(5)),
            }],
            vec![],
        );
        let out = project_net_worth_series(&inp).unwrap();
        let expected = Decimal::from(52_500);
        let diff = (out.net_worth[12] - expected).abs();
        assert!(
            diff < Decimal::new(50, 2),
            "esperado ≈ {expected}, obtenido {} (diff {diff})",
            out.net_worth[12]
        );
    }

    #[test]
    fn fire_target_grows_with_inflation_each_month() {
        let ft = FireTarget {
            base_amount: Decimal::from(750_000),
            annual_inflation_percent: Decimal::from(3),
        };
        let t0 = fire_target_at_month_index(Some(&ft), 0).unwrap();
        assert_eq!(t0, Decimal::from(750_000));
        let t20y = fire_target_at_month_index(Some(&ft), 240).unwrap();
        // 750_000 × 1.03^20 ≈ 1_354_583. Comprobamos con tolerancia ≤ 1€.
        let factor = (Decimal::ONE + Decimal::from(3) / Decimal::from(100u32))
            .powd(Decimal::from(20u32));
        let expected = Decimal::from(750_000) * factor;
        let diff = (t20y - expected).abs();
        assert!(
            diff < Decimal::ONE,
            "esperado ≈ {expected}, obtenido {t20y} (diff {diff})"
        );
        // El target a 10 años está estrictamente entre el base y el de 20 años.
        let t10y = fire_target_at_month_index(Some(&ft), 120).unwrap();
        assert!(t10y > Decimal::from(750_000));
        assert!(t10y < t20y);
    }

    #[test]
    fn fire_target_with_zero_inflation_is_flat() {
        let ft = FireTarget {
            base_amount: Decimal::from(500_000),
            annual_inflation_percent: Decimal::ZERO,
        };
        assert_eq!(
            fire_target_at_month_index(Some(&ft), 0).unwrap(),
            Decimal::from(500_000)
        );
        assert_eq!(
            fire_target_at_month_index(Some(&ft), 600).unwrap(),
            Decimal::from(500_000)
        );
    }

    /// Regresión Fase 1.5: con el helper nuevo, el target en month_index=12 corresponde a 1 año
    /// completo de inflación. Antes, el handler usaba `month_index/12` y el motor `(k-1)/12` —
    /// había una diferencia de un mes entre la serie que se devolvía al cliente y el cruce que
    /// disparaba `fire_reached`. Ahora ambos consumen `fire_target_at_month_index`.
    #[test]
    fn fire_target_helper_matches_compound_factor_at_year_boundaries() {
        let ft = FireTarget {
            base_amount: Decimal::from(100_000),
            annual_inflation_percent: Decimal::from(5),
        };
        let r = Decimal::ONE + Decimal::from(5) / Decimal::from(100u32);

        let t1y = fire_target_at_month_index(Some(&ft), 12).unwrap();
        let expected_1y = Decimal::from(100_000) * r;
        assert!(
            (t1y - expected_1y).abs() < Decimal::new(1, 4),
            "month_index=12 → 1 año compuesto"
        );

        let t5y = fire_target_at_month_index(Some(&ft), 60).unwrap();
        let expected_5y = Decimal::from(100_000) * r.powd(Decimal::from(5u32));
        assert!(
            (t5y - expected_5y).abs() < Decimal::new(1, 4),
            "month_index=60 → 5 años compuesto"
        );
    }

    /// REGRESIÓN — un hogar que YA está por encima de su número FIRE no aporta: drena.
    ///
    /// `first_month_allocation` no miraba `fire_target`, así que publicaba la cascada del mes 1
    /// como si el hogar siguiera acumulando, mientras el bucle de simulación —que sí lo mira—
    /// conmutaba a ingreso de jubilación y vendía activos ese mismo mes. `/v1/assets` decía
    /// «aportas 2.000 €» sobre un activo que la proyección **reduce en 1.000 €**: 3.000 € de
    /// error y con el signo cambiado, sostenido en todo el horizonte porque el patrimonio nunca
    /// vuelve a bajar del target.
    ///
    /// Aritmética: gasto de jubilación 1.000 €/mes ⇒ necesidad anual 12.000 ⇒ con SWR 3,5 % el
    /// target es 342.857,14 €. El activo vale 1.000.000, así que el cruce es inmediato.
    #[test]
    fn already_fire_at_month_zero_drains_instead_of_contributing() {
        let mut inp = base_input(
            3,
            Decimal::from(3000),
            Decimal::from(1000),
            vec![mk_asset(1, Decimal::from(1_000_000), true, Some(Decimal::ZERO))],
            vec![rule_remainder(0)],
        );
        inp.expense_retirement_monthly = Decimal::from(1000);
        inp.fire_target = Some(FireTarget {
            base_amount: Decimal::from_str_exact("342857.142857").unwrap(),
            annual_inflation_percent: Decimal::ZERO,
        });

        let out = project_net_worth_series(&inp).expect("simulación");
        assert_eq!(
            out.per_asset_series[0][1],
            Decimal::from(999_000),
            "la simulación drena 1.000 € en el mes 1: {:?}",
            out.per_asset_series[0]
        );

        let alloc = first_month_allocation(&inp).expect("cascada del mes 1");
        assert_eq!(
            alloc.per_asset[0],
            Decimal::ZERO,
            "la aportación publicada debe ser 0, no la cascada de un hogar que ya no aporta"
        );
        assert_eq!(
            alloc.recurring_net,
            Decimal::from(-1000),
            "el neto recurrente publicado debe ser el de jubilación (0 − 1.000), no 3.000 − 1.000"
        );

        // Y el mismo input SIN target sigue comportándose como antes: nada se ha roto para el
        // hogar que aún acumula.
        let mut sin_target = inp.clone();
        sin_target.fire_target = None;
        let alloc2 = first_month_allocation(&sin_target).expect("cascada sin target");
        assert_eq!(alloc2.per_asset[0], Decimal::from(2000), "sin target FIRE la cascada es la de siempre");
    }

    // -----------------------------------------------------------------------
    // Pin baseline del modelo de pasivos ANTES de la reforma 4.2.0
    // -----------------------------------------------------------------------

    /// Decimal exacto desde string. Los pines de abajo son cadenas largas (hasta 27 dígitos
    /// significativos) que salen de componer `powd` 300 veces: escribirlas con `Decimal::new`
    /// sería ilegible y con `f64` sería directamente ilegal en este repo.
    fn dec(s: &str) -> Decimal {
        Decimal::from_str_exact(s).expect("literal Decimal válido")
    }

    /// Input representativo del pin: dos activos con rentabilidad distinta, cascada con tope +
    /// sumidero, un pasivo de 100.000 € a 500 €/mes con `payment_end` lejano, planning ≠ 0 y
    /// target FIRE configurado. Vive fuera del test para que la reforma 4.2.0 pueda reutilizarlo
    /// añadiendo los campos nuevos a `None`/default sin tocar los valores esperados.
    fn liability_pin_input() -> ProjectionInput {
        let assets = vec![
            mk_asset(0xA1, Decimal::from(50_000), true, Some(Decimal::from(7))),
            mk_asset(0xB2, Decimal::from(20_000), false, Some(Decimal::new(35, 1))),
        ];
        let rules = vec![
            rule_fixed(
                0,
                Decimal::from(300),
                Some(AllocationCap::Amount(Decimal::from(120_000))),
            ),
            rule_remainder(1),
        ];
        let mut inp = base_input(300, Decimal::from(4000), Decimal::from(1500), assets, rules);
        inp.liabilities = vec![ProjectionLiabilityInput {
            principal: Decimal::from(100_000),
            monthly_payment: Decimal::from(500),
            payment_end: Some(NaiveDate::from_ymd_opt(2090, 1, 1).unwrap()),
        }];
        inp.planning_monthly_cash_adjustment[0] = Decimal::from(250);
        inp.planning_monthly_cash_adjustment[5] = Decimal::from(-100);
        inp.fire_target = Some(FireTarget {
            base_amount: Decimal::from(1_500_000),
            annual_inflation_percent: Decimal::new(25, 1),
        });
        inp
    }

    /// **Pin de regresión pre-4.2.0 del modelo de pasivos.**
    ///
    /// Captura la salida EXACTA de `project_net_worth_series` sobre un input representativo con un
    /// pasivo vivo, tal y como la produce el modelo actual (`fixed_payments`): la cuota mensual
    /// sale de la caja del mes y reduce el principal en el MISMO importe — el pasivo **no devenga
    /// intereses**, así que servir deuda es neutro para el patrimonio neto.
    ///
    /// Por qué existe: la reforma 4.2.0 introduce el cobro de intereses en los pasivos. Este test
    /// se conserva tal cual tras la reforma (con los campos nuevos del pasivo a `None`/default) y
    /// **debe seguir pasando bit a bit**: es la prueba de que el modelo por defecto
    /// `fixed_payments` no cambió NADA. Si un solo dígito se mueve, la reforma ha cambiado el
    /// comportamiento por defecto y no solo añadido uno nuevo.
    ///
    /// Qué pinea, en concreto:
    /// 1. `net_worth` en los meses 0, 1, 2, 12, 200, 201 y el último (300) — valores Decimal
    ///    exactos, `assert_eq!` sin tolerancia. La aritmética de `rust_decimal` (incluido `powd`)
    ///    es determinista y pura, así que no hace falta margen: cualquier deriva es una deriva
    ///    real del modelo, no ruido de coma flotante.
    /// 2. El calendario de amortización: con 100.000 € de principal y 500 €/mes el pasivo se
    ///    extingue en el mes **200** (verificado contra la salida real, no asumido). Se pinea
    ///    mediante el principal implícito (`Σ activos − net_worth`, exacto aquí porque el
    ///    sumidero deja `surplus_cash` en 0) y mediante los NW alrededor del corte.
    /// 3. La **neutralidad** del pago: el mes 201 es el primero sin cuota (`debt_service = 0`) y
    ///    aun así el salto de NW 200→201 NO da el escalón de +500 € que daría si el pago
    ///    estuviera restando patrimonio. Con intereses, este assert dejaría de cuadrar — que es
    ///    justo lo que la reforma debe cambiar SOLO cuando se activa el modelo nuevo.
    /// 4. `debt_service` del primer mes, que `FirstMonthAllocation` sí expone.
    #[test]
    fn liability_payment_plan_series_pin_pre_4_2_0() {
        let inp = liability_pin_input();
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth.len(), 301, "301 puntos: meses 0..=300");

        // (1) Serie de patrimonio neto — valores exactos capturados sobre 4.1.0.
        assert_eq!(out.net_worth[0], dec("-30000"), "mes 0 = 50.000 + 20.000 − 100.000");
        assert_eq!(out.net_worth[1], dec("-26902.580260129782587857672390"));
        assert_eq!(out.net_worth[2], dec("-24046.794776804757383934090581"));
        assert_eq!(out.net_worth[12], dec("4876.52949312863908655995354"));
        assert_eq!(out.net_worth[200], dec("754661.26626719402043993492188"));
        assert_eq!(out.net_worth[201], dec("759950.40876629663413040639649"));
        assert_eq!(out.net_worth[300], dec("1389197.0697233021375734775522"));

        // (2) Calendario de amortización: 100.000 / 500 = 200 cuotas exactas. `surplus_cash` es 0
        // en todo momento (la regla sumidero absorbe la caja entera), así que el principal vivo se
        // reconstruye exactamente como `Σ activos − net_worth`.
        let principal_at = |k: usize| -> Decimal {
            out.per_asset_series.iter().map(|s| s[k]).sum::<Decimal>() - out.net_worth[k]
        };
        assert_eq!(principal_at(0), Decimal::from(100_000));
        assert_eq!(principal_at(1), Decimal::from(99_500), "cuota íntegra a principal: 0 % interés");
        assert_eq!(principal_at(199), Decimal::from(500), "queda la última cuota");
        assert_eq!(principal_at(200), Decimal::ZERO, "el pasivo se extingue en el mes 200");
        assert_eq!(principal_at(201), Decimal::ZERO, "y no resucita");

        // (3) Neutralidad del servicio de deuda. El mes 201 es el primero sin cuota, así que
        // dispone de 500 € más de caja para aportar; pero hasta el 200 esos 500 € tampoco se
        // perdían — iban íntegros a reducir principal. Resultado: la serie NO tiene escalón de
        // 500 €. Se comprueba sobre las segundas diferencias del NW.
        let delta = |k: usize| out.net_worth[k] - out.net_worth[k - 1];
        let salto_previo = delta(200) - delta(199);
        let salto_en_el_corte = delta(201) - delta(200);
        assert_eq!(salto_previo, dec("17.08730926366701766536981"));
        assert_eq!(salto_en_el_corte, dec("18.59126818624123937199547"));
        // El único efecto real de extinguir el pasivo es de segundo orden y ~1,44 €: los 500 €
        // liberados dejan de amortizar (crecimiento 0) y pasan a componer un mes en el activo
        // destino, 500 × (1,035^(1/12) − 1). Con intereses en el pasivo, este residuo dejaría de
        // ser calderilla — que es exactamente lo que la reforma 4.2.0 debe cambiar SOLO cuando se
        // activa el modelo nuevo.
        let residuo = salto_en_el_corte - salto_previo;
        assert!(
            residuo.abs() < Decimal::from(5),
            "sin intereses la extinción del pasivo no produce escalón de 500 €; residuo = {residuo}"
        );

        // (4) Servicio de deuda publicado del primer mes (única superficie del output que lo
        // expone) y la caja que reparte la cascada: 4000 − 1500 − 500 + 250 de planning.
        let alloc = first_month_allocation(&inp).unwrap();
        assert_eq!(alloc.debt_service, Decimal::from(500));
        assert_eq!(alloc.base_cash, Decimal::from(2250));
        assert_eq!(alloc.per_asset, vec![Decimal::from(300), Decimal::from(1950)]);
    }
}
