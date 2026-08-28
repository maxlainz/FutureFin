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

/// Modelo de amortización de un pasivo: decide cómo se reparte la cuota mensual entre intereses
/// y principal. Enum **propio del crate** (mismo patrón que [`AllocationKind`]): el engine no
/// conoce la columna SQL ni sus literales, el handler mapea.
///
/// `FixedPayments` es el modelo histórico (pre-4.2.0) y el default de la columna: la cuota va
/// íntegra a principal, el pasivo no devenga intereses. Los otros tres sí los devengan.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepaymentModel {
    /// Sin intereses: la cuota reduce el principal en su importe exacto. Modelo pre-4.2.0.
    FixedPayments,
    /// Sistema francés: interés sobre el saldo de apertura, cuota a fin de mes.
    French,
    /// Solo intereses: la cuota paga el devengo y el principal queda constante.
    InterestOnly,
    /// Línea revolving. En 4.2.0 comparte recurrencia con [`RepaymentModel::French`]
    /// (ver `revolving_matches_french_recurrence`); existe como concepto separado porque su
    /// evolución futura —disposiciones, cuota mínima como % del saldo— sí diverge.
    Revolving,
}

#[derive(Debug, Clone)]
pub struct ProjectionLiabilityInput {
    pub principal: Decimal,
    pub monthly_payment: Decimal,
    pub payment_end: Option<NaiveDate>,
    /// Cómo se cobra la cuota. `FixedPayments` reproduce bit a bit el modelo pre-4.2.0.
    pub repayment_model: RepaymentModel,
    /// TIN **nominal anual** en puntos porcentuales (3 = 3 %/año). El tipo mensual es
    /// `i = apr_percent / 1200`, la MISMA convención que `LoanTerms::apr_percent` en
    /// `history.rs` (`i = apr / 1200`), para que la curva histórica y la proyectada hablen el
    /// mismo idioma.
    ///
    /// `None` o `≤ 0` ⇒ **sin interés**: cualquier modelo degenera exactamente en
    /// `FixedPayments` (y `InterestOnly` en un principal congelado). La degeneración es total y
    /// deliberada: el import de un `.ffbackup` puede colar un pasivo `french` sin TIN, y el
    /// engine jamás debe panicar ni devolver error por eso — devuelve la serie sin intereses.
    pub apr_percent: Option<Decimal>,
    /// **Amortización extra mensual** por encima de la cuota (eje what-if, 4.4.0). `0` reproduce
    /// bit a bit el comportamiento anterior.
    ///
    /// Vive en el input del MOTOR y no como un ajuste de caja del handler porque amortizar es
    /// dos cosas a la vez: una **salida de caja** y una **reducción de principal**. Un ajuste de
    /// caja solo puede hacer la primera — y hacer solo la primera es exactamente el «gasto
    /// puntual» que ya existía y que responde a otra pregunta (drena caja sin tocar la deuda ni
    /// liberar la cuota). Al ser las dos, el efecto instantáneo sobre el patrimonio es **nulo**
    /// (−caja, +principal amortizado): la ganancia aparece después, en los intereses que ya no
    /// se devengan y en la cuota que se libera antes.
    ///
    /// Se aplica DESPUÉS de la cuota del mes, se topa al saldo de cierre (jamás deja el
    /// principal en negativo ni «paga de más») y **solo con plan de pago activo**, igual que el
    /// devengo: sin cuota no hay nada que adelantar.
    pub extra_principal_monthly: Decimal,
    /// **Amortizaciones puntuales** `(mes 1-based, importe)` (eje what-if, 4.4.0). Misma
    /// semántica que [`Self::extra_principal_monthly`] pero en un solo mes; varias entradas del
    /// mismo mes se suman. Vacío = ninguna.
    pub extra_principal_lump_sums: Vec<(u32, Decimal)>,
}

/// ¿Tiene el pasivo un plan de pago vivo en el mes que empieza en `m_start`?
///
/// Predicado ÚNICO: `monthly_payment > 0` **y** (`payment_end` ausente o `>= m_start`). Estaba
/// triplicado (cobro de la cuota, amortización del principal y `first_month_allocation`), tres
/// copias que había que mantener sincronizadas a mano. Sin plan activo el pasivo no cobra caja,
/// no amortiza y —desde 4.2.0— **tampoco devenga intereses**: es una resta constante al
/// patrimonio, que es justo el contrato que explotan los modos B/C del handler (pasan
/// `monthly_payment = 0` para congelar el principal).
fn liability_active(liab: &ProjectionLiabilityInput, m_start: NaiveDate) -> bool {
    liab.monthly_payment > Decimal::ZERO
        && match liab.payment_end {
            None => true,
            Some(end) => end >= m_start,
        }
}

/// Un mes de vida de un pasivo: devuelve `(caja que sale, principal de cierre)`.
///
/// Única implementación de la recurrencia — la consumen el bucle de simulación (para el
/// `debt_service` y para el principal del mes siguiente) y `first_month_allocation`. Dos
/// implementaciones divergirían en silencio y el chart contaría una historia distinta que la
/// KPI de aportación.
///
/// Convención común a todos los modelos que devengan: **interés sobre el saldo de apertura y
/// cuota a fin de mes**, `P' = P·(1 + i) − M` — la misma recurrencia que `theo(y)` en
/// `history.rs`, para que la interpolación del pasado y la proyección del futuro sean la misma
/// curva.
///
/// - inactivo → `(0, P)`: ni caja, ni amortización, ni devengo.
/// - `FixedPayments` → `cash = min(M, P)`, `P' = P − cash`. **Bit-idéntico** al modelo pre-4.2.0.
/// - `French` / `Revolving` → `payoff = P·(1 + i)`, `cash = min(M, payoff)`, `P' = payoff − cash`.
///   Con `i = 0` degenera exactamente en `FixedPayments`. El tope de la cuota es el **payoff**,
///   no el principal: cancelar el préstamo cuesta el saldo *con* el interés del mes.
/// - `InterestOnly` → `cash = min(M, P)` y `P' = P` (constante). El TIN es informativo aquí: la
///   cuota que el usuario declara YA es el interés que paga; recalcularlo lo cobraría dos veces.
///
/// **Saturación, nunca pánico**: si el `checked_mul`/`checked_add` del payoff desborda (TIN
/// absurdo × horizonte largo), se devuelve el principal sin devengar más. La salida sigue siendo
/// finita y la simulación termina.
fn liability_month(
    model: RepaymentModel,
    principal: Decimal,
    monthly_payment: Decimal,
    apr_percent: Option<Decimal>,
    active: bool,
) -> (Decimal, Decimal) {
    if !active {
        return (Decimal::ZERO, principal);
    }
    let i = match apr_percent {
        Some(apr) if apr > Decimal::ZERO => apr / Decimal::from(1200),
        _ => Decimal::ZERO,
    };
    match model {
        RepaymentModel::FixedPayments => {
            let cash = monthly_payment.min(principal).max(Decimal::ZERO);
            (cash, principal - cash)
        }
        RepaymentModel::InterestOnly => {
            let cash = monthly_payment.min(principal).max(Decimal::ZERO);
            (cash, principal)
        }
        RepaymentModel::French | RepaymentModel::Revolving => {
            let payoff = Decimal::ONE
                .checked_add(i)
                .and_then(|factor| principal.checked_mul(factor))
                .unwrap_or(principal);
            let cash = monthly_payment.min(payoff).max(Decimal::ZERO);
            (cash, payoff - cash)
        }
    }
}

/// Amortización extra del mes `month` (1-based), ya topada al saldo que quedaría tras la cuota.
///
/// Única implementación, como [`liability_month`]: la consumen el bucle de simulación, el
/// calendario de amortización y `first_month_allocation`. Devuelve siempre un importe en
/// `0 ..= closing_after_payment`, así que sumarla al servicio de deuda y restarla del principal
/// no puede producir ni caja fantasma ni principal negativo.
///
/// Sin plan de pago activo devuelve 0: amortizar «extra» un pasivo que no cobra cuota no
/// adelanta nada (no hay devengo que evitar ni cuota que liberar) y además rompería el contrato
/// de los modos B/C del handler, donde el principal es una resta CONSTANTE al patrimonio.
fn liability_extra_principal(
    liab: &ProjectionLiabilityInput,
    month: u32,
    closing_after_payment: Decimal,
    active: bool,
) -> Decimal {
    if !active {
        return Decimal::ZERO;
    }
    let mut wanted = liab.extra_principal_monthly.max(Decimal::ZERO);
    for (m, amount) in &liab.extra_principal_lump_sums {
        if *m == month {
            wanted += (*amount).max(Decimal::ZERO);
        }
    }
    wanted
        .min(closing_after_payment.max(Decimal::ZERO))
        .max(Decimal::ZERO)
}

/// Valor actual de una renta de `months` cuotas mensuales de `monthly_payment`, descontada al
/// TIN nominal anual `apr_percent` (misma convención que
/// [`ProjectionLiabilityInput::apr_percent`]: `i = apr / 1200`):
///
/// `P = M · (1 − (1 + i)^−n) / i`
///
/// `apr_percent` ausente o `≤ 0` devuelve `M · n` **exacto**, sin pasar por `powd`: el límite de
/// la fórmula cuando `i → 0` es justo eso, y calcularlo con la transcendental metería error de
/// redondeo en el caso más común y más fácil. `n` puede ser fraccionario (equivalencias
/// semanales: `M = cuota·52/12`, `n = intervalos·12/52`). Si algún `checked_*` falla, cae al
/// mismo `M · n`.
pub fn present_value_of_payments(
    monthly_payment: Decimal,
    months: Decimal,
    apr_percent: Option<Decimal>,
) -> Decimal {
    let plain = monthly_payment * months;
    let i = match apr_percent {
        Some(apr) if apr > Decimal::ZERO => apr / Decimal::from(1200),
        _ => return plain,
    };
    (|| -> Option<Decimal> {
        let u = Decimal::ONE.checked_add(i)?;
        // `(1+i)^−n` como `1 / (1+i)^n`: `powd` con exponente positivo es el camino probado en
        // este crate (`history.rs` lo usa así) y evita depender de cómo trate el exponente
        // negativo.
        let u_pow_n = u.checked_powd(months)?;
        let discount = Decimal::ONE.checked_div(u_pow_n)?;
        let numerator = Decimal::ONE.checked_sub(discount)?;
        monthly_payment.checked_mul(numerator)?.checked_div(i)
    })()
    .unwrap_or(plain)
}

// ---------------------------------------------------------------------------
// Calendario de amortización (4.4.0)
// ---------------------------------------------------------------------------

/// Tope del calendario de amortización: 70 años, el mismo `MAX_PROJECTION_MONTHS` del horizonte
/// de proyección. No es una elección de producto sino de terminación: `interest_only` y una cuota
/// por debajo del interés **nunca** extinguen la deuda, así que el bucle necesita una cota dura
/// que además sea la misma que ya acota la simulación (un calendario más largo que el horizonte
/// describiría meses que ninguna otra superficie modela).
pub const MAX_LIABILITY_SCHEDULE_MONTHS: u32 = 840;

/// Por qué el calendario **no** llega a saldar la deuda. `None` ⟺ hay `payoff_month_index`.
///
/// Las cuatro razones tienen remedios distintos y por eso no se colapsan en un booleano, mismo
/// criterio que [`AllocationSkipReason`]: «no tienes plan de pago» se arregla dando de alta la
/// cuota, «tu plan se acaba antes» se arregla alargando la fecha, «la cuota no reduce el
/// principal» se arregla subiendo la cuota, y «no cabe en 70 años» es simplemente un préstamo
/// larguísimo. El engine no conoce literales de wire: el handler los mapea.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiabilityPayoffAbsence {
    /// El pasivo no tiene plan de pago activo en el primer mes (`monthly_payment ≤ 0`, o
    /// `payment_end` ya pasado). El calendario sale **vacío**: no hay nada que devengar ni
    /// amortizar, y el principal es una resta constante al patrimonio.
    NoPaymentPlan,
    /// El plan de pago (`payment_end`) termina con principal vivo. El calendario acaba en el
    /// último mes con cuota y `final_principal` es lo que queda debiendo.
    PaymentPlanEndsBeforePayoff,
    /// Al agotar el horizonte el principal **no ha bajado** respecto al de partida: es
    /// `interest_only` (principal congelado por definición) o una cuota por debajo del interés
    /// devengado (amortización negativa: la deuda crece).
    PaymentDoesNotReducePrincipal,
    /// El principal baja, pero no llega a cero dentro de los meses simulados.
    NotWithinHorizon,
}

/// Un mes del calendario de amortización.
///
/// **Identidad exacta en `Decimal`, en los cuatro modelos y en todos los meses**:
/// `payment + extra_principal == interest_accrued + principal_repaid`.
/// Se cumple *por construcción* — `principal_repaid` se deriva de los saldos
/// (`opening − closing`) y `interest_accrued` de la cuota (`payment − lo que la cuota amortizó`),
/// nunca al revés. Una implementación que devengara el interés por su cuenta podría separarse de
/// [`liability_month`] en silencio, que es la recurrencia que de verdad pinta el chart.
#[derive(Debug, Clone)]
pub struct LiabilityScheduleMonth {
    /// Mes 1-based, misma base que el bucle de simulación: el mes `k` es el mes civil que empieza
    /// en `month_first_calendar(ref_date) + (k−1)`. **No es una posición de array** — el
    /// calendario puede empezar en un mes distinto de 1 si el llamante lo recorta.
    pub month_index: u32,
    /// Saldo al abrir el mes (antes del devengo).
    pub opening_principal: Decimal,
    /// Interés devengado en el mes. `0` en `fixed_payments` (no devenga) y en cualquier modelo con
    /// TIN ausente o ≤ 0. En `interest_only` es la cuota entera: el principal no se mueve, así que
    /// todo lo que paga es interés.
    pub interest_accrued: Decimal,
    /// Principal amortizado en el mes, cuota **y** amortización extra incluidas
    /// (`opening_principal − closing_principal`). Puede ser **negativo** cuando la cuota no cubre
    /// el devengo: la deuda crece, y publicarlo como 0 escondería justo eso.
    pub principal_repaid: Decimal,
    /// Parte de `principal_repaid` que viene de una amortización extra what-if. `0` en el
    /// calendario real de un pasivo guardado.
    pub extra_principal: Decimal,
    /// Caja que sale por la **cuota** (topada al saldo de cancelación del mes: el último mes de un
    /// préstamo es de cuota parcial). No incluye `extra_principal`.
    pub payment: Decimal,
    /// Saldo al cerrar el mes. Nunca negativo.
    pub closing_principal: Decimal,
}

/// Calendario de amortización completo de UN pasivo, más los agregados que responden las dos
/// preguntas que hoy son incontestables desde el chat: «¿cuánto pago de intereses?» y «¿cuándo
/// termino?».
///
/// **Cero matemática nueva**: cada mes sale de [`liability_month`] —la misma recurrencia que el
/// bucle de proyección ejecuta hasta 840 veces por request y cuyo principal de cierre tiraba a la
/// basura— más [`liability_extra_principal`]. Por eso el mes de extinción que devuelve este
/// calendario y el que se deduce de `ProjectionOutput` son el mismo número, no dos aproximaciones.
#[derive(Debug, Clone)]
pub struct LiabilitySchedule {
    /// Meses simulados, en orden, desde el mes 1 hasta la extinción, el fin del plan de pago o el
    /// horizonte — lo que llegue antes. Vacío ⟺ no había plan de pago activo (o el principal ya
    /// era 0).
    pub months: Vec<LiabilityScheduleMonth>,
    /// Saldo de partida (mes 0), ya saneado a ≥ 0.
    pub opening_principal: Decimal,
    /// Saldo tras el último mes simulado.
    pub final_principal: Decimal,
    /// Σ `interest_accrued`. Es el interés que **queda por pagar** desde hoy: el calendario
    /// arranca en el saldo actual, no en el original del préstamo.
    pub total_interest: Decimal,
    /// Σ `payment` (solo cuotas).
    pub total_payments: Decimal,
    /// Σ `extra_principal`.
    pub total_extra_principal: Decimal,
    /// `total_payments + total_extra_principal`: todo lo que sale de la caja. Es el «total a
    /// pagar» de la pregunta.
    pub total_cash_out: Decimal,
    /// Mes en que el saldo llega a **cero exacto**. `Some(0)` ⟺ el pasivo ya estaba saldado al
    /// arrancar. `None` ⟺ hay `payoff_absent`.
    pub payoff_month_index: Option<u32>,
    /// Por qué no hay mes de extinción. **Invariante**: `payoff_month_index.is_some() ==
    /// payoff_absent.is_none()`.
    pub payoff_absent: Option<LiabilityPayoffAbsence>,
    /// Meses que se llegaron a simular como cota (tras el clamp a [`MAX_LIABILITY_SCHEDULE_MONTHS`]),
    /// no los que salen en `months`.
    pub horizon_months: u32,
}

/// Calendario de amortización de un pasivo desde `ref_date`, mes a mes.
///
/// Pura y determinista como el resto del crate. `horizon_months` se clampa a
/// `1..=MAX_LIABILITY_SCHEDULE_MONTHS`; el bucle además corta antes en cuanto (a) el saldo llega a
/// cero o (b) el plan de pago deja de estar activo — y esa segunda condición es **monótona**
/// (`monthly_payment > 0` no cambia con el tiempo y `payment_end >= inicio_de_mes` solo puede
/// pasar de cierta a falsa), así que cortar ahí no se salta ningún mes con actividad.
pub fn liability_amortization_schedule(
    liab: &ProjectionLiabilityInput,
    ref_date: NaiveDate,
    horizon_months: u32,
) -> LiabilitySchedule {
    let horizon = horizon_months.clamp(1, MAX_LIABILITY_SCHEDULE_MONTHS);
    let opening_principal = liab.principal.max(Decimal::ZERO);
    let start_month_first = month_first_calendar(ref_date);

    let mut principal = opening_principal;
    let mut months: Vec<LiabilityScheduleMonth> = Vec::new();
    let mut total_interest = Decimal::ZERO;
    let mut total_payments = Decimal::ZERO;
    let mut total_extra_principal = Decimal::ZERO;
    // Un pasivo con saldo 0 ya está extinguido HOY: `Some(0)` y calendario vacío. No se deja caer
    // al bucle porque emitiría meses de ceros que no describen nada.
    let mut payoff_month_index = principal.is_zero().then_some(0u32);
    let mut plan_ended = false;

    if !principal.is_zero() {
        for k in 1..=horizon {
            let month_first = add_months(start_month_first, k - 1);
            let (m_start, _m_end) = month_window(month_first);
            let active = liability_active(liab, m_start);
            if !active {
                plan_ended = true;
                break;
            }

            let (payment, closing_after_payment) = liability_month(
                liab.repayment_model,
                principal,
                liab.monthly_payment,
                liab.apr_percent,
                true,
            );
            let extra = liability_extra_principal(liab, k, closing_after_payment, true);
            let closing = closing_after_payment - extra;

            // Derivación en este orden a propósito: los saldos mandan, el interés es el residuo.
            // Así `payment + extra == interest + principal_repaid` es exacto por construcción y
            // no una coincidencia numérica que un cambio de modelo pueda romper.
            let repaid_by_payment = principal - closing_after_payment;
            let interest_accrued = payment - repaid_by_payment;
            let principal_repaid = principal - closing;

            months.push(LiabilityScheduleMonth {
                month_index: k,
                opening_principal: principal,
                interest_accrued,
                principal_repaid,
                extra_principal: extra,
                payment,
                closing_principal: closing,
            });
            total_interest += interest_accrued;
            total_payments += payment;
            total_extra_principal += extra;
            principal = closing;

            if principal.is_zero() {
                payoff_month_index = Some(k);
                break;
            }
        }
    }

    let payoff_absent = if payoff_month_index.is_some() {
        None
    } else if months.is_empty() {
        Some(LiabilityPayoffAbsence::NoPaymentPlan)
    } else if plan_ended {
        Some(LiabilityPayoffAbsence::PaymentPlanEndsBeforePayoff)
    } else if principal >= opening_principal {
        Some(LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal)
    } else {
        Some(LiabilityPayoffAbsence::NotWithinHorizon)
    };

    LiabilitySchedule {
        months,
        opening_principal,
        final_principal: principal,
        total_interest,
        total_payments,
        total_extra_principal,
        total_cash_out: total_payments + total_extra_principal,
        payoff_month_index,
        payoff_absent,
        horizon_months: horizon,
    }
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
        // Mismos helpers que el bucle de simulación; el principal de cierre se descarta aquí
        // porque esta función solo resuelve el mes 1 — pero la amortización extra SÍ entra en el
        // servicio de deuda, que es lo que decide cuánto sobrante llega a la cascada.
        let active = liability_active(liab, m_start);
        let opening = principals.get(i).copied().unwrap_or(Decimal::ZERO);
        let (cash, closing) = liability_month(
            liab.repayment_model,
            opening,
            liab.monthly_payment,
            liab.apr_percent,
            active,
        );
        debt_service += cash + liability_extra_principal(liab, 1, closing, active);
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

        // Servicio de deuda del mes. Desde 4.2.0 la recurrencia del pasivo se resuelve **una
        // sola vez** por mes: aquí salen a la vez la caja que se paga y el principal de cierre,
        // que se guarda y se aplica en el paso de amortización más abajo. Antes eran dos
        // recorridos que recalculaban el mismo `min(cuota, principal)`; con intereses de por
        // medio recalcularlo sería recalcular el devengo, y una de las dos copias acabaría
        // divergiendo. El orden de los pasos del mes NO cambia (servicio de deuda → caja →
        // crecimiento de activos → amortización → NW) y nada muta `principals` entre este punto
        // y aquel.
        let mut debt_service = Decimal::ZERO;
        let mut closing_principals: Vec<Decimal> = Vec::with_capacity(principals.len());
        for (i, liab) in input.liabilities.iter().enumerate() {
            if i >= principals.len() {
                break;
            }
            let active = liability_active(liab, m_start);
            let (cash, closing) = liability_month(
                liab.repayment_model,
                principals[i],
                liab.monthly_payment,
                liab.apr_percent,
                active,
            );
            // Amortización extra (what-if): sale de la caja del mes como servicio de deuda Y baja
            // el principal el mismo importe. Las dos cosas o ninguna — hacer solo la primera
            // drenaría caja sin reducir deuda, y solo la segunda imprimiría dinero.
            let extra = liability_extra_principal(liab, k, closing, active);
            debt_service += cash + extra;
            closing_principals.push(closing - extra);
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

        // Amortización: solo se asienta el cierre ya calculado arriba. Sin recomputar nada.
        for (i, closing) in closing_principals.iter().enumerate() {
            principals[i] = *closing;
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
            repayment_model: RepaymentModel::FixedPayments,
            apr_percent: None,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
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
            // El pin es del modelo histórico: sin intereses. La reforma 4.2.0 añade los campos
            // pero no toca sus valores esperados; la 4.4.0 añade los de amortización extra
            // (a cero: el pin describe un pasivo real, no un what-if).
            repayment_model: RepaymentModel::FixedPayments,
            apr_percent: None,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
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

    // -----------------------------------------------------------------------
    // 4.2.0 — intereses de los pasivos (`RepaymentModel` + `apr_percent`)
    // -----------------------------------------------------------------------

    /// Input mínimo para leer el principal vivo de UN pasivo mes a mes.
    ///
    /// Un solo activo sin rentabilidad y una regla sumidero: así `surplus_cash` es 0 (el
    /// sumidero absorbe la caja entera), el activo no compone y el patrimonio se reduce a
    /// `activo − principal`. Es la misma técnica del pin pre-4.2.0, que evita exponer los
    /// principales en la API pública solo para poder testearlos.
    fn one_liability_input(
        horizon: u32,
        principal: Decimal,
        payment: Decimal,
        model: RepaymentModel,
        apr: Option<Decimal>,
    ) -> ProjectionInput {
        let mut inp = base_input(
            horizon,
            Decimal::from(4000),
            Decimal::from(1500),
            vec![mk_asset(0xC3, Decimal::ZERO, true, None)],
            vec![rule_remainder(0)],
        );
        inp.liabilities = vec![ProjectionLiabilityInput {
            principal,
            monthly_payment: payment,
            payment_end: None,
            repayment_model: model,
            apr_percent: apr,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
        }];
        inp
    }

    /// Principal vivo implícito en el mes `k`: `Σ activos − net_worth`. Exacto con el input de
    /// [`one_liability_input`].
    fn implicit_principal(out: &ProjectionOutput, k: usize) -> Decimal {
        out.per_asset_series.iter().map(|s| s[k]).sum::<Decimal>() - out.net_worth[k]
    }

    /// El TIN NO se cobra en `fixed_payments`: el modelo por defecto ignora `apr_percent` por
    /// completo. Se comprueba sobre el pin pre-4.2.0 —el input más rico que hay— añadiéndole un
    /// 3 % anual: la serie debe salir idéntica, punto por punto.
    ///
    /// Es la garantía de que el TIN es un dato **inerte** hasta que el usuario cambia de modelo:
    /// alguien puede rellenar el interés de su hipoteca sin tocar el modelo, y sus números no se
    /// pueden mover por eso.
    #[test]
    fn fixed_payments_with_apr_is_bit_identical_to_the_pre_4_2_0_pin() {
        let base = liability_pin_input();
        let mut con_apr = liability_pin_input();
        con_apr.liabilities[0].apr_percent = Some(Decimal::from(3));

        let a = project_net_worth_series(&base).unwrap();
        let b = project_net_worth_series(&con_apr).unwrap();
        assert_eq!(a.net_worth, b.net_worth, "el TIN no debe mover fixed_payments");
        assert_eq!(a.contributed_capital, b.contributed_capital);
        assert_eq!(a.per_asset_series, b.per_asset_series);
    }

    /// Sistema francés, cuatro meses a mano. P = 100.000, TIN 3 % ⇒ i = 3/1200 = 0,0025 exacto,
    /// cuota 500 a fin de mes. `P' = P·1,0025 − 500`:
    ///
    /// | mes | payoff              | cierre                |
    /// |-----|---------------------|-----------------------|
    /// | 1   | 100.250             | 99.750                |
    /// | 2   | 99.999,375          | 99.499,375            |
    /// | 3   | 99.748,1234375      | 99.248,1234375        |
    /// | 4   | 99.496,24374609375  | 98.996,24374609375    |
    ///
    /// Todo es exacto en `Decimal` (0,0025 tiene representación finita), así que `assert_eq!` sin
    /// tolerancia: no hay `powd` por medio, solo multiplicaciones.
    #[test]
    fn french_two_months_hand_checked() {
        let inp = one_liability_input(
            4,
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(implicit_principal(&out, 0), Decimal::from(100_000));
        assert_eq!(implicit_principal(&out, 1), dec("99750.00"));
        assert_eq!(implicit_principal(&out, 2), dec("99499.375"));
        assert_eq!(implicit_principal(&out, 3), dec("99248.1234375"));
        assert_eq!(implicit_principal(&out, 4), dec("98996.24374609375"));
    }

    /// Extinción real del préstamo francés: 100.000 € al 3 % con 500 €/mes no se acaban en 200
    /// meses (lo que costaría sin intereses) sino en **278**. Los 78 meses de diferencia son la
    /// razón de ser de la reforma: el modelo pre-4.2.0 declaraba libre de deuda a un hogar que
    /// todavía debe seis años y medio de cuotas.
    ///
    /// El último mes es de **cuota parcial**: el saldo pendiente ya no llega a 500 €, así que la
    /// caja que sale es solo lo que queda por pagar (`payoff` del mes 277).
    #[test]
    fn french_extinction_at_month_278() {
        let inp = one_liability_input(
            280,
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let out = project_net_worth_series(&inp).unwrap();

        let p277 = implicit_principal(&out, 277);
        assert!(
            (p277 - Decimal::from(302)).abs() < Decimal::ONE,
            "el mes 277 debe dejar ≈ 302 € vivos, obtenido {p277}"
        );
        assert_eq!(
            implicit_principal(&out, 278),
            Decimal::ZERO,
            "el pasivo se extingue en el mes 278"
        );
        assert_eq!(implicit_principal(&out, 279), Decimal::ZERO, "y no resucita");

        // Caja del mes 278 = payoff del saldo anterior − cierre = cuota PARCIAL, < 500.
        let cash_278 = p277 * (Decimal::ONE + Decimal::from(3) / Decimal::from(1200))
            - implicit_principal(&out, 278);
        assert!(
            cash_278 < Decimal::from(500) && cash_278 > Decimal::ZERO,
            "la última cuota es parcial, obtenida {cash_278}"
        );
    }

    /// Cuota por debajo del interés: la deuda **crece**. P = 100.000 al 12 % ⇒ i = 0,01; el
    /// devengo es 1.000 €/mes y la cuota 500. Cierres: 100.500 y 101.005 (= 100.500·1,01 − 500).
    ///
    /// El modelo pre-4.2.0 no podía ni representar esto: amortizaba 500 € al mes de un préstamo
    /// que en realidad se hace más grande cada mes.
    #[test]
    fn french_payment_below_interest_grows_the_principal() {
        let inp = one_liability_input(
            2,
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(12)),
        );
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(implicit_principal(&out, 1), dec("100500.00"));
        assert_eq!(implicit_principal(&out, 2), dec("101005.0000"));
    }

    /// Solo intereses: el principal no se mueve NUNCA y la caja que sale es la cuota entera —
    /// que es justo lo que declara el usuario de un préstamo de este tipo. El TIN es informativo
    /// aquí: la cuota YA es el interés, devengarlo otra vez lo cobraría dos veces.
    #[test]
    fn interest_only_principal_constant_and_cash_is_the_quota() {
        let inp = one_liability_input(
            24,
            Decimal::from(80_000),
            Decimal::from(400),
            RepaymentModel::InterestOnly,
            Some(Decimal::from(6)),
        );
        let out = project_net_worth_series(&inp).unwrap();
        for k in 0..=24 {
            assert_eq!(
                implicit_principal(&out, k),
                Decimal::from(80_000),
                "el principal de un interest_only es constante (mes {k})"
            );
        }
        assert_eq!(
            first_month_allocation(&inp).unwrap().debt_service,
            Decimal::from(400),
            "la caja del mes es la cuota íntegra"
        );

        // Con la cuota por encima del saldo, la caja se recorta al saldo (`min`): nadie paga
        // 500 € por un préstamo del que solo debe 300.
        let pequeno = one_liability_input(
            1,
            Decimal::from(300),
            Decimal::from(500),
            RepaymentModel::InterestOnly,
            Some(Decimal::from(6)),
        );
        assert_eq!(
            first_month_allocation(&pequeno).unwrap().debt_service,
            Decimal::from(300)
        );
    }

    /// `Revolving` y `French` comparten recurrencia en 4.2.0 — **deliberadamente**. Este test
    /// pinea esa equivalencia para que no se rompa por accidente: el día que revolving modele lo
    /// suyo (disposiciones, cuota mínima como % del saldo) este test se cambia A PROPÓSITO, con
    /// su entrada de CHANGELOG. Mientras tanto, dos etiquetas para la misma matemática es una
    /// decisión de producto (el usuario nombra su deuda como es), no un descuido.
    #[test]
    fn revolving_matches_french_recurrence() {
        let frances = one_liability_input(
            120,
            Decimal::from(12_000),
            Decimal::from(250),
            RepaymentModel::French,
            Some(Decimal::from(18)),
        );
        let revolving = one_liability_input(
            120,
            Decimal::from(12_000),
            Decimal::from(250),
            RepaymentModel::Revolving,
            Some(Decimal::from(18)),
        );
        let a = project_net_worth_series(&frances).unwrap();
        let b = project_net_worth_series(&revolving).unwrap();
        assert_eq!(a.net_worth, b.net_worth);
        assert_eq!(a.contributed_capital, b.contributed_capital);
        assert_eq!(a.per_asset_series, b.per_asset_series);
    }

    /// `payment_end` congela el pasivo en los CUATRO modelos: desde el mes siguiente no sale
    /// caja, no se amortiza y —lo nuevo en 4.2.0— tampoco se devenga interés. Un plan de pago
    /// terminado con principal vivo es una resta constante al patrimonio, no una bola de nieve.
    ///
    /// `ref_date` = 2026-01-15 ⇒ el mes `k` abre el 2026-01-01 + (k−1) meses. Con
    /// `payment_end = 2026-03-15` el último mes activo es el **3**.
    #[test]
    fn payment_end_freezes_every_model() {
        for model in [
            RepaymentModel::FixedPayments,
            RepaymentModel::French,
            RepaymentModel::InterestOnly,
            RepaymentModel::Revolving,
        ] {
            let mut inp = one_liability_input(
                8,
                Decimal::from(100_000),
                Decimal::from(500),
                model,
                Some(Decimal::from(6)),
            );
            inp.liabilities[0].payment_end = Some(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
            let out = project_net_worth_series(&inp).unwrap();

            let congelado = implicit_principal(&out, 3);
            assert!(
                congelado > Decimal::ZERO,
                "{model:?}: el principal debe seguir vivo al terminar el plan"
            );
            for k in 4..=8 {
                assert_eq!(
                    implicit_principal(&out, k),
                    congelado,
                    "{model:?}: sin plan activo el principal no se mueve (mes {k})"
                );
                // Sin caja: el patrimonio sube exactamente el neto del mes (4000 − 1500).
                assert_eq!(
                    out.net_worth[k] - out.net_worth[k - 1],
                    Decimal::from(2500),
                    "{model:?}: sin plan activo no sale caja (mes {k})"
                );
            }
        }
    }

    /// Cuota 0 ⇒ el pasivo no tiene plan activo ⇒ **no devenga**, ni siquiera con TIN. Es el
    /// contrato que explotan los modos B/C del handler (`savings_source.uses_transactions()`):
    /// ponen `monthly_payment = 0` en memoria para que el principal sea una resta constante,
    /// porque las cuotas pagadas ya viven dentro del promedio de gasto real. Si un pasivo sin
    /// plan devengara intereses, esos hogares verían crecer una deuda que ya están pagando.
    #[test]
    fn zero_payment_liability_never_accrues_interest() {
        let inp = one_liability_input(
            36,
            Decimal::from(50_000),
            Decimal::ZERO,
            RepaymentModel::French,
            Some(Decimal::from(5)),
        );
        let out = project_net_worth_series(&inp).unwrap();
        for k in 0..=36 {
            assert_eq!(
                implicit_principal(&out, k),
                Decimal::from(50_000),
                "sin plan de pago el principal es constante (mes {k})"
            );
        }
        assert_eq!(
            first_month_allocation(&inp).unwrap().debt_service,
            Decimal::ZERO
        );
    }

    /// El tope de la cuota es el **payoff**, no el principal: cancelar cuesta el saldo CON el
    /// interés del mes. P = 400 al 3 % ⇒ payoff = 400 · 1,0025 = 401,00 exacto; con cuota de
    /// 500 la caja del mes es 401,00 y el pasivo queda a cero.
    ///
    /// Cambio de comportamiento consciente respecto a 4.1.0, donde el tope era el principal (400)
    /// — pero solo aplica a los modelos que devengan; `fixed_payments` sigue topando en 400.
    #[test]
    fn first_month_allocation_debt_service_caps_at_payoff_with_apr() {
        let inp = one_liability_input(
            1,
            Decimal::from(400),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        assert_eq!(
            first_month_allocation(&inp).unwrap().debt_service,
            dec("401.00")
        );

        let historico = one_liability_input(
            1,
            Decimal::from(400),
            Decimal::from(500),
            RepaymentModel::FixedPayments,
            Some(Decimal::from(3)),
        );
        assert_eq!(
            first_month_allocation(&historico).unwrap().debt_service,
            Decimal::from(400),
            "fixed_payments sigue topando en el principal"
        );
    }

    /// TIN absurdo (1.000 %) sobre el horizonte máximo (840 meses): el payoff desborda `Decimal`
    /// en pocos meses y el `checked_mul` **satura** en vez de panicar. La simulación termina y
    /// devuelve una serie completa. Nadie configura esto a mano, pero un `.ffbackup` importado o
    /// un dedo torcido en el formulario no pueden tumbar el proceso.
    #[test]
    fn high_apr_never_panics() {
        let inp = one_liability_input(
            840,
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(1000)),
        );
        let out = project_net_worth_series(&inp).expect("la simulación debe terminar");
        assert_eq!(out.net_worth.len(), 841);
        // Saturado: el principal deja de crecer y la cuota lo va royendo 500 €/mes.
        let p1 = implicit_principal(&out, 839);
        let p2 = implicit_principal(&out, 840);
        assert_eq!(p1 - p2, Decimal::from(500), "saturado, la cuota va a principal");
    }

    // -----------------------------------------------------------------------
    // 4.4.0 — calendario de amortización (`liability_amortization_schedule`)
    // -----------------------------------------------------------------------

    fn liab(
        principal: Decimal,
        payment: Decimal,
        model: RepaymentModel,
        apr: Option<Decimal>,
    ) -> ProjectionLiabilityInput {
        ProjectionLiabilityInput {
            principal,
            monthly_payment: payment,
            payment_end: None,
            repayment_model: model,
            apr_percent: apr,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
        }
    }

    fn ref_2026() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()
    }

    /// **La identidad contable del calendario**, exacta en `Decimal`, en los cuatro modelos y en
    /// todos los meses: `payment + extra_principal == interest_accrued + principal_repaid`.
    ///
    /// No es decorativa: es la única garantía de que el desglose que se publica describe la
    /// recurrencia que el motor ejecuta de verdad. Si alguien devengara el interés por su cuenta
    /// (`P·i` recalculado) en vez de derivarlo de los saldos, esta igualdad se rompería en el mes
    /// en que las dos implementaciones se separaran — que es exactamente el aviso que se quiere.
    #[test]
    fn schedule_payment_identity_holds_in_every_model() {
        for model in [
            RepaymentModel::FixedPayments,
            RepaymentModel::French,
            RepaymentModel::InterestOnly,
            RepaymentModel::Revolving,
        ] {
            for apr in [None, Some(Decimal::from(6))] {
                let mut l = liab(Decimal::from(30_000), Decimal::from(400), model, apr);
                l.extra_principal_monthly = Decimal::from(50);
                l.extra_principal_lump_sums = vec![(7, Decimal::from(1_000))];
                let sch = liability_amortization_schedule(&l, ref_2026(), 120);
                assert!(!sch.months.is_empty(), "{model:?}/{apr:?}: calendario vacío");
                for m in &sch.months {
                    assert_eq!(
                        m.payment + m.extra_principal,
                        m.interest_accrued + m.principal_repaid,
                        "{model:?}/{apr:?}: identidad rota en el mes {}",
                        m.month_index
                    );
                    assert_eq!(
                        m.closing_principal,
                        m.opening_principal - m.principal_repaid,
                        "{model:?}/{apr:?}: saldos incoherentes en el mes {}",
                        m.month_index
                    );
                    assert!(
                        m.closing_principal >= Decimal::ZERO,
                        "{model:?}/{apr:?}: principal negativo en el mes {}",
                        m.month_index
                    );
                }
                // Invariante de la ausencia: o hay mes de extinción, o hay razón. Nunca las dos,
                // nunca ninguna.
                assert_eq!(
                    sch.payoff_month_index.is_some(),
                    sch.payoff_absent.is_none(),
                    "{model:?}/{apr:?}: payoff y razón deben excluirse"
                );
            }
        }
    }

    /// **Amortización francesa contra su fórmula cerrada.** P = 100.000 €, TIN 3 % ⇒ i = 0,0025
    /// exacto, n = 240. La cuota teórica es
    ///
    /// `M = P·i / (1 − (1+i)^−n) = 100.000·0,0025 / (1 − 1,0025^−240) = 554,597597853912… €`
    ///
    /// verificada a 60 dígitos con aritmética decimal exacta. Redondeada como la redondearía un
    /// banco, **554,60 €**, la predicción es: el préstamo se extingue **exactamente en el mes
    /// 240**, el mes 239 deja **552,4302949… €** vivos y la última cuota es **parcial**
    /// (553,8113706… €, ocho céntimos menos que la cuota redondeada porque redondear hacia arriba
    /// amortiza un pelín más rápido). Total pagado **133.103,2114 €**, interés total
    /// **33.103,2114 €**.
    ///
    /// El segundo contraste es cruzado y no depende de esta predicción: `present_value_of_payments`
    /// —la otra implementación de la misma matemática que ya vive en el crate, la que usa
    /// `liabilities.rs` para derivar el principal— tiene que devolver los 100.000 € de partida
    /// desde esa misma cuota y ese mismo plazo.
    #[test]
    fn schedule_french_matches_the_closed_form_annuity() {
        let m = dec("554.60");
        let l = liab(
            Decimal::from(100_000),
            m,
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let sch = liability_amortization_schedule(&l, ref_2026(), 840);

        assert_eq!(
            sch.payoff_month_index,
            Some(240),
            "la cuota de la fórmula cerrada salda el préstamo en su plazo"
        );
        assert_eq!(sch.payoff_absent, None);
        assert_eq!(sch.months.len(), 240);
        assert_eq!(sch.final_principal, Decimal::ZERO);

        // Mes 1, exacto: interés = 100.000 · 0,0025 = 250; principal = 554,60 − 250 = 304,60.
        let m1 = &sch.months[0];
        assert_eq!(m1.opening_principal, Decimal::from(100_000));
        assert_eq!(m1.interest_accrued, dec("250.0000"));
        assert_eq!(m1.principal_repaid, dec("304.6000"));
        assert_eq!(m1.closing_principal, dec("99695.4000"));

        // Saldo al cerrar el mes 239 y última cuota, parcial.
        let m239 = &sch.months[238];
        assert!(
            (m239.closing_principal - dec("552.4302949022704995")).abs() < dec("0.0001"),
            "saldo tras 239 meses esperado ≈ 552,4303 €, obtenido {}",
            m239.closing_principal
        );
        let last = sch.months.last().unwrap();
        assert!(
            last.payment < m && last.payment > Decimal::ZERO,
            "la última cuota es parcial, obtenida {}",
            last.payment
        );
        assert!(
            (last.payment - dec("553.8113706395")).abs() < dec("0.0001"),
            "última cuota esperada ≈ 553,8114 €, obtenida {}",
            last.payment
        );

        // Agregados: la respuesta a «¿cuánto pago de intereses?» y «¿cuánto pago en total?».
        assert!(
            (sch.total_cash_out - dec("133103.2113706395")).abs() < dec("0.01"),
            "total a pagar esperado ≈ 133.103,21 €, obtenido {}",
            sch.total_cash_out
        );
        assert!(
            (sch.total_interest - dec("33103.2113706395")).abs() < dec("0.01"),
            "interés total esperado ≈ 33.103,21 €, obtenido {}",
            sch.total_interest
        );
        // Telescopio exacto: Σ interés = Σ caja − (saldo inicial − saldo final).
        assert!(
            (sch.total_interest
                - (sch.total_cash_out - sch.opening_principal + sch.final_principal))
                .abs()
                < dec("0.0001"),
            "Σ interés debe cuadrar con Σ caja − principal amortizado"
        );

        // Contraste cruzado con la OTRA implementación de la misma matemática que ya vive en el
        // crate: el valor actual de 240 cuotas de 554,60 al 3 % son los 100.000 de partida.
        let pv = present_value_of_payments(m, Decimal::from(240), Some(Decimal::from(3)));
        assert!(
            (pv - Decimal::from(100_000)).abs() < Decimal::ONE,
            "PV(554,60; 240; 3 %) esperado ≈ 100.000 €, obtenido {pv}"
        );
    }

    /// **Once meses a mano, con aritmética exacta.** P = 1.000 €, TIN 12 % ⇒ i = 12/1200 = 0,01
    /// **exacto** en `Decimal` (no hay `powd` por medio, solo multiplicaciones), cuota 100 €.
    ///
    /// | mes | interés | principal | cierre |
    /// |----:|--------:|----------:|-------:|
    /// | 1 | 10,00 | 90,00 | 910,00 |
    /// | 2 | 9,10 | 90,90 | 819,10 |
    /// | 3 | 8,191 | 91,809 | 727,291 |
    /// | 11 | 0,58400871299… | 58,40087129915940991 | 0 |
    ///
    /// Extinción en el **mes 11** con cuota parcial de **58,984880012151004009 €**; interés total
    /// **58,984880012151004009 €** (que aquí coincide con la última cuota por casualidad
    /// aritmética: el interés total es `total pagado − 1.000`). `assert_eq!` sin tolerancia.
    #[test]
    fn schedule_hand_checked_eleven_months_exact() {
        let l = liab(
            Decimal::from(1_000),
            Decimal::from(100),
            RepaymentModel::French,
            Some(Decimal::from(12)),
        );
        let sch = liability_amortization_schedule(&l, ref_2026(), 60);

        assert_eq!(sch.payoff_month_index, Some(11));
        assert_eq!(sch.months.len(), 11);
        assert_eq!(sch.months[0].interest_accrued, dec("10.00"));
        assert_eq!(sch.months[0].principal_repaid, dec("90.00"));
        assert_eq!(sch.months[0].closing_principal, dec("910.00"));
        assert_eq!(sch.months[1].interest_accrued, dec("9.1000"));
        assert_eq!(sch.months[1].closing_principal, dec("819.1000"));
        assert_eq!(sch.months[2].closing_principal, dec("727.291000"));
        assert_eq!(
            sch.months[9].closing_principal,
            dec("58.40087129915940991000")
        );
        assert_eq!(
            sch.months[10].payment,
            dec("58.9848800121510040091000")
        );
        assert_eq!(sch.months[10].closing_principal, Decimal::ZERO);
        assert_eq!(
            sch.total_interest,
            dec("58.9848800121510040091000"),
            "interés total = total pagado − principal"
        );
        assert_eq!(sch.total_cash_out, dec("1058.9848800121510040091000"));
        assert_eq!(sch.total_extra_principal, Decimal::ZERO);
    }

    /// **El calendario y la simulación cuentan la misma historia.** Mismo input que
    /// `french_extinction_at_month_278` (100.000 € al 3 % con 500 €/mes): el mes de extinción que
    /// devuelve el calendario y el que se deduce del `ProjectionOutput` completo tienen que ser el
    /// MISMO número —278—, y el saldo de cierre de cada mes tiene que coincidir mes a mes.
    ///
    /// Es la prueba de que «cero matemática nueva» es literal: el calendario publica el principal
    /// de cierre que el bucle ya calculaba y tiraba, no una segunda derivación de él.
    #[test]
    fn schedule_agrees_month_by_month_with_the_projection_loop() {
        let inp = one_liability_input(
            300,
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let out = project_net_worth_series(&inp).unwrap();
        let sch = liability_amortization_schedule(&inp.liabilities[0], inp.ref_date, 300);

        assert_eq!(sch.payoff_month_index, Some(278));
        for m in &sch.months {
            // Tolerancia de 1e−6 € y no `assert_eq!` por una razón que NO es el motor: el helper
            // `implicit_principal` deduce el principal restándoselo a un patrimonio de seis
            // cifras (`Σ activos − net_worth`), y esa resta pierde los últimos dígitos de la
            // mantisa de 28 de `Decimal`. La recurrencia es literalmente la misma llamada a
            // `liability_month`; quien redondea es el test. Medido: la mayor discrepancia sobre
            // los 278 meses está en el orden de 1e−24 €.
            let del_bucle = implicit_principal(&out, m.month_index as usize);
            assert!(
                (m.closing_principal - del_bucle).abs() < dec("0.000001"),
                "el saldo del mes {} debe ser el mismo en el calendario ({}) y en la simulación ({})",
                m.month_index,
                m.closing_principal,
                del_bucle
            );
        }
        // El mes de extinción sí es exacto: el saldo llega a CERO en los dos sitios.
        assert_eq!(implicit_principal(&out, 278), Decimal::ZERO);
        assert!(
            (sch.total_interest - dec("38802.7999")).abs() < dec("0.01"),
            "interés total esperado ≈ 38.802,80 €, obtenido {}",
            sch.total_interest
        );
        assert!(
            (sch.total_cash_out - dec("138802.7999")).abs() < dec("0.01"),
            "total a pagar esperado ≈ 138.802,80 €, obtenido {}",
            sch.total_cash_out
        );
    }

    /// `fixed_payments` —el default de la columna— no devenga: todos los meses tienen interés
    /// **exactamente 0** y el total a pagar es el principal, con TIN o sin él. El calendario de un
    /// pasivo histórico sigue siendo el de siempre.
    #[test]
    fn schedule_fixed_payments_never_charges_interest() {
        for apr in [None, Some(Decimal::from(9))] {
            let l = liab(
                Decimal::from(1_200),
                Decimal::from(100),
                RepaymentModel::FixedPayments,
                apr,
            );
            let sch = liability_amortization_schedule(&l, ref_2026(), 60);
            assert_eq!(sch.payoff_month_index, Some(12), "{apr:?}");
            assert_eq!(sch.total_interest, Decimal::ZERO, "{apr:?}");
            assert_eq!(sch.total_cash_out, Decimal::from(1_200), "{apr:?}");
            for m in &sch.months {
                assert_eq!(m.interest_accrued, Decimal::ZERO);
                assert_eq!(m.payment, Decimal::from(100));
            }
        }
    }

    /// Las cuatro razones por las que no hay mes de extinción, cada una con su remedio distinto.
    #[test]
    fn schedule_payoff_absent_reasons_are_distinguishable() {
        // (1) Sin plan de pago: cuota 0 ⇒ calendario vacío, ni devengo ni amortización.
        let sin_plan = liab(
            Decimal::from(50_000),
            Decimal::ZERO,
            RepaymentModel::French,
            Some(Decimal::from(5)),
        );
        let s1 = liability_amortization_schedule(&sin_plan, ref_2026(), 120);
        assert_eq!(s1.payoff_absent, Some(LiabilityPayoffAbsence::NoPaymentPlan));
        assert!(s1.months.is_empty());
        assert_eq!(s1.final_principal, Decimal::from(50_000));
        assert_eq!(s1.total_cash_out, Decimal::ZERO);

        // (2) El plan termina antes: `ref_date` = 2026-01-15 ⇒ el mes k abre el 2026-01-01 + (k−1);
        // con `payment_end` = 2026-03-15 el último mes con cuota es el 3.
        let mut corto = liab(
            Decimal::from(50_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(5)),
        );
        corto.payment_end = Some(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap());
        let s2 = liability_amortization_schedule(&corto, ref_2026(), 120);
        assert_eq!(
            s2.payoff_absent,
            Some(LiabilityPayoffAbsence::PaymentPlanEndsBeforePayoff)
        );
        assert_eq!(s2.months.len(), 3, "solo tres meses con cuota");
        assert!(s2.final_principal > Decimal::ZERO);

        // (3) La cuota no reduce el principal: `interest_only` (congelado) y cuota por debajo del
        // devengo (la deuda crece) comparten razón porque comparten remedio: subir la cuota.
        let solo_interes = liab(
            Decimal::from(80_000),
            Decimal::from(400),
            RepaymentModel::InterestOnly,
            Some(Decimal::from(6)),
        );
        let s3 = liability_amortization_schedule(&solo_interes, ref_2026(), 120);
        assert_eq!(
            s3.payoff_absent,
            Some(LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal)
        );
        assert_eq!(s3.final_principal, Decimal::from(80_000));
        assert_eq!(
            s3.total_interest, s3.total_cash_out,
            "en interest_only todo lo que se paga es interés"
        );

        let bola_de_nieve = liab(
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::Revolving,
            Some(Decimal::from(12)),
        );
        let s3b = liability_amortization_schedule(&bola_de_nieve, ref_2026(), 120);
        assert_eq!(
            s3b.payoff_absent,
            Some(LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal)
        );
        assert!(s3b.final_principal > Decimal::from(100_000), "la deuda crece");

        // (4) Baja, pero no llega a cero dentro de los meses pedidos.
        let largo = liab(
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let s4 = liability_amortization_schedule(&largo, ref_2026(), 100);
        assert_eq!(
            s4.payoff_absent,
            Some(LiabilityPayoffAbsence::NotWithinHorizon)
        );
        assert_eq!(s4.months.len(), 100);

        // (5) Ya saldado hoy: mes 0, sin razón y sin meses.
        let saldado = liab(
            Decimal::ZERO,
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let s5 = liability_amortization_schedule(&saldado, ref_2026(), 120);
        assert_eq!(s5.payoff_month_index, Some(0));
        assert_eq!(s5.payoff_absent, None);
        assert!(s5.months.is_empty());
    }

    // -----------------------------------------------------------------------
    // 4.4.0 — amortización extra (ejes what-if de `simulate_projection`)
    // -----------------------------------------------------------------------

    /// **Amortizar antes no imprime dinero.** Con `fixed_payments` —que no devenga interés—
    /// adelantar la amortización no puede mejorar el patrimonio en NINGÚN mes: lo que sale de más
    /// por la caja entra igual de principal amortizado, y como no hay interés que evitar, el
    /// beneficio futuro es exactamente cero.
    ///
    /// Es el test que caza el error más caro de este eje: reducir el principal sin cobrar la caja
    /// (o cobrarla sin reducir el principal). Cualquiera de los dos movería esta serie.
    #[test]
    fn extra_principal_is_net_worth_neutral_without_interest() {
        let base = one_liability_input(
            15,
            Decimal::from(1_000),
            Decimal::from(100),
            RepaymentModel::FixedPayments,
            None,
        );
        let mut con_extra = base.clone();
        con_extra.liabilities[0].extra_principal_monthly = Decimal::from(100);

        let a = project_net_worth_series(&base).unwrap();
        let b = project_net_worth_series(&con_extra).unwrap();
        assert_eq!(
            a.net_worth, b.net_worth,
            "sin intereses, amortizar antes es un intercambio de balance: mismo patrimonio"
        );

        // Y la deuda sí desaparece antes: 200 €/mes la liquidan en 5 meses en vez de 10.
        assert_eq!(implicit_principal(&b, 5), Decimal::ZERO);
        assert_eq!(implicit_principal(&a, 5), Decimal::from(500));
    }

    /// **La cuota liberada vuelve a la cascada — y no por una decisión nueva.**
    ///
    /// Cuando el principal llega a 0, `liability_month` devuelve `cash = min(M, 0) = 0`: la cuota
    /// deja de salir de la caja sola, el sobrante del mes sube en ese importe y la cascada lo
    /// encamina como cualquier otro euro. Es el comportamiento que el motor YA tenía para un
    /// préstamo que se acaba antes de su `payment_end`; el eje de amortización extra solo hace
    /// que ese momento llegue antes. Suprimir ese dinero exigiría añadir maquinaria para esconder
    /// caja que el modelo tiene — y respondería a una pregunta que nadie hace.
    ///
    /// Aquí: cuota 100, extra 100, deuda 1.000 ⇒ liquidada en el mes 5. Desde el mes 6 el activo
    /// crece 2.500 €/mes (4.000 − 1.500) en vez de 2.300, y el activo es el destino del sumidero.
    #[test]
    fn extra_principal_frees_the_quota_into_the_cascade() {
        let mut inp = one_liability_input(
            8,
            Decimal::from(1_000),
            Decimal::from(100),
            RepaymentModel::FixedPayments,
            None,
        );
        inp.liabilities[0].extra_principal_monthly = Decimal::from(100);
        let out = project_net_worth_series(&inp).unwrap();
        let asset = &out.per_asset_series[0];

        for k in 1..=5usize {
            assert_eq!(
                asset[k] - asset[k - 1],
                Decimal::from(2_300),
                "con deuda viva salen 200 €/mes (cuota + extra) — mes {k}"
            );
        }
        for k in 6..=8usize {
            assert_eq!(
                asset[k] - asset[k - 1],
                Decimal::from(2_500),
                "liquidada la deuda, la cuota liberada entra en la cascada — mes {k}"
            );
        }
    }

    /// **Con intereses sí compensa, y por cuánto.** 100.000 € al 3 % con 500 €/mes se extinguen en
    /// el mes 278 y cuestan 138.802,7999 € de caja; con 100 €/mes de amortización extra se
    /// extinguen en el **216** y cuestan **129.520,8776 €**. La diferencia, **9.281,9223 €**, es
    /// interés que no se devenga — y tiene que aparecer, exacta, como más patrimonio al final del
    /// horizonte (el activo no compone, así que cada euro que no sale de la caja se queda en él).
    ///
    /// Números verificados a 60 dígitos con aritmética decimal exacta antes de correr el test.
    #[test]
    fn extra_principal_saves_exactly_the_interest_not_accrued() {
        let base = one_liability_input(
            300,
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let mut con_extra = base.clone();
        con_extra.liabilities[0].extra_principal_monthly = Decimal::from(100);

        let sch_base = liability_amortization_schedule(&base.liabilities[0], base.ref_date, 300);
        let sch_extra =
            liability_amortization_schedule(&con_extra.liabilities[0], base.ref_date, 300);
        assert_eq!(sch_base.payoff_month_index, Some(278));
        assert_eq!(sch_extra.payoff_month_index, Some(216));
        let ahorro = sch_base.total_cash_out - sch_extra.total_cash_out;
        assert!(
            (ahorro - dec("9281.9223")).abs() < dec("0.01"),
            "ahorro esperado ≈ 9.281,92 €, obtenido {ahorro}"
        );

        let a = project_net_worth_series(&base).unwrap();
        let b = project_net_worth_series(&con_extra).unwrap();
        let delta_nw = b.net_worth[300] - a.net_worth[300];
        assert!(
            (delta_nw - ahorro).abs() < dec("0.01"),
            "el patrimonio final debe mejorar exactamente en la caja no desembolsada: \
             ahorro {ahorro}, delta patrimonio {delta_nw}"
        );
    }

    /// Amortización puntual: 20.000 € en el mes 13 sobre la misma hipoteca la extinguen en el
    /// **207** (en vez del 278) con **123.361,6899 €** de caja. Y el importe se topa al saldo: un
    /// lump sum de 10 millones paga lo que se debe y ni un céntimo más.
    #[test]
    fn extra_principal_lump_sum_lands_on_its_month_and_caps_at_the_balance() {
        let base = liab(
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let mut lump = base.clone();
        lump.extra_principal_lump_sums = vec![(13, Decimal::from(20_000))];
        let sch = liability_amortization_schedule(&lump, ref_2026(), 300);
        assert_eq!(sch.payoff_month_index, Some(207));
        assert!(
            (sch.total_cash_out - dec("123361.6899")).abs() < dec("0.01"),
            "total a pagar esperado ≈ 123.361,69 €, obtenido {}",
            sch.total_cash_out
        );
        assert_eq!(sch.months[12].extra_principal, Decimal::from(20_000));
        assert_eq!(sch.months[11].extra_principal, Decimal::ZERO);

        let mut absurdo = base.clone();
        absurdo.extra_principal_lump_sums = vec![(2, Decimal::from(10_000_000))];
        let s = liability_amortization_schedule(&absurdo, ref_2026(), 300);
        assert_eq!(s.payoff_month_index, Some(2), "liquida en el mes 2");
        assert_eq!(s.final_principal, Decimal::ZERO);
        assert!(
            s.total_cash_out < Decimal::from(101_000),
            "el lump sum se topa al saldo, no se paga de más: {}",
            s.total_cash_out
        );
    }

    /// **Sin amortización extra, nada se mueve.** Los campos nuevos a cero reproducen el pin
    /// pre-4.4.0 punto por punto: quien no simule nada no puede notar que este eje existe.
    #[test]
    fn zero_extra_principal_is_bit_identical_to_the_pin() {
        let base = liability_pin_input();
        let mut explicito = liability_pin_input();
        explicito.liabilities[0].extra_principal_monthly = Decimal::ZERO;
        explicito.liabilities[0].extra_principal_lump_sums = vec![(3, Decimal::ZERO)];

        let a = project_net_worth_series(&base).unwrap();
        let b = project_net_worth_series(&explicito).unwrap();
        assert_eq!(a.net_worth, b.net_worth);
        assert_eq!(a.contributed_capital, b.contributed_capital);
        assert_eq!(a.per_asset_series, b.per_asset_series);
    }

    /// Sin plan de pago activo no hay amortización extra que valga: es el contrato que explotan
    /// los modos B/C del handler (`payment_amount = 0` en memoria, principal como resta
    /// constante). Si la amortización extra se colara ahí, un what-if movería el principal de un
    /// hogar cuyas cuotas ya están dentro del promedio de gasto — contándolas dos veces.
    #[test]
    fn extra_principal_needs_an_active_payment_plan() {
        let mut sin_cuota = liab(
            Decimal::from(50_000),
            Decimal::ZERO,
            RepaymentModel::French,
            Some(Decimal::from(5)),
        );
        sin_cuota.extra_principal_monthly = Decimal::from(1_000);
        let sch = liability_amortization_schedule(&sin_cuota, ref_2026(), 60);
        assert_eq!(sch.payoff_absent, Some(LiabilityPayoffAbsence::NoPaymentPlan));
        assert_eq!(sch.final_principal, Decimal::from(50_000));
        assert_eq!(sch.total_extra_principal, Decimal::ZERO);

        let mut inp = one_liability_input(
            12,
            Decimal::from(50_000),
            Decimal::ZERO,
            RepaymentModel::French,
            Some(Decimal::from(5)),
        );
        inp.liabilities[0].extra_principal_monthly = Decimal::from(1_000);
        let out = project_net_worth_series(&inp).unwrap();
        for k in 0..=12 {
            assert_eq!(
                implicit_principal(&out, k),
                Decimal::from(50_000),
                "sin plan de pago el principal es constante (mes {k})"
            );
        }
    }

    /// Valor actual de una renta. Tres contratos:
    /// 1. Sin TIN (o con TIN ≤ 0) es `M · n` **exacto**, sin pasar por `powd`.
    /// 2. Con TIN, la fórmula clásica: 500 €/mes × 200 meses al 3 % ⇒ i = 0,0025,
    ///    `500 · (1 − 1,0025^−200) / 0,0025 = 78.618,154230356139584585434… €` (tolerancia 1 €,
    ///    hay `powd` por medio). Nota: el plan de la reforma predecía «≈ 78.621»; el valor real,
    ///    verificado a 40 dígitos con aritmética decimal exacta, es 78.618,15 — `powd` de
    ///    `rust_decimal` reproduce los 26 primeros dígitos, así que la desviación era de la
    ///    predicción, no del cálculo.
    /// 3. `n` fraccionario: la equivalencia semanal (`M = cuota·52/12`, `n = intervalos·12/52`)
    ///    reconstruye `cuota · intervalos` exacto con TIN 0.
    #[test]
    fn present_value_of_payments_matches_the_closed_form_and_degenerates_exactly() {
        // (1) Sin descuento: la suma nominal, sin error de redondeo.
        assert_eq!(
            present_value_of_payments(Decimal::from(500), Decimal::from(200), None),
            Decimal::from(100_000)
        );
        assert_eq!(
            present_value_of_payments(Decimal::from(500), Decimal::from(200), Some(Decimal::ZERO)),
            Decimal::from(100_000)
        );
        assert_eq!(
            present_value_of_payments(Decimal::from(500), Decimal::from(200), Some(Decimal::from(-2))),
            Decimal::from(100_000),
            "un TIN negativo degenera igual que la ausencia de TIN"
        );

        // (2) Fórmula cerrada.
        let pv = present_value_of_payments(
            Decimal::from(500),
            Decimal::from(200),
            Some(Decimal::from(3)),
        );
        assert!(
            (pv - dec("78618.154230356139584585434")).abs() < Decimal::ONE,
            "esperado ≈ 78.618,15 €, obtenido {pv}"
        );
        assert!(pv < Decimal::from(100_000), "descontar reduce el nominal");

        // (3) `n` fraccionario: 300 €/semana durante 52 semanas ⇒ M = 300·52/12 = 1.300 €/mes y
        // n = 52·12/52 = 12 meses. Con TIN 0 el producto debe ser 15.600 € exactos.
        let m = Decimal::from(300) * Decimal::from(52) / Decimal::from(12);
        let n = Decimal::from(52) * Decimal::from(12) / Decimal::from(52);
        assert_eq!(
            present_value_of_payments(m, n, None),
            Decimal::from(15_600),
            "la equivalencia semanal es exacta sin descuento"
        );
    }
}
