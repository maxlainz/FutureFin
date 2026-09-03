//! Monthly projection: regular budget (no derived liability rows) + active debt service +
//! asset contributions / drain / compound growth. Ajustes opcionales por mes desde «Próximos»
//! (`planning_monthly_cash_adjustment`) suman al flujo de caja recurrente del mes (ingreso (+)
//! / gasto (−)) antes del reparto a activos o el drenaje.
//!
//! El reparto del sobrante mensual se hace mediante una **cascada de reglas**
//! ([`AllocationRule`]) ejecutadas en orden ascendente. Cada regla consume parte del sobrante
//! para un activo destino hasta su tope opcional; lo que queda pasa a la siguiente regla.

use crate::phases::{EngineWarning, Phase, PhasePlan};
use crate::sim::{SimInput, SimLiability};
use crate::sim_core::{self, liability_extra_principal_g, liability_month_g, plan_alive_g};
use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("horizon_months must be >= 1")]
    InvalidHorizon,
    #[error("planning_monthly_cash_adjustment must have length horizon_months")]
    InvalidPlanningAdjustments,
    #[error("allocation_rules contains an out-of-bounds target_index")]
    InvalidAllocationRuleTarget,
    /// El valor de un activo desbordó el rango de `Decimal` (~7,9e28) al componer su
    /// rentabilidad. Error tipado y no saturación silenciosa: a diferencia del payoff de un
    /// pasivo (donde saturar conserva una serie finita y conservadora), congelar aquí el valor
    /// produciría un patrimonio gigante y PLAUSIBLE — exactamente la clase de número que esta
    /// casa no publica. El input es corregible por el usuario (una rentabilidad absurda).
    #[error("asset value overflowed Decimal range while compounding expected_annual_return_percent over the horizon")]
    AssetValueOverflow,
    #[error("history timeline dates must be strictly ascending")]
    InvalidHistoryTimeline,
    /// 5.0.0: la tabla del objetivo PUENTE no cabe en el tipo numérico — descuento
    /// `bridge_discount_annual_pct` demasiado negativo para el número de meses hasta la pensión.
    ///
    /// **Rango alcanzable y por qué existe el error.** El descuento no se escribe: se DERIVA de
    /// la rentabilidad esperada ponderada de los activos líquidos, y `expected_annual_return_percent`
    /// solo está acotada por `> −100`. Con `d < 0` el factor `q(j) = (1+d/100)^{j/12}` se hunde
    /// hacia cero, el término descontado `G(m)/q(m)` explota y la suma sufijo se sale del rango
    /// de `Decimal` (~7,9e28). La cota depende del número de meses hasta la pensión `P`:
    ///
    /// | `P` (meses) | primer `d` que desborda |
    /// |---|---|
    /// | 120 (10 años) | −99,6 % |
    /// | 204 | −95,9 % |
    /// | 324 | −86,6 % |
    /// | 600 | −66,1 % |
    /// | 840 (70 años) | −53,8 % |
    /// | 1200 (`MAX_BRIDGE_MONTHS`) | −41,8 % |
    ///
    /// Hasta el pase de correcciones de la revisión adversarial esto **panicaba** dentro de
    /// `powd` («Pow overflowed») o de un `+`/`*` de `Decimal`, y salía como un 500 opaco de
    /// `/v1/projection/series`. Ahora es un error tipado; acotar `d` aguas arriba es trabajo de
    /// la API, no del motor, que es una función pura y admite cualquier `Decimal` en su firma.
    #[error("the bridge target table overflowed: bridge_discount_annual_pct is too negative for the months until the pension")]
    BridgeDiscountOverflow,
    /// 5.0.0: el `PhasePlan` pide una regla de retirada que este motor todavía no ejecuta.
    ///
    /// **Desde WP2 no la produce ninguna regla**: las cuatro de `WithdrawalRule` se simulan
    /// (`crates/engine/src/withdrawal.rs`). La variante sobrevive porque `apps/api` la mapea
    /// JUNTO a [`EngineError::UnsupportedPhase`] al mismo `engine_feature_unavailable`
    /// (`handlers/projection.rs::map_engine_err`), y retirarla es un cambio del API que
    /// pertenece a WP3 — cuando la fase parcial y la pensión con fecha dejen de estar pendientes
    /// y `UnsupportedPhase` se vaya con ella.
    #[error("withdrawal rule not implemented yet in this engine version")]
    UnsupportedWithdrawalRule,
    /// 5.0.0 WP2: la regla de retirada trae un parámetro que no describe ninguna política
    /// simulable (un porcentaje ≤ 0, un ajuste de guardarraíl ≥ 100 %). La API ya los acota mucho
    /// antes (`handlers/retirement_profile.rs`), pero el motor es una **función pura** y su firma
    /// admite cualquier `Decimal`: rechaza con un error tipado en vez de panicar o de simular un
    /// plan distinto del configurado.
    #[error("withdrawal rule parameters are outside the simulable range")]
    InvalidWithdrawalRule,
    /// 5.0.0 WP1b: el `PhasePlan` declara una fase que este motor todavía no ejecuta (media
    /// jornada o pensión con fecha; WP3). Mismo criterio que
    /// [`EngineError::UnsupportedWithdrawalRule`].
    #[error("phase plan uses a phase not implemented yet in this engine version")]
    UnsupportedPhase,
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
    /// Cuota mínima revolving: porcentaje del saldo de APERTURA (3 = 3 %/mes). Solo lo usa
    /// `Revolving`; `None` ⇒ 0. (Ola 3/#144.)
    pub min_payment_pct: Option<Decimal>,
    /// Suelo en euros de la cuota mínima revolving. Solo `Revolving`; `None` ⇒ 0.
    pub min_payment_eur: Option<Decimal>,
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
    /// Compensación por reembolso anticipado (Ley 5/2019 art. 23), en % del capital extra
    /// amortizado (cota legal [0, 2] a tipo fijo — la valida el handler). Eje del what-if: el
    /// ensamblado real la deja en `None` (= 0 %); `simulate` aplica su default (2 %). Sale de
    /// la caja como coste puro: NO amortiza ni baja el principal.
    pub early_repayment_fee_pct: Option<Decimal>,
    /// Qué hace la amortización extra con el plan (what-if, #151): acortarlo (`ReduceTerm`,
    /// default y comportamiento 4.4.0 — la cuota no cambia) o bajar la cuota (`ReducePayment`,
    /// λ-escala que conserva EXACTAMENTE el mes de extinción; ver el bucle de simulación).
    pub early_repayment_effect: EarlyRepaymentEffect,
}

/// Efecto de la amortización anticipada sobre el plan de pago (#151). En una renta francesa el
/// plazo restante `n` cumple `(1+i)^(−n) = 1 − P·i/M` — depende SOLO del cociente `P·i/M` —,
/// así que escalar la cuota por el mismo factor que bajó el principal (`M' = λ·M` con
/// `λ = P'/P`) deja el cociente intacto: «reducir cuota» libera caja mensual sin mover el mes
/// de extinción **para una amortización PUNTUAL (lump)** — pineado en test. Con amortización
/// extra RECURRENTE la invariancia es solo un `≤`: el extra es un importe absoluto y cerca del
/// final cancela antes (verificado: 200 €/mes adelanta el 239 a 232). Nunca alarga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EarlyRepaymentEffect {
    /// La cuota no cambia; el préstamo acaba antes (comportamiento histórico del what-if).
    #[default]
    ReduceTerm,
    /// La cuota baja (λ-escala) y el mes de extinción se conserva.
    ReducePayment,
}

/// ¿Devenga interés este pasivo en el mes que empieza en `m_start`?
///
/// ÚNICA definición del predicado (Ola 3, #121): la cumplen por construcción los brazos de
/// `liability_month_g`, y la consumen el KPI `net_return` de `/v1/summary` y su espejo TS
/// `liabilityAccruesInterest` (`apps/web/src/lib/ledger.ts`). Antes eran tres copias y las tres
/// decían cosas distintas: el KPI restaba el TIN de pasivos que la simulación cobraba a 0 €.
///
/// Los modelos que devengan son todos menos `FixedPayments` (post-#144 `interest_only` cobra
/// `P·i` de verdad), siempre que haya TIN > 0 y plan de pago vivo — sin plan, el pasivo es una
/// resta constante al patrimonio: ni caja, ni amortización, ni devengo.
///
/// **5.0.0 WP5.5**: la mitad «¿plan vivo?» del predicado vive en el núcleo genérico
/// (`sim_core::plan_alive_g`), que es donde la ejecuta el bucle. Aquí solo se instancia.
pub fn liability_interest_accrues(
    model: RepaymentModel,
    apr_percent: Option<Decimal>,
    monthly_payment: Decimal,
    payment_end: Option<NaiveDate>,
    m_start: NaiveDate,
) -> bool {
    model != RepaymentModel::FixedPayments
        && matches!(apr_percent, Some(a) if a > Decimal::ZERO)
        && plan_alive_g(monthly_payment, payment_end, m_start)
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
    /// Compensación por reembolso anticipado del mes (#151): `extra_principal ×
    /// early_repayment_fee_pct / 100`. Coste puro — queda FUERA de la identidad
    /// `payment + extra_principal == interest_accrued + principal_repaid` a propósito: sale de
    /// la caja pero no amortiza. `0` en el calendario real de un pasivo guardado.
    pub early_repayment_fee: Decimal,
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
    /// Σ `early_repayment_fee` (#151). `0` sin what-if de amortización o con comisión 0.
    pub total_early_repayment_fee: Decimal,
    /// `total_payments + total_extra_principal + total_early_repayment_fee`: todo lo que sale
    /// de la caja. Es el «total a pagar» de la pregunta.
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

/// Serie del término finito de deuda del objetivo FIRE (#142): para cada mes `m`,
/// `Σ_pasivos ( Σ_{j>m} caja(j) + saldo_residual )` — la caja de cada mes es la del calendario
/// completo (cuota + amortización extra + comisión), y el saldo residual es lo que quede vivo
/// al terminar el plan (constante: esa deuda no se paga sola). El último elemento del vector es
/// la cola residual; consumido por `fire_target_at_month_index` con fallback al último valor.
///
/// Invariantes pineadas por test: `serie[0] == Σ total_cash_out + Σ final_principal` y, para un
/// francés que se extingue, `serie[m] == principal_vivo(m) + interés_restante(m)` — la identidad
/// que hace equivalentes la base líquida + cuotas (#143) y la base NW + interés.
///
/// **No depende del horizonte de la proyección** (invariante protegido por test): se calcula
/// sobre el calendario completo (`MAX_LIABILITY_SCHEDULE_MONTHS`), no sobre los meses servidos.
pub fn debt_payments_remaining_series(
    liabilities: &[ProjectionLiabilityInput],
    ref_date: NaiveDate,
) -> Vec<Decimal> {
    let mut monthly: Vec<Decimal> = Vec::new();
    let mut residual_tail = Decimal::ZERO;
    for liab in liabilities {
        let sch = liability_amortization_schedule(liab, ref_date, MAX_LIABILITY_SCHEDULE_MONTHS);
        residual_tail += sch.final_principal;
        for m in &sch.months {
            let idx = m.month_index as usize;
            if monthly.len() <= idx {
                monthly.resize(idx + 1, Decimal::ZERO);
            }
            monthly[idx] += m.payment + m.extra_principal + m.early_repayment_fee;
        }
    }
    if monthly.is_empty() {
        return if residual_tail > Decimal::ZERO {
            vec![residual_tail]
        } else {
            Vec::new()
        };
    }
    // Sufijos ESTRICTOS: serie[m] = Σ_{j > m} monthly[j] + residual_tail — la caja del mes m ya
    // está pagada al cierre de m (el cruce compara con el cierre de k−1, ver el bucle). Los
    // meses del calendario son 1-based, así que serie[0] cubre el plan entero.
    let len = monthly.len();
    let mut out = vec![Decimal::ZERO; len];
    let mut acc = residual_tail;
    for m in (0..len).rev() {
        out[m] = acc;
        acc += monthly[m];
    }
    out
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
    // 5.0.0 WP5.5: la recurrencia del pasivo vive en el núcleo genérico, así que el calendario
    // convierte el pasivo UNA vez (una copia campo a campo) y ejecuta EXACTAMENTE los mismos
    // helpers que el bucle de simulación. Cero matemática nueva, igual que antes.
    let liab: &SimLiability<Decimal> = &SimLiability::from(liab);
    let opening_principal = liab.principal.max(Decimal::ZERO);
    let start_month_first = month_first_calendar(ref_date);

    let mut principal = opening_principal;
    let mut months: Vec<LiabilityScheduleMonth> = Vec::new();
    let mut total_interest = Decimal::ZERO;
    let mut total_payments = Decimal::ZERO;
    let mut total_extra_principal = Decimal::ZERO;
    let mut total_early_repayment_fee = Decimal::ZERO;
    // Cuota efectiva (#151): solo la muta «reducir cuota»; con el default es la declarada.
    let mut effective_payment = liab.monthly_payment;
    // Un pasivo con saldo 0 ya está extinguido HOY: `Some(0)` y calendario vacío. No se deja caer
    // al bucle porque emitiría meses de ceros que no describen nada.
    let mut payoff_month_index = principal.is_zero().then_some(0u32);
    let mut plan_ended = false;

    if !principal.is_zero() {
        for k in 1..=horizon {
            let month_first = add_months(start_month_first, k - 1);
            let (m_start, _m_end) = month_window(month_first);
            let active = sim_core::liability_active_g(liab, m_start);
            if !active {
                plan_ended = true;
                break;
            }

            let (payment, closing_after_payment) =
                liability_month_g(liab, principal, effective_payment, true);
            let (extra, fee) = liability_extra_principal_g(liab, k, closing_after_payment, true);
            let closing = closing_after_payment - extra;

            // Derivación en este orden a propósito: los saldos mandan, el interés es el residuo.
            // Así `payment + extra == interest + principal_repaid` es exacto por construcción y
            // no una coincidencia numérica que un cambio de modelo pueda romper. La comisión
            // (#151) queda FUERA de la identidad: es caja sin contrapartida en el principal.
            let repaid_by_payment = principal - closing_after_payment;
            let interest_accrued = payment - repaid_by_payment;
            let principal_repaid = principal - closing;

            // «Reducir cuota» (#151): misma λ-escala que el bucle de simulación.
            if extra > Decimal::ZERO
                && liab.early_repayment_effect == EarlyRepaymentEffect::ReducePayment
                && closing_after_payment > Decimal::ZERO
            {
                effective_payment = effective_payment * closing / closing_after_payment;
            }

            months.push(LiabilityScheduleMonth {
                month_index: k,
                opening_principal: principal,
                interest_accrued,
                principal_repaid,
                extra_principal: extra,
                early_repayment_fee: fee,
                payment,
                closing_principal: closing,
            });
            total_interest += interest_accrued;
            total_payments += payment;
            total_extra_principal += extra;
            total_early_repayment_fee += fee;
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
        total_early_repayment_fee,
        total_cash_out: total_payments + total_extra_principal + total_early_repayment_fee,
        payoff_month_index,
        payoff_absent,
        horizon_months: horizon,
    }
}

/// Target FIRE evaluado mes a mes SOBRE LA NECESIDAD (#170): el objetivo del mes `k` es
/// `gross_up(need(k)) / SWR + término_deuda(k)`, con la necesidad indexada según su
/// estructura (`FireNeed`) — no una base pre-calculada que se infla entera.
#[derive(Debug, Clone)]
pub struct FireTarget {
    /// La NECESIDAD, no el resultado (#170): hasta 4.9.0 aquí vivía `base_amount` — una cifra
    /// ya grosseada, ya dividida por el SWR y con la pensión YA RESTADA antes de inflar. Eso
    /// infló el neto entero mientras el motor drena `gasto·f(k) − pensión`: el objetivo se
    /// quedaba corto en `pensión·(f(k)−1)` al mes. Ahora el objetivo se evalúa mes a mes sobre
    /// los ingredientes.
    pub need: FireNeed,
    /// SWR en % (3,5 = 3,5 %). ≤ 0 ⇒ sin objetivo (el handler ya lo rechaza antes; guarda).
    pub swr_pct: Decimal,
    /// La MISMA escala y el MISMO switch que el drenaje (#140): objetivo y venta simulada
    /// hablan del mismo euro bruto.
    pub tax_brackets: Vec<crate::tax::TaxBracket>,
    pub taxes_enabled: bool,
    /// Fracción de cada euro bruto que es plusvalía gravable (g, [0,1] — #140 fase 2).
    /// `ONE` = histórico (reembolso íntegro gravado); `ZERO` ≡ sin impuestos.
    pub taxable_gain_ratio: Decimal,
    pub annual_inflation_percent: Decimal,
    /// Término finito de deuda (#142): ver `debt_payments_remaining_series`.
    pub debt_payments_remaining: Vec<Decimal>,
}

/// Estructura de la necesidad por modo FIRE (#170) — NO es la misma en los tres modos.
#[derive(Debug, Clone, PartialEq)]
pub enum FireNeed {
    /// Cifra declarada en euros de HOY que se indexa ENTERA (modos `manual` y
    /// `current_income`: con los ingresos planos de #139, descomponerla crearía un objetivo
    /// plano — un cambio semántico que nadie pidió).
    Indexed { annual_net_today: Decimal },
    /// Gasto de jubilación (euros de hoy, se INDEXA con el gasto del bucle) menos pensión
    /// (PLANA por decisión de #139) — modo `annual_expense`. Es la necesidad REAL que el
    /// drenaje ejecuta: `max(0, E·f(k) − I)·12`.
    ExpenseMinusPension {
        expense_monthly: Decimal,
        pension_monthly: Decimal,
    },
}

/// **5.0.0 WP5.5**: `annual_net_at` —la expresión de la necesidad— vive en el gemelo genérico
/// ([`crate::sim::FireNeedG`]), que es el que ejecuta el núcleo. Una sola definición: la fórmula
/// duplicada es la trampa que #170 ya pagó una vez.

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    /// Civil "today" de la instalación (inicio del mes simulado para el índice 1).
    pub ref_date: NaiveDate,
    pub horizon_months: u32,
    /// Inflación anual asumida de la instalación, en % ([−2, 50] tras la validación de la API).
    /// Desde la Ola 5 (#139) indexa el GASTO del bucle — regular y de jubilación — con
    /// `inflation_factor_at_month_index` sobre el eje `(k−1)/12`; los ingresos quedan planos
    /// (decisión del owner). Independiente de `FireTarget.annual_inflation_percent` en la firma
    /// (el target puede no existir y el gasto se indexa igual); el handler rellena ambos del
    /// MISMO supuesto efectivo, overrides de simulate incluidos.
    pub annual_inflation_percent: Decimal,
    /// Escala del ahorro con la que se GROSSEA todo drenaje de activos (#140 fase 1): vender un
    /// fondo para cubrir un déficit realiza plusvalía, jubilado o no — gatear por fase crearía
    /// un salto artificial del +25,95 % en el cruce. La fase 1 grava el 100 % de lo drenado
    /// (base íntegra, sin mirar la base de coste de #120 — lectura literal del issue); la caja
    /// (`surplus_cash`) NUNCA se grossea: entró ya tributada como renta. Vacía ⇒ sin impuesto.
    pub tax_brackets: Vec<crate::tax::TaxBracket>,
    /// `false` ⇒ bruto = neto: la rama de déficit es bit-idéntica a 4.9.0.
    pub taxes_enabled: bool,
    /// g de #140 fase 2 — la MISMA fracción que el objetivo (`FireTarget.taxable_gain_ratio`):
    /// el drenaje y el target dimensionan la misma venta bruta.
    pub taxable_gain_ratio: Decimal,
    pub income_regular_monthly: Decimal,
    pub expense_regular_monthly: Decimal,
    pub assets: Vec<SimAsset>,
    /// Cascada de reglas, en orden ascendente de prioridad (índice 0 = primera).
    pub allocation_rules: Vec<AllocationRule>,
    pub liabilities: Vec<ProjectionLiabilityInput>,
    /// Signed cash from planning flows per simulated month (`len == horizon_months`): index `i`
    /// pairs with month `i+1` (calendar month `add_months(month_first_calendar(ref_date), i)`).
    pub planning_monthly_cash_adjustment: Vec<Decimal>,
    /// **Plan de fases** (5.0.0 WP1b, `phases.rs`): trigger de jubilación, ingreso y gasto de la
    /// fase jubilada, retirada extra, y —declaradas pero aún no simuladas— fase parcial, pensión
    /// con fecha y regla de retirada. Absorbe los cuatro campos sueltos de 4.15.0
    /// (`retirement_start_month`, `income_retirement_monthly`, `expense_retirement_monthly`,
    /// `retirement_monthly_withdrawal`), que el bucle y `first_month_allocation` interpretaban
    /// cada uno por su cuenta. [`PhasePlan::classic`] reproduce 4.15.0 bit a bit.
    pub phase_plan: PhasePlan,
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
    /// **El mes en que la cartera se quedó sin nada Y eso costó dinero** (1-based, misma base que
    /// el bucle). Se publica solo si se cumplen las DOS condiciones, y en este orden:
    ///
    /// 1. Es el PRIMER mes cuya venta dejó lo vendible a cero (o no se pudo fundar). Se mide
    ///    DESPUÉS de vender, sobre los saldos: si cada activo entregó su capacidad entera,
    ///    `v − v = 0` exacto en cualquier tipo numérico. El viejo predicado `venta_bruta ≥
    ///    drenable` comparaba dos cantidades calculadas por caminos distintos y `Decimal` y `f64`
    ///    lo resolvían al revés en el aterrizaje exacto.
    /// 2. Desde ese mes en adelante, **alguna venta se quedó sin fundar**. Sin esto, un puente que
    ///    se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto posterior
    ///    —un plan perfecto— se publicaba como «cartera agotada en el mes 120» con
    ///    `uncovered_deficit_total = 0`.
    ///
    /// `None` = no se agota dentro del horizonte; no es «no calculado». Issue #119 + hallazgo #2
    /// de la segunda revisión adversarial.
    pub assets_depleted_month_index: Option<u32>,
    /// Déficit acumulado NO cubierto al final del horizonte (= `undrained_cumulative`). `0` es
    /// cero euros descubiertos, no «no aplica». Ya se restaba del patrimonio publicado; ahora
    /// además se declara. Issue #119.
    ///
    /// **Puede traer una cola de ±1e-24 € y NO se clampa aquí.** El acumulador suma el operando
    /// LITERAL de 4.15.0 (`need − after_tax(bruto)` en la vía escalar, `net_shortfall_monthly` en
    /// la mixta), y `after_tax(gross_up(n))` devuelve `n` solo hasta el redondeo a 28 dígitos:
    /// medido, hasta −1,7e-24 € en un hogar que nunca se acerca a quedarse sin cartera. Tocarlo
    /// con un `max(0,·)` movería el pin dorado, y este campo existe para NO moverse. **Quien
    /// publica clampa**: la serie [`Self::unmet_need`] ya sale clampada, y el handler de la API
    /// clampa este total antes de ponerlo en el JSON.
    pub uncovered_deficit_total: Decimal,
    /// Ahorro mensual que ninguna regla de la cascada absorbió, acumulado en euros nominales
    /// (4.12.1, decisión del owner). NO entra en `net_worth`, NO compone y NO cuenta en
    /// `contributed_capital`: el modelo se niega a simular un euro sin destino declarado. `0`
    /// es cero euros, no «no aplica». En producción es inalcanzable con activos vivos
    /// (sumidero indestructible #176 + retro-siembra 4.12.0); existe porque el motor es una
    /// función pura y debe definir el estado.
    pub unallocated_savings_total: Decimal,
    /// Patrimonio LÍQUIDO por mes (#143): Σ de los activos con `is_liquid` — exactamente el
    /// stock que el drenaje de jubilación puede vender, SIN restar pasivos (por eso el término
    /// de deuda del objetivo son las cuotas completas, ver
    /// `FireTarget::debt_payments_remaining`) y, desde 4.12.1, sin término de caja
    /// (`surplus_cash` murió). Es la base que decide el cruce FIRE desde 4.8.0; `net_worth`
    /// sigue siendo el total y no cambia.
    pub liquid_worth: Vec<Decimal>,
    // -----------------------------------------------------------------------------------------
    // 5.0.0 WP1b — LECTURAS de fase (§B.8). Ninguna cambia la aritmética: todas se derivan de
    // valores que el bucle ya tenía en la mano. El pin dorado de 4.15.0 (`pins-4.15.json`) sigue
    // hasheando SOLO los campos de arriba, y estos se pinean aparte en `pins-5.0-outputs.json`.
    // -----------------------------------------------------------------------------------------
    /// Primer mes (1-based, misma base que `assets_depleted_month_index`) en el que el hogar está
    /// jubilado — por cruce o por trigger forzado, lo que ocurra ANTES. `None` = no se jubila
    /// dentro del horizonte; no es «no calculado». Es el mes efectivo que el handler publicará
    /// como `jubilacion_month_index` (R8) cuando WP5 cambie esa derivación.
    pub retirement_month_index: Option<u32>,
    /// Primer mes con `líquido(k−1) ≥ objetivo(k−1)`. **Lectura pura**: se evalúa cada mes,
    /// también DESPUÉS de que el latch cierre, y no interviene en ninguna decisión. Con una
    /// jubilación forzada anterior al cruce, este índice es posterior a
    /// `retirement_month_index`; sin `fire_target` es `None`.
    pub liquid_crossing_month_index: Option<u32>,
    /// Fases atravesadas, en orden y con el mes 1-based en que empieza cada una. En WP1b son dos
    /// como mucho: `[(Accumulating, 0)]` y, si hay jubilación, `(Retired, retirement_month_index)`.
    /// `Partial` entra en WP3.
    pub phase_transitions: Vec<(Phase, u32)>,
    /// Retirada NETA efectiva del mes: los euros que de verdad salieron de los activos —
    /// `after_tax(bruto vendido)`, con el bruto decidido por la regla de retirada y el modo de
    /// gasto (§B.2). `len == horizon+1`, índice 0 = 0 (el mes 0 es el estado inicial, no un mes
    /// simulado). Con `fixed_real` ES el drenaje de 4.15.0 — el que ya alimentaba
    /// `uncovered_deficit_total`.
    pub withdrawal: Vec<Decimal>,
    /// Recorte de la REGLA: `max(0, necesidad_neta − neto que el techo permitía)` (B.1.5 /
    /// D22-D24). **Informativo y solo eso**: no resta patrimonio, no entra en
    /// `uncovered_deficit_total` y no cuenta como fracaso. Mide cuánto gasto declarado dejó fuera
    /// la política de retirada, NO cuánto no pudieron vender los activos — esa es la otra
    /// magnitud, y confundirlas fue el hallazgo B2 de la revisión adversarial.
    ///
    /// Cero por construcción con `fixed_real` (no hay techo que recorte) y, en modo `ceiling`,
    /// fuera de los meses de déficit.
    pub withdrawal_shortfall: Vec<Decimal>,
    /// Exceso de la regla sobre la necesidad en modo `rule_is_spend`: `max(0, retirada neta −
    /// necesidad neta)`. Son euros VENDIDOS y GASTADOS — salen de la cartera y no vuelven a
    /// entrar. Cero por construcción en modo `ceiling` (allí la retirada nunca supera la
    /// necesidad) y con `fixed_real` en cualquiera de los dos modos.
    pub withdrawal_excess: Vec<Decimal>,
    /// **Necesidad NETA que el mes NO obtuvo** porque los activos no pudieron fundar la venta:
    /// el incremento mensual de `uncovered_deficit_total`, clampado a 0 (el acumulador conserva
    /// el operando literal de 4.15.0, que puede llevar una cola de ±1e-24; una serie publicada
    /// no). `len == horizon+1`, índice 0 = 0.
    ///
    /// Es la TERCERA magnitud del mes y la que faltaba: `withdrawal` es lo que se obtuvo,
    /// `withdrawal_shortfall` lo que la REGLA rechazó y `unmet_need` lo que la CARTERA no dio.
    /// Su suma es la necesidad neta del mes, y sin ella cualquier cociente de cobertura miente
    /// en el caso que más importa —la cartera agotada— porque con `fixed_real` el recorte es
    /// cero por construcción (hallazgo #4 de la revisión adversarial: un cociente publicado de
    /// 1,0 sobre caminos que cubrían el 8,65 % de la necesidad).
    pub unmet_need: Vec<Decimal>,
    /// Primer mes con pensión con fecha. `None` en WP1b (la pensión con fecha llega en WP3; la
    /// pensión plana de hoy viaja dentro de `income_retirement_monthly` y no tiene mes propio).
    pub pension_start_month_index: Option<u32>,
    /// Primer mes de media jornada. `None` en WP1b (WP3).
    pub partial_retirement_month_index: Option<u32>,
    /// Avisos del motor (§B.8). Desde WP3 el bucle sabe emitir dos —jubilación por edad
    /// infra-financiada y capital menguante en media jornada—; el tercero
    /// ([`EngineWarning::CoastNotReachable`]) lo emite el solve, que es quien lo puede saber.
    /// Los de ensamblado (`birth_date_missing`) los añade el handler.
    pub warnings: Vec<EngineWarning>,
    // -----------------------------------------------------------------------------------------
    // 5.0.0 WP3 — LECTURAS de pensión, puente y media jornada (§B.3, §B.7). Todas APÉNDICE: el
    // pin de 4.15.0 no las mira y el aditivo de 5.0.0 sí.
    // -----------------------------------------------------------------------------------------
    /// **Tasa de retirada efectiva del puente**, en % ANUAL:
    /// `100 · 12·need_full_m(R−1) / L(R−1)` en el mes efectivo de jubilación.
    ///
    /// Responde a la pregunta que el puente plantea y la perpetuidad esconde: mientras la pensión
    /// no llega hay que sacar de la cartera el gasto ENTERO, y eso es una tasa que puede estar muy
    /// por encima del SWR — legítimamente, porque dura pocos años (D7: por eso el riesgo del
    /// puente es lo que Monte Carlo tendrá que medir en WP6).
    ///
    /// `None` sin pensión con fecha, sin base puente, sin objetivo, sin jubilación dentro del
    /// horizonte o con `L(R−1) ≤ 0`: en ninguno de esos casos hay una tasa que medir — **jamás un
    /// cero inventado**.
    pub bridge_effective_withdrawal_pct: Option<Decimal>,
    /// **Qué fracción del gasto cubre la pensión** el mes en que empieza: `P_m(P)/(E·f(P))`, en
    /// FRACCIÓN (0,6 = 60 %). Es la lectura que hace explícitos los dos escenarios de D15 sin
    /// asumir ninguno. `None` sin pensión con fecha, sin objetivo o con gasto no positivo en `P`.
    pub pension_coverage_ratio: Option<Decimal>,
    /// Capital que sostendría a perpetuidad el HUECO de la media jornada:
    /// `gross_up(12·gap_m(X))/SWR` (§B.3). Informativo: no dispara nada.
    ///
    /// `None` cuando la fase parcial **no llegó a vivirse** (declarada o no: si el hogar se jubila
    /// del todo antes de `X`, no hay hueco que medir), sin objetivo, o sin fase parcial en el
    /// plan; `Some(0)` = la media jornada se paga sola.
    ///
    /// Va atado a [`Self::partial_retirement_month_index`], igual que
    /// [`Self::partial_phase_capital_growing`]: los dos describen la MISMA fase y no pueden usar
    /// criterios distintos para existir. Antes del pase de correcciones de la revisión
    /// adversarial este se calculaba de la fase DECLARADA y publicaba 270.000 € para una media
    /// jornada que el cruce FIRE había dejado 58 meses atrás.
    pub partial_gap_target: Option<Decimal>,
    /// `true` ⟺ **hubo** fase parcial y el patrimonio LÍQUIDO no bajó ni un mes durante ella.
    ///
    /// Sin fase parcial es `false` — no hay fase que crezca. Para distinguir «no hubo» de «hubo y
    /// menguó» está [`ProjectionOutput::partial_retirement_month_index`]; el caso malo además
    /// emite [`EngineWarning::PartialPhaseCapitalShrinking`].
    pub partial_phase_capital_growing: bool,
    /// Caja del mes que un techo de aportación (`PhasePlan::contribution_cap_monthly` o el corte
    /// de coast) dejó FUERA de la cascada. `len == horizon+1`, índice 0 = 0.
    ///
    /// **No es patrimonio**: no se invierte, no compone y no entra en `net_worth` — exactamente el
    /// mismo trato que `unallocated_savings_total`, y por la misma razón (el modelo no simula un
    /// euro sin destino declarado). Es el «margen disponible» de D16: lo que el hogar podría
    /// gastarse sin mover su fecha de jubilación.
    ///
    /// Identidad del mes, con `sobrante > 0`: `sobrante = Σ aportado + no_asignado + disposable`.
    /// Sin techo es cero mes a mes.
    pub disposable_cash: Vec<Decimal>,
    /// Σ de [`ProjectionOutput::disposable_cash`]. `0` son cero euros, no «no aplica».
    pub disposable_cash_total: Decimal,
}

/// Primero-de-mes de una fecha (día 1 del mismo mes). Compartido con `history.rs`.
pub(crate) fn month_first_calendar(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

pub(crate) fn add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(Months::new(n)).unwrap_or(d)
}

pub(crate) fn month_window(month_first: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = month_first;
    let next_first = add_months(month_first, 1);
    let end = next_first.pred_opt().unwrap_or(start);
    (start, end)
}

/// Factor de crecimiento **mensual** equivalente a una tasa anual nominal — envoltorio `Decimal`
/// de [`crate::sim_core::monthly_multiplier_g`], que es donde vive la semántica (tasas negativas
/// que componen, clamp a 0 por debajo de −100 %).
///
/// `pub(crate)` porque `runway.rs` lo comparte: el runway debe usar EXACTAMENTE la misma
/// conversión anual→mensual que la simulación, o divergiría del chart de proyección.
pub(crate) fn monthly_multiplier(annual_percent: Option<Decimal>) -> Decimal {
    sim_core::monthly_multiplier_g(annual_percent)
}

/// Factor de indexación al IPC en el índice de mes `m`: `(1 + annual_percent/100)^(m/12)`.
///
/// Envoltorio `Decimal` de [`crate::sim_core::inflation_factor_at_index_g`] — **única
/// implementación del factor**: la consumen el objetivo FIRE, la indexación del gasto del bucle
/// (#139) y, desde 5.0.0, `apps/api` para su factor de crecimiento real (en vez de mantener una
/// copia local).
///
/// `m = 0` o `annual_percent == 0` ⇒ `ONE` **exacto**. La guarda es `is_zero()`, NO `<= ZERO`
/// (#146): una inflación negativa DEBE componer.
pub fn inflation_factor_at_month_index(annual_percent: Decimal, month_index: u32) -> Decimal {
    sim_core::inflation_factor_at_index_g(annual_percent, month_index)
}

/// La BASE del objetivo (sin el término de deuda) en el mes `month_index` — evaluada sobre la
/// necesidad REAL del mes (#170): `gross_up(need(k), tramos, g) / SWR`. La puerta de `k = 0`
/// decide para TODA la serie: sin necesidad positiva HOY no hay objetivo en ningún mes — un
/// `max(0,·)` suelto publicaría `target = 0` y un cruce FIRE inmediato y falso (D-8; el caso 4 de
/// fire-parity, pensión > gasto, es su regresión).
///
/// El gross-up de la necesidad INFLADA no es el gross-up inflado: la escala es afín, no
/// homogénea, y los tramos son NOMINALES — retirar más euros nominales dentro de 30 años SÍ cae
/// en tramos más altos (fiscal drag: +7.140,43 € a 30 años con 24.000 €/año al 2 %, sin pensión).
pub fn fire_target_base_at_month_index(ft: &FireTarget, month_index: u32) -> Option<Decimal> {
    let brackets = crate::sim::TaxBracketG::<Decimal>::from_decimal_slice(&ft.tax_brackets);
    sim_core::fire_target_base_at_index_g(fire_target_view(ft, &brackets), month_index)
}

/// Target FIRE en el `month_index` indicado (0 = punto de partida, 12 = un año después, etc.).
///
/// Es la **única fuente de verdad** del objetivo clásico: tanto el motor (para decidir
/// `fire_reached`) como el handler de la API (para construir `fire_target_series`) la consumen,
/// evitando off-by-one entre la serie y el cruce.
///
/// OJO: el objetivo **NO es monótono**, y por DOS razones — término de deuda decreciente (#142)
/// y, con pensión, base que crece MÁS rápido que f(k) (#170). Cualquier optimización que asuma
/// monotonía (búsqueda binaria del cruce, salida temprana) quedaría rota en silencio: escaneo
/// lineal, siempre.
///
/// **Esta función es la de 4.15.0 y se queda así**: el objetivo por FASES (pensión con fecha,
/// puente) vive en [`crate::target`] y, cuando el plan no trae pensión, ejecuta esta misma
/// función del núcleo. Que el camino común pase por aquí es lo que hace que el pin dorado no
/// pueda moverse.
pub fn fire_target_at_month_index(ft: Option<&FireTarget>, month_index: u32) -> Option<Decimal> {
    let ft = ft?;
    let brackets = crate::sim::TaxBracketG::<Decimal>::from_decimal_slice(&ft.tax_brackets);
    sim_core::fire_target_at_index_g(Some(fire_target_view(ft, &brackets)), month_index)
}

/// La vista PRESTADA de un objetivo público, con la escala de tramos ya convertida.
///
/// La escala se convierte (5 elementos); la serie de deuda —hasta 841 números— se **presta**.
/// Convertir el objetivo entero en cada evaluación costaría ~11 MB de copias por request para
/// leer un elemento del vector, y `fire_target_at_month_index` la llama una vez por mes.
fn fire_target_view<'a>(
    ft: &'a FireTarget,
    brackets: &'a [crate::sim::TaxBracketG<Decimal>],
) -> crate::sim::FireTargetView<'a, Decimal> {
    crate::sim::FireTargetView {
        need: crate::sim::FireNeedG::from(&ft.need),
        swr_pct: ft.swr_pct,
        tax_brackets: brackets,
        taxes_enabled: ft.taxes_enabled,
        taxable_gain_ratio: ft.taxable_gain_ratio,
        annual_inflation_percent: ft.annual_inflation_percent,
        debt_payments_remaining: &ft.debt_payments_remaining,
    }
}

/// Orden TOTAL de drenaje: líquidos primero; dentro de cada grupo, menor rentabilidad esperada
/// primero (`None` cuenta como 0); empate por índice.
///
/// Envoltorio `Decimal` de [`crate::sim_core::drain_order_g`], que es la **implementación ÚNICA**
/// (#178). Lo consume el bucle finito del runway; el bucle de simulación llama al genérico
/// directamente. Una segunda copia haría divergir en silencio la base gravada y la venta
/// ejecutada.
pub(crate) fn drain_order(liquid: &[bool], rates: &[Option<Decimal>]) -> Vec<usize> {
    sim_core::drain_order_g(liquid, rates)
}

/// Resolve a rule's cap into an absolute € ceiling for the destination asset.
/// Returns `None` for an uncapped rule.
///
/// `pub` desde la Ola 1 (issue #96): el techo de una regla depende de la regla y de los
/// escalares del mes, NO de si hay sobrante — así que se resuelve siempre, y esta es la única
/// implementación (la copia del handler `resolve_cap_ceiling_eur` delega aquí).
pub fn resolve_cap_ceiling(
    cap: Option<AllocationCap>,
    monthly_expense_with_debt: Decimal,
    monthly_income: Decimal,
) -> Option<Decimal> {
    let cap = cap.map(|c| match c {
        AllocationCap::Amount(v) => crate::sim::AllocationCapG::Amount(v),
        AllocationCap::MonthsExpense(v) => crate::sim::AllocationCapG::MonthsExpense(v),
        AllocationCap::IncomeMultiple(v) => crate::sim::AllocationCapG::IncomeMultiple(v),
    });
    sim_core::resolve_cap_ceiling_g(cap, monthly_expense_with_debt, monthly_income)
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
/// `Σ per_asset + leftover + disposable = base_cash` cuando `base_cash > 0` (con `base_cash ≤ 0`
/// no se reparte nada y `leftover` y `disposable` son 0). Sin techo de aportación —el único caso
/// que produce el camino de lectura— `disposable` es 0 y la identidad es la de 4.15.0.
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
    /// 5.0.0 WP3: la parte del sobrante que un **techo de aportación**
    /// (`PhasePlan::contribution_cap_monthly`, o el corte de coast) dejó fuera de la cascada.
    /// `0` sin techo, que es el único caso que la API de lectura produce hoy — el techo lo ponen
    /// los solves, sobre entradas que ellos mismos clonan.
    ///
    /// Extiende la identidad documentada arriba: con `base_cash > 0`,
    /// `Σ per_asset + leftover + disposable = base_cash`.
    pub disposable: Decimal,
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
    let sim = SimInput::<Decimal>::from(input);
    sim_core::first_month_allocation_g(&sim).map(FirstMonthAllocation::from)
}

/// La simulación mensual completa. **5.0.0 WP5.5**: envoltorio de
/// [`crate::sim_core::simulate`] — convierte la entrada al tipo del núcleo (una copia campo a
/// campo, cero operaciones) y devuelve la salida movida sin copiar un número.
pub fn project_net_worth_series(input: &ProjectionInput) -> Result<ProjectionOutput, EngineError> {
    let sim = SimInput::<Decimal>::from(input);
    sim_core::simulate(&sim).map(ProjectionOutput::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::RetirementTrigger;
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

    /// Target «plano equivalente» al histórico `base_amount` pre-#170: `Indexed` con SWR
    /// 100 % y sin impuestos ⇒ `target(k) = base·f(k) + término_deuda`, EXACTO al contrato
    /// antiguo — mantiene válidos, sin mover un dígito, los pins escritos contra base_amount.
    fn ft_flat(base: Decimal, inflation: Decimal) -> FireTarget {
        FireTarget {
            need: FireNeed::Indexed {
                annual_net_today: base,
            },
            swr_pct: Decimal::from(100u32),
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: inflation,
            debt_payments_remaining: Vec::new(),
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
            annual_inflation_percent: Decimal::ZERO,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            income_regular_monthly: income,
            expense_regular_monthly: expense,
            assets,
            allocation_rules: rules,
            liabilities: vec![],
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
            phase_plan: PhasePlan::classic(Decimal::ZERO, expense),
            fire_target: None,
        }
    }

    /// #143 — solo el patrimonio LÍQUIDO decide el cruce. Cartera del issue: vivienda ilíquida
    /// de 400.000 € (0 %) + fondo líquido de 480.000 € (5 %), objetivo 863.652,80 €. El total
    /// (880.000) supera el objetivo — la app de 4.7.0 declaraba «FIRE hoy» con un déficit real
    /// de 383.652,80 € de capital vendible. Con la base líquida, el hogar SIGUE trabajando:
    /// la aportación del mes 1 entra (1.000 € = ingreso − gasto), cosa que jubilado no haría.
    #[test]
    fn the_crossing_uses_only_liquid_worth() {
        let casa = mk_asset(0xA1, Decimal::from(400_000), false, None);
        let fondo = mk_asset(0xA2, Decimal::from(480_000), true, Some(Decimal::from(5)));
        let mut inp = base_input(
            24,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![casa, fondo],
            vec![rule_remainder(1)],
        );
        inp.fire_target = Some(ft_flat(dec("863652.80"), Decimal::ZERO));
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(
            out.contributed_capital[1],
            Decimal::from(1_000),
            "880.000 totales no jubilan a nadie: el líquido (480.000) no llega"
        );
        // La serie líquida publicada arranca en el fondo, sin la casa.
        assert_eq!(out.liquid_worth[0], Decimal::from(480_000));
        assert!(out.net_worth[0] == Decimal::from(880_000));
    }

    /// #142 ⇄ #143, el invariante de EMPAREJAMIENTO (spike §4.3): con base líquida BRUTA el
    /// término del objetivo son TODAS las cuotas restantes (+ residual), y eso es EXACTAMENTE
    /// el mismo requisito sobre activos que «base NW + interés restante»:
    /// `serie[m] == principal_vivo(m) + interés_restante_tras(m)` — identidad del calendario.
    /// Números del caso E del spike (francés 100.000 € / TIN 3 % / cuota 500, 278 meses):
    /// serie[0] = **138.802,7999147153** (Σ cuotas; residual 0), serie[139] = 69.302,799915
    /// (= P(139) 58.508,940182 + interés restante 10.793,859733), serie[278] = 0.
    #[test]
    fn target_and_crossing_base_agree_on_the_liability_accounting() {
        let l = liab(
            Decimal::from(100_000),
            Decimal::from(500),
            RepaymentModel::French,
            Some(Decimal::from(3)),
        );
        let serie = debt_payments_remaining_series(std::slice::from_ref(&l), ref_2026());
        let sch = liability_amortization_schedule(&l, ref_2026(), MAX_LIABILITY_SCHEDULE_MONTHS);

        assert_eq!(
            serie[0].round_dp(10),
            dec_s("138802.7999147153"),
            "serie[0] = Σ cuotas del plan entero"
        );
        assert_eq!(
            serie[278],
            Decimal::ZERO,
            "extinguido: no queda nada que cubrir"
        );
        assert_eq!(
            serie[139].round_dp(6),
            dec_s("69302.799915"),
            "k=139: P(139) + interés restante"
        );

        // La identidad, mes a mes (muestreada): cuotas restantes == principal vivo + interés
        // restante. Si alguien cambia una de las dos contabilidades sin la otra, esto revienta.
        for probe in [1usize, 50, 139, 200, 277] {
            let principal_close = sch.months[probe - 1].closing_principal;
            let interest_after: Decimal =
                sch.months[probe..].iter().map(|m| m.interest_accrued).sum();
            // Igualdad a 15 decimales: son las MISMAS cantidades sumadas en orden distinto, y
            // Decimal (28 dígitos) acumula el redondeo del último dígito de forma distinta.
            assert_eq!(
                serie[probe].round_dp(15),
                (principal_close + interest_after).round_dp(15),
                "identidad rota en el mes {probe}"
            );
        }
    }

    /// La cola residual (#142): un plan que vence con saldo vivo deja ese saldo como término
    /// CONSTANTE del objetivo para siempre (esa deuda no se paga sola), servido por el fallback
    /// «último valor» de `fire_target_at_month_index`.
    #[test]
    fn the_residual_balance_is_a_constant_tail_of_the_target() {
        let mut l = liab(
            Decimal::from(30_000),
            Decimal::from(500),
            RepaymentModel::FixedPayments,
            None,
        );
        // Plan de 10 meses: paga 5.000, quedan 25.000 congelados.
        l.payment_end = Some(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap());
        let serie = debt_payments_remaining_series(std::slice::from_ref(&l), ref_2026());
        let mut ft = ft_flat(Decimal::from(600_000), Decimal::ZERO);
        ft.debt_payments_remaining = serie.clone();
        assert_eq!(
            serie[0],
            Decimal::from(30_000),
            "10 cuotas de 500 + 25.000 residuales"
        );
        // Muy lejos del plan: el término es el residuo, no 0.
        assert_eq!(
            fire_target_at_month_index(Some(&ft), 500),
            Some(Decimal::from(625_000)),
            "la deuda congelada sigue exigiendo capital"
        );
    }

    /// #141 — la jubilación es un estado ABSORBENTE. Escenario del issue: 500.000 € líquidos
    /// al 2 %, nómina 4.000 € / gasto 2.000 € (igual en jubilación, sin ingreso de jubilación),
    /// objetivo 500.000 € con inflación 2 %, 120 meses. El cruce ocurre ya en el mes 1
    /// (nw(0) == target(0)) y el target inflado supera enseguida al patrimonio drenado: bajo la
    /// re-evaluación mensual pre-4.8.0 el hogar «volvía al trabajo» en cada caída y el NW final
    /// salía 609.497,21 € con ~120.000 € de aportado fantasma (el 77 % de sobreestimación del
    /// issue). Con el latch, a mano (50 dígitos): V_k = (V_{k−1} − 2.000)·1,02^(1/12) ⇒
    /// V_120 = **343.865,59 €**, y `contributed_capital` es 0 en TODA la serie — ni un euro de
    /// nómina reinsertado.
    #[test]
    fn retirement_is_an_absorbing_state() {
        let asset = mk_asset(0xF1, Decimal::from(500_000), true, Some(Decimal::from(2)));
        let mut inp = base_input(
            120,
            Decimal::from(4_000),
            Decimal::from(2_000),
            vec![asset],
            vec![rule_remainder(0)],
        );
        inp.phase_plan.income_retirement_monthly = Decimal::ZERO;
        inp.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
        // Como lo cablea el handler: la MISMA inflación en el input (gasto, #139) y en el target.
        inp.annual_inflation_percent = Decimal::from(2);
        inp.fire_target = Some(ft_flat(Decimal::from(500_000), Decimal::from(2)));
        let out = project_net_worth_series(&inp).unwrap();

        // Sanidad del flap: la serie pasa casi todo el horizonte POR DEBAJO del target inflado
        // — exactamente el estado que antes reactivaba la nómina mes a mes.
        let dips = (1..=120u32)
            .filter(|&k| {
                let t = fire_target_at_month_index(inp.fire_target.as_ref(), k).unwrap();
                out.net_worth[k as usize] < t
            })
            .count();
        assert!(
            dips > 100,
            "el escenario debe vivir bajo el target inflado: dips={dips}"
        );

        for k in 0..=120usize {
            // (#120: el activo no tiene purchase_price y el jubilado va en déficit — base 0,
            // drenar 0 de base sigue dando 0. Este bucle pinea el LATCH, no la monotonía de
            // `contributed_capital`, que desde la Ola 6 puede decrecer.)
            assert_eq!(
                out.contributed_capital[k],
                Decimal::ZERO,
                "mes {k}: jubilado = jubilado, nada de nómina reinsertada"
            );
        }
        // Pin movido en la Ola 5 (#139, gasto indexado; antes 343.865,59 con gasto congelado).
        // Con rentabilidad e inflación IGUALES (2 % ambas) hay forma cerrada exacta:
        // V_k = (V_{k−1} − 2.000·m^(k−1))·m  ⟹  V_k/m^k = V_{k−1}/m^(k−1) − 2.000
        //   ⟹  V_120 = 1,02^10 · (500.000 − 2.000·120) = 1,21899441999475713024 × 260.000
        //            = 316.938,549198… → 316.938,55 (exacto en Decimal, sin tolerancia:
        // el exponente 120/12 = 10 normaliza a entero y powd va por checked_powu).
        assert_eq!(
            out.net_worth[120].round_dp(2),
            "316938.55".parse::<Decimal>().unwrap()
        );
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

    /// Regresión (auditoría 2026-08): componer una tasa absurda desbordaba Decimal y PANICABA
    /// (`Multiplication overflowed`), y el pool blocking lo servía como 400 `task_panic`
    /// permanente. Predicción a mano: 1.000 € al 1000 % anual es factor mensual 11^(1/12);
    /// el valor cruza el techo de Decimal (~7,9e28) en el mes k con 1000·11^(k/12) ≈ 7,9e28
    /// ⇒ k ≈ 12·log₁₁(7,9e25) ≈ 298 < 840, así que la simulación DEBE devolver el error
    /// tipado, jamás panicar ni publicar un valor congelado plausible.
    #[test]
    fn absurd_return_overflows_with_typed_error_not_panic() {
        let a = mk_asset(1, Decimal::from(1_000), true, Some(Decimal::from(1_000)));
        let inp = base_input(840, Decimal::ZERO, Decimal::ZERO, vec![a], vec![]);
        let err = project_net_worth_series(&inp).unwrap_err();
        assert_eq!(err, EngineError::AssetValueOverflow);
    }

    /// INVERTIDO en 4.12.1 (#175 — antes: `…_skips_cascade_in_retirement_like_the_loop`, la
    /// regresión H-cascada-1 de la auditoría 2026-08, que pineaba «en jubilación la cascada no
    /// corre y todo va a `surplus_cash`»). La decisión del owner la reescribe entera: la MISMA
    /// cascada corre jubilado o no, y `surplus_cash` murió. A mano: NW(0) = 200.000 ≥ target
    /// 100.000 ⇒ jubilado; caja = 2.200 − 1.600 = 600 ⇒ el sumidero se lleva los 600
    /// (per_asset = [600], leftover = 0, sin skipped_reason). El bucle coincide:
    /// NW(1) = 200.600 — el MISMO número que antes, por el mecanismo contrario (el euro vive en
    /// el activo, no en una caja fantasma) — y la serie por activo lo enseña: 200.600, no
    /// 200.000. Nunca borrar este test.
    #[test]
    fn first_month_allocation_runs_the_cascade_in_retirement_like_the_loop() {
        let a = mk_asset(1, Decimal::from(200_000), true, None);
        let mut inp = base_input(
            2,
            Decimal::from(3_000),
            Decimal::from(1_000),
            vec![a],
            vec![rule_remainder(0)],
        );
        inp.phase_plan.income_retirement_monthly = Decimal::from(2_200);
        inp.phase_plan.expense_retirement_monthly = Decimal::from(1_600);
        inp.fire_target = Some(ft_flat(Decimal::from(100_000), Decimal::ZERO));

        let fma = first_month_allocation(&inp).unwrap();
        assert_eq!(fma.per_asset, vec![Decimal::from(600)]);
        assert_eq!(fma.leftover, Decimal::ZERO);
        assert_eq!(fma.base_cash, Decimal::from(600));
        assert_eq!(fma.rules.len(), 1);
        assert_eq!(fma.rules[0].skipped_reason, None);

        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[1], Decimal::from(200_600));
        assert_eq!(out.per_asset_series[0][1], Decimal::from(200_600));
        assert_eq!(out.unallocated_savings_total, Decimal::ZERO);
    }

    /// Issue #96: el techo del cap se resuelve TAMBIÉN en un mes sin sobrante. A mano:
    /// cap MonthsExpense(6) con gasto+deuda 1.500 ⇒ techo 9.000; activo en 704 ⇒ hueco 8.296.
    /// Antes ambos salían `None` y el llamante duplicaba la fórmula para publicar el techo.
    #[test]
    fn cap_ceiling_is_resolved_even_without_surplus() {
        let rules = vec![rule_fixed(
            0,
            Decimal::from(150),
            Some(AllocationCap::MonthsExpense(Decimal::from(6))),
        )];
        let values = vec![Decimal::from(704)];
        let mut trace = Vec::new();
        // WP5.5: la cascada vive en el núcleo genérico; la regla pública se convierte (copia).
        let rules_g: Vec<crate::sim::AllocationRuleG<Decimal>> = rules
            .iter()
            .map(crate::sim::AllocationRuleG::from)
            .collect();
        let (alloc, leftover) = crate::sim_core::distribute_contributions_g(
            Decimal::ZERO,
            &rules_g,
            &values,
            Decimal::from(1_500),
            Decimal::from(3_000),
            Some(&mut trace),
        );
        assert_eq!(alloc, vec![Decimal::ZERO]);
        assert_eq!(leftover, Decimal::ZERO);
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].skipped_reason, Some(AllocationSkipReason::NoCash));
        assert_eq!(trace[0].cap_ceiling, Some(Decimal::from(9_000)));
        assert_eq!(trace[0].cap_room, Some(Decimal::from(8_296)));
    }

    /// #119: el mes de agotamiento con números a mano. 30.000 € líquidos al 0 %, ingreso 1.000,
    /// gasto 2.500 ⇒ déficit 1.500/mes. Tras el mes 19 quedan 30.000 − 19×1.500 = 1.500 €; en el
    /// mes 20 need (1.500) == drenable (1.500) ⇒ la cartera se VACÍA en el 20 (caso exacto, por
    /// eso `>=`), el descubierto empieza en el 21, y al mes 60 acumula 40 × 1.500 = 60.000 €
    /// (NW(60) = −60.000, ya pineado por el arnés audit_dump como P1).
    #[test]
    fn depletion_month_and_uncovered_deficit_are_reported() {
        let a = mk_asset(1, Decimal::from(30_000), true, None);
        let inp = base_input(
            60,
            Decimal::from(1_000),
            Decimal::from(2_500),
            vec![a],
            vec![],
        );
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.assets_depleted_month_index, Some(20));
        assert_eq!(out.uncovered_deficit_total, Decimal::from(60_000));
        assert_eq!(out.net_worth[60], Decimal::from(-60_000));

        // Control: con horizonte 19 no llega a agotarse — `None` significa «no en el horizonte».
        let corto = base_input(
            19,
            Decimal::from(1_000),
            Decimal::from(2_500),
            vec![mk_asset(1, Decimal::from(30_000), true, None)],
            vec![],
        );
        let out = project_net_worth_series(&corto).unwrap();
        assert_eq!(out.assets_depleted_month_index, None);
        assert_eq!(out.uncovered_deficit_total, Decimal::ZERO);
    }

    #[test]
    fn no_rules_strands_the_surplus_entirely() {
        // INVERTIDO en 4.12.1 (antes: `no_rules_routes_surplus_to_cash`, que pineaba «el
        // sobrante queda como surplus_cash y entra al NW»). Decisión 3 del owner: el euro sin
        // regla que lo destine NO se simula — fuera del balance, solo CUANTIFICADO. En
        // producción este estado es inalcanzable (siembra + sumidero indestructible #176);
        // el motor, función pura, lo define así. Nunca borrar este test.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(3, Decimal::from(3000), Decimal::from(1000), vec![a], vec![]);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::ZERO);
        assert_eq!(out.net_worth[1], Decimal::ZERO);
        assert_eq!(out.net_worth[2], Decimal::ZERO);
        assert_eq!(out.net_worth[3], Decimal::ZERO);
        assert_eq!(out.unallocated_savings_total, Decimal::from(6000));
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
            vec![rule_fixed(0, Decimal::from(200), None), rule_remainder(1)],
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
                rule_percent(
                    0,
                    Decimal::from(100),
                    Some(AllocationCap::Amount(Decimal::from(1000))),
                ),
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
                rule_percent(
                    0,
                    Decimal::from(100),
                    Some(AllocationCap::Amount(Decimal::from(1000))),
                ),
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
                rule_percent(
                    0,
                    Decimal::from(100),
                    Some(AllocationCap::MonthsExpense(Decimal::from(2))),
                ),
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
        // R1: fija 300 a A (cap 500). R2: remainder CON TOPE a A (cap 500) — un remainder con
        // tope no es un sumidero. Sobrante 1000. R1 pone 300, A=300, room=200. R2 quiere todo
        // (700) pero room=200 → 200. Quedan 500 sin regla que los destine: desde 4.12.1 NO
        // entran al balance (decisión 3) — solo se cuantifican.
        let a = mk_asset(1, Decimal::ZERO, true, None);
        let inp = base_input(
            1,
            Decimal::from(1000),
            Decimal::ZERO,
            vec![a],
            vec![
                rule_fixed(
                    0,
                    Decimal::from(300),
                    Some(AllocationCap::Amount(Decimal::from(500))),
                ),
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
        // A llega a 500 y ahí termina el balance; los 500 varados se declaran aparte.
        assert_eq!(out.net_worth[1], Decimal::from(500));
        assert_eq!(out.unallocated_savings_total, Decimal::from(500));
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
            rule_fixed(
                0,
                Decimal::from(150),
                Some(AllocationCap::MonthsExpense(Decimal::from(6))),
            ),
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
            min_payment_pct: None,
            min_payment_eur: None,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
            early_repayment_fee_pct: None,
            early_repayment_effect: EarlyRepaymentEffect::default(),
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
        assert_eq!(
            b.recurring_net, a.recurring_net,
            "la parte estable no se mueve"
        );
        assert_eq!(b.base_cash, Decimal::from(1743));
        assert_eq!(b.base_cash, b.recurring_net + b.planning_component);
        assert_eq!(
            b.per_asset.iter().sum::<Decimal>() + b.leftover,
            b.base_cash
        );
        // El wrapper de compatibilidad devuelve exactamente `per_asset`.
        assert_eq!(
            first_month_per_asset_contribution_nominals(&inp).unwrap(),
            b.per_asset
        );
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
            rule_fixed(
                0,
                Decimal::from(100),
                Some(AllocationCap::Amount(Decimal::from(1000))),
            ),
            rule_fixed(1, Decimal::ZERO, None),
            rule_fixed(2, Decimal::from(500), None),
            rule_remainder(3),
        ];
        let inp = base_input(1, Decimal::from(500), Decimal::ZERO, assets, rules);
        let a = first_month_allocation(&inp).unwrap();

        assert_eq!(
            a.rules.len(),
            4,
            "se emite una traza por regla, también las saltadas"
        );
        assert_eq!(
            a.rules[0].skipped_reason,
            Some(AllocationSkipReason::CapFull)
        );
        assert_eq!(a.rules[0].cap_ceiling, Some(Decimal::from(1000)));
        assert_eq!(a.rules[0].cap_room, Some(Decimal::ZERO));
        assert_eq!(
            a.rules[1].skipped_reason,
            Some(AllocationSkipReason::ZeroAmount)
        );
        assert_eq!(a.rules[2].skipped_reason, None);
        assert_eq!(a.rules[2].amount_resolved, Decimal::from(500));
        // La caja se agotó en la regla 2: la 3 nunca llegó a evaluarse. `NotReached` y `NoCash` son
        // diagnósticos distintos («las de arriba se lo comieron» vs «no te sobra dinero») y por eso
        // no se colapsan.
        assert_eq!(
            a.rules[3].skipped_reason,
            Some(AllocationSkipReason::NotReached)
        );
        assert_eq!(
            a.per_asset.iter().sum::<Decimal>() + a.leftover,
            a.base_cash
        );
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
        assert_eq!(
            a.leftover,
            Decimal::ZERO,
            "con caja negativa no hay sobrante"
        );
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
            rule_fixed(
                0,
                Decimal::from(500),
                Some(AllocationCap::Amount(Decimal::from(1000))),
            ),
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
        inp.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(3);
        inp.phase_plan.extra_monthly_withdrawal = Decimal::from(1_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(12_000));
        assert_eq!(out.net_worth[1], Decimal::from(12_000));
        assert_eq!(out.net_worth[2], Decimal::from(12_000));
        assert_eq!(out.net_worth[3], Decimal::from(11_000));
        assert_eq!(out.net_worth[4], Decimal::from(10_000));
        // 5.0.0 WP1b — ANCLA derivada a mano de la serie de retirada: el mes 0 no es un mes
        // simulado, los meses 1–2 no están jubilados y los meses 3–4 retiran la retirada extra
        // entera (no hay más déficit: ingreso y gasto son 0). Es la misma caída de 1.000 €/mes
        // que el patrimonio de arriba enseña, vista desde la otra cara.
        assert_eq!(
            out.withdrawal,
            vec![
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::from(1_000),
                Decimal::from(1_000)
            ]
        );
        assert_eq!(out.retirement_month_index, Some(3));
        assert_eq!(
            out.liquid_crossing_month_index, None,
            "sin objetivo FIRE no hay cruce que leer: la jubilación es forzada"
        );
        assert_eq!(
            out.phase_transitions,
            vec![(Phase::Accumulating, 0), (Phase::Retired, 3)]
        );
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
        inp.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(3);
        inp.phase_plan.income_retirement_monthly = Decimal::from(500);
        inp.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[0], Decimal::from(10_000));
        assert_eq!(out.net_worth[1], Decimal::from(11_000));
        assert_eq!(out.net_worth[2], Decimal::from(12_000));
        assert_eq!(out.net_worth[3], Decimal::from(10_500));
        assert_eq!(out.net_worth[4], Decimal::from(9_000));
        // 5.0.0 WP1b: meses 1–2 aportando 1.000 (sin retirada), meses 3–5 jubilado con déficit
        // 500 − 2.000 = −1.500 €/mes. La retirada es EL déficit, no el gasto.
        assert_eq!(out.withdrawal[1], Decimal::ZERO);
        assert_eq!(out.withdrawal[2], Decimal::ZERO);
        assert_eq!(out.withdrawal[3], Decimal::from(1_500));
        assert_eq!(out.withdrawal[4], Decimal::from(1_500));
        assert_eq!(out.retirement_month_index, Some(3));
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
        inp.fire_target = Some(ft_flat(Decimal::from(50_000), Decimal::from(10)));
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
        let ft = ft_flat(Decimal::from(750_000), Decimal::from(3));
        let t0 = fire_target_at_month_index(Some(&ft), 0).unwrap();
        assert_eq!(t0, Decimal::from(750_000));
        let t20y = fire_target_at_month_index(Some(&ft), 240).unwrap();
        // 750_000 × 1.03^20 ≈ 1_354_583. Comprobamos con tolerancia ≤ 1€.
        let factor =
            (Decimal::ONE + Decimal::from(3) / Decimal::from(100u32)).powd(Decimal::from(20u32));
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
        let ft = ft_flat(Decimal::from(500_000), Decimal::ZERO);
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
        let ft = ft_flat(Decimal::from(100_000), Decimal::from(5));
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
            vec![mk_asset(
                1,
                Decimal::from(1_000_000),
                true,
                Some(Decimal::ZERO),
            )],
            vec![rule_remainder(0)],
        );
        inp.phase_plan.expense_retirement_monthly = Decimal::from(1000);
        inp.fire_target = Some(ft_flat(
            Decimal::from_str_exact("342857.142857").unwrap(),
            Decimal::ZERO,
        ));

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
        assert_eq!(
            alloc2.per_asset[0],
            Decimal::from(2000),
            "sin target FIRE la cascada es la de siempre"
        );
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
    // Ola 5 (#139): el input de este pin se queda DELIBERADAMENTE con `annual_inflation_percent`
    // a 0 (el default de `base_input`): su propósito es la estabilidad bit a bit de la MATEMÁTICA
    // DE PASIVOS respecto de pre-4.2.0, y congelar el gasto aísla ese eje. La combinación
    // «target inflado + gasto congelado» ya no es alcanzable desde el handler — aquí es un
    // instrumento de aislamiento, no un contrato de producto.
    fn liability_pin_input() -> ProjectionInput {
        let assets = vec![
            mk_asset(0xA1, Decimal::from(50_000), true, Some(Decimal::from(7))),
            mk_asset(
                0xB2,
                Decimal::from(20_000),
                false,
                Some(Decimal::new(35, 1)),
            ),
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
            min_payment_pct: None,
            min_payment_eur: None,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
            early_repayment_fee_pct: None,
            early_repayment_effect: EarlyRepaymentEffect::default(),
        }];
        inp.planning_monthly_cash_adjustment[0] = Decimal::from(250);
        inp.planning_monthly_cash_adjustment[5] = Decimal::from(-100);
        inp.fire_target = Some(ft_flat(Decimal::from(1_500_000), Decimal::new(25, 1)));
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
        assert_eq!(
            out.net_worth[0],
            dec("-30000"),
            "mes 0 = 50.000 + 20.000 − 100.000"
        );
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
        assert_eq!(
            principal_at(1),
            Decimal::from(99_500),
            "cuota íntegra a principal: 0 % interés"
        );
        assert_eq!(
            principal_at(199),
            Decimal::from(500),
            "queda la última cuota"
        );
        assert_eq!(
            principal_at(200),
            Decimal::ZERO,
            "el pasivo se extingue en el mes 200"
        );
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
        assert_eq!(
            alloc.per_asset,
            vec![Decimal::from(300), Decimal::from(1950)]
        );
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
            min_payment_pct: None,
            min_payment_eur: None,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
            early_repayment_fee_pct: None,
            early_repayment_effect: EarlyRepaymentEffect::default(),
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
        assert_eq!(
            a.net_worth, b.net_worth,
            "el TIN no debe mover fixed_payments"
        );
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
        assert_eq!(
            implicit_principal(&out, 279),
            Decimal::ZERO,
            "y no resucita"
        );

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

    /// INVERTIDO en la Ola 3 (#144). Hasta 4.6.0 este test pineaba «la caja es la cuota
    /// declarada, entera» — sobre una deuda de 300 € con cuota 500 salían 300 €/mes, y con
    /// cuota 300 €/mes durante 70 años salían 252.000 € de «interés» sobre 300 € de deuda.
    /// La carencia real española cobra el interés del período: `cash = min(M, P·i)`.
    ///
    /// Números a mano (80.000 € al 6 % ⇒ i = 0,005; interés = 400,00 €/mes exacto):
    /// - cuota declarada 400 = interés ⇒ caja 400, principal plano;
    /// - cuota declarada 600 > interés ⇒ caja 400 igualmente (la cuota es tope, no suelo);
    /// - deuda 300 € al 6 % ⇒ interés 1,50 €/mes; cuota declarada 500 ⇒ caja 1,50, no 300.
    #[test]
    fn interest_only_cash_is_the_period_interest_not_the_declared_quota() {
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
                "con cuota = interés el principal es constante (mes {k})"
            );
        }
        assert_eq!(
            first_month_allocation(&inp).unwrap().debt_service,
            Decimal::from(400),
            "la caja del mes es el interés del período"
        );

        // Cuota por ENCIMA del interés: la caja se recorta al interés (600 → 400,00). En
        // carencia no se amortiza pagando de más — eso es `extra_principal_monthly`.
        let generoso = one_liability_input(
            1,
            Decimal::from(80_000),
            Decimal::from(600),
            RepaymentModel::InterestOnly,
            Some(Decimal::from(6)),
        );
        assert_eq!(
            first_month_allocation(&generoso).unwrap().debt_service,
            Decimal::from(400)
        );

        // La deuda pequeña que motivó la inversión: 300 € al 6 % ⇒ 1,50 €/mes, no 300.
        let pequeno = one_liability_input(
            1,
            Decimal::from(300),
            Decimal::from(500),
            RepaymentModel::InterestOnly,
            Some(Decimal::from(6)),
        );
        assert_eq!(
            first_month_allocation(&pequeno).unwrap().debt_service,
            dec("1.500"),
        );
    }

    /// Cuota (tope) por DEBAJO del interés en carencia: el déficit capitaliza. 100.000 € al
    /// 12 % ⇒ devengo 1.000 €/mes, tope declarado 400. A mano:
    /// mes 1: caja 400, cierre 100.000 + 1.000 − 400 = 100.600;
    /// mes 2: interés 1.006, caja 400, cierre 100.600 + 1.006 − 400 = 101.206.
    #[test]
    fn interest_only_deficit_capitalizes_like_a_real_carencia() {
        let inp = one_liability_input(
            2,
            Decimal::from(100_000),
            Decimal::from(400),
            RepaymentModel::InterestOnly,
            Some(Decimal::from(12)),
        );
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(implicit_principal(&out, 1), dec("100600.00"));
        assert_eq!(implicit_principal(&out, 2), dec("101206.0000"));
    }

    /// INVERTIDO en la Ola 3 (#144). El test 4.2.0 `revolving_matches_french_recurrence` pineaba
    /// «revolving = etiqueta de la recurrencia francesa» y su propio comentario anunciaba que
    /// moriría a propósito el día que revolving modelara lo suyo. Ese día es este: la cuota
    /// mínima real es `max(pct × saldo de apertura, suelo €)` — lo que queda de la equivalencia
    /// es el caso degenerado `pct = 0, suelo = cuota declarada`, que es EXACTAMENTE el backfill
    /// de la migración y por eso se pinea bit-idéntico.
    #[test]
    fn revolving_backfill_shape_degenerates_to_french_bit_identical() {
        let frances = one_liability_input(
            120,
            Decimal::from(12_000),
            Decimal::from(250),
            RepaymentModel::French,
            Some(Decimal::from(18)),
        );
        let mut revolving = one_liability_input(
            120,
            Decimal::from(12_000),
            Decimal::from(250),
            RepaymentModel::Revolving,
            Some(Decimal::from(18)),
        );
        revolving.liabilities[0].min_payment_pct = Some(Decimal::ZERO);
        revolving.liabilities[0].min_payment_eur = Some(Decimal::from(250));
        let a = project_net_worth_series(&frances).unwrap();
        let b = project_net_worth_series(&revolving).unwrap();
        assert_eq!(a.net_worth, b.net_worth);
        assert_eq!(a.contributed_capital, b.contributed_capital);
        assert_eq!(a.per_asset_series, b.per_asset_series);
    }

    /// La cuota mínima revolving de verdad (#144): `max(pct × saldo de apertura, suelo €)`,
    /// topada al payoff. A mano, TIN 18 % ⇒ i = 0,015, pct 3 %, suelo 30 €:
    /// - saldo 3.000 ⇒ max(90, 30) = 90,00; cierre = 3.000·1,015 − 90 = 2.955,00;
    /// - saldo 800 ⇒ max(24, 30) = 30,00 (manda el suelo); cierre = 800·1,015 − 30 = 782,00.
    /// La cuota DECLARADA no pinta nada en la caja: se declara 999 y salen 90.
    #[test]
    fn revolving_minimum_is_pct_of_opening_balance_with_a_floor() {
        let mut grande = one_liability_input(
            1,
            Decimal::from(3_000),
            Decimal::from(999),
            RepaymentModel::Revolving,
            Some(Decimal::from(18)),
        );
        grande.liabilities[0].min_payment_pct = Some(Decimal::from(3));
        grande.liabilities[0].min_payment_eur = Some(Decimal::from(30));
        assert_eq!(
            first_month_allocation(&grande).unwrap().debt_service,
            dec("90.00")
        );
        let out = project_net_worth_series(&grande).unwrap();
        assert_eq!(implicit_principal(&out, 1), dec("2955.000"));

        let mut pequeno = one_liability_input(
            1,
            Decimal::from(800),
            Decimal::from(999),
            RepaymentModel::Revolving,
            Some(Decimal::from(18)),
        );
        pequeno.liabilities[0].min_payment_pct = Some(Decimal::from(3));
        pequeno.liabilities[0].min_payment_eur = Some(Decimal::from(30));
        assert_eq!(
            first_month_allocation(&pequeno).unwrap().debt_service,
            dec("30")
        );
        let out = project_net_worth_series(&pequeno).unwrap();
        assert_eq!(implicit_principal(&out, 1), dec("782.000"));
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
        assert_eq!(
            p1 - p2,
            Decimal::from(500),
            "saturado, la cuota va a principal"
        );
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
            min_payment_pct: None,
            min_payment_eur: None,
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
            early_repayment_fee_pct: None,
            early_repayment_effect: EarlyRepaymentEffect::default(),
        }
    }

    fn dec_s(v: &str) -> Decimal {
        v.parse().unwrap()
    }

    fn ref_2026() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()
    }

    /// #151, números a mano (francés 150.000 € al TIN 2,50 % ⇒ i = 1/480, cuota 800, lump de
    /// 20.000 € en el mes 12, comisión 2 %):
    /// - comisión del mes 12 = 20.000 × 2 % = **400,00 €** exactos, FUERA de la identidad
    ///   cuota+extra = interés+amortizado (coste puro, no baja el principal);
    /// - `total_cash_out` = cuotas + extra + comisión.
    #[test]
    fn early_repayment_fee_is_charged_outside_the_amortization_identity() {
        let mut l = liab(
            Decimal::from(150_000),
            Decimal::from(800),
            RepaymentModel::French,
            Some(dec("2.5")),
        );
        l.extra_principal_lump_sums = vec![(12, Decimal::from(20_000))];
        l.early_repayment_fee_pct = Some(Decimal::from(2));
        let sch = liability_amortization_schedule(&l, ref_2026(), 480);

        let m12 = sch.months.iter().find(|m| m.month_index == 12).unwrap();
        assert_eq!(m12.extra_principal, Decimal::from(20_000));
        assert_eq!(
            m12.early_repayment_fee,
            Decimal::from(400),
            "20.000 × 2 % = 400,00"
        );
        assert_eq!(
            m12.payment + m12.extra_principal,
            m12.interest_accrued + m12.principal_repaid,
            "la identidad se cumple SIN la comisión"
        );
        assert_eq!(sch.total_early_repayment_fee, Decimal::from(400));
        assert_eq!(
            sch.total_cash_out,
            sch.total_payments + sch.total_extra_principal + sch.total_early_repayment_fee
        );
        // Los meses sin extra no pagan comisión.
        assert!(sch
            .months
            .iter()
            .filter(|m| m.month_index != 12)
            .all(|m| m.early_repayment_fee.is_zero()));
    }

    /// #151 «reducir cuota» — LA INVARIANTE, que no depende de aritmética a mano: en una renta
    /// francesa el plazo restante depende solo del cociente P·i/M, así que λ-escalar la cuota
    /// por el factor que bajó el principal conserva EXACTAMENTE el mes de extinción del
    /// préstamo SIN amortizar. A mano (verificado con la recurrencia a 50 dígitos): baseline
    /// sin extra extingue en el mes **239**; con 20.000 € en el mes 12 y `reduce_payment`,
    /// TAMBIÉN en el 239 — y la cuota baja de 800 a **688,9525 €** (caja liberada 111,0475 €/mes).
    #[test]
    fn reduce_payment_keeps_the_payoff_month_and_lowers_the_instalment() {
        let base = liab(
            Decimal::from(150_000),
            Decimal::from(800),
            RepaymentModel::French,
            Some(dec("2.5")),
        );
        let base_sch = liability_amortization_schedule(&base, ref_2026(), 480);
        assert_eq!(base_sch.payoff_month_index, Some(239), "baseline a mano");

        let mut reduced = base.clone();
        reduced.extra_principal_lump_sums = vec![(12, Decimal::from(20_000))];
        reduced.early_repayment_effect = EarlyRepaymentEffect::ReducePayment;
        let sch = liability_amortization_schedule(&reduced, ref_2026(), 480);
        assert_eq!(
            sch.payoff_month_index, base_sch.payoff_month_index,
            "reducir cuota conserva el mes de extinción"
        );
        let m13 = sch.months.iter().find(|m| m.month_index == 13).unwrap();
        let cuota = m13.payment.round_dp(4);
        assert_eq!(cuota, dec("688.9525"), "cuota nueva desde el mes 13");

        // El gemelo: con el efecto default (acortar plazo) la cuota NO cambia y el préstamo
        // acaba antes — a mano, en el mes **200**.
        let mut shortened = base.clone();
        shortened.extra_principal_lump_sums = vec![(12, Decimal::from(20_000))];
        let sch = liability_amortization_schedule(&shortened, ref_2026(), 480);
        assert_eq!(sch.payoff_month_index, Some(200), "acortar plazo, a mano");
        let m13 = sch.months.iter().find(|m| m.month_index == 13).unwrap();
        assert_eq!(m13.payment, Decimal::from(800), "la cuota no se toca");
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
                assert!(
                    !sch.months.is_empty(),
                    "{model:?}/{apr:?}: calendario vacío"
                );
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
        assert_eq!(sch.months[10].payment, dec("58.9848800121510040091000"));
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
        assert_eq!(
            s1.payoff_absent,
            Some(LiabilityPayoffAbsence::NoPaymentPlan)
        );
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
        assert!(
            s3b.final_principal > Decimal::from(100_000),
            "la deuda crece"
        );

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
        assert_eq!(
            sch.payoff_absent,
            Some(LiabilityPayoffAbsence::NoPaymentPlan)
        );
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
            present_value_of_payments(
                Decimal::from(500),
                Decimal::from(200),
                Some(Decimal::from(-2))
            ),
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

    /// INVERTIDO en la Ola 5 (#139) — el baseline congelado cruzaba en el mes **386** (número
    /// del issue, reproducido exacto). Con el gasto indexado a la inflación e ingresos PLANOS
    /// (la decisión firmada; el «335» que anunciaba el issue era la alternativa RECHAZADA de
    /// indexarlo todo — corrección publicada en el issue el 2026-08-31), este hogar **no cruza
    /// en 840 meses**: la aportación real decrece cada mes y la caja entra en déficit en el mes
    /// **247** (forma cerrada: k−1 > 12·ln(1,5)/ln(1,02) = 245,70). Patrimonio final ≈
    /// 1.549.432,92 (réplica a 50 dígitos), lejos del objetivo (~2,4 M€ inflado a 70 años).
    #[test]
    fn indexed_expense_postpones_the_crossing_beyond_the_horizon() {
        let fondo = mk_asset(0xB1, Decimal::ZERO, true, Some(Decimal::from(6)));
        let mut inp = base_input(
            840,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![fondo],
            vec![rule_remainder(0)],
        );
        inp.annual_inflation_percent = Decimal::from(2);
        inp.fire_target = Some(ft_flat(Decimal::from(600_000), Decimal::from(2)));
        let out = project_net_worth_series(&inp).unwrap();
        let cruce = (0..=840u32).find(|&m| {
            fire_target_at_month_index(inp.fire_target.as_ref(), m)
                .is_some_and(|t| out.liquid_worth[m as usize] >= t)
        });
        assert_eq!(
            cruce, None,
            "con gasto indexado e ingresos planos no hay cruce en 70 años"
        );

        // Primer déficit de caja en el mes 247: la aportación del 246 aún entra; desde el 247
        // el déficit se cubre VENDIENDO — y con #120 (Ola 6) la base de coste baja con la
        // venta, así que `contributed_capital` DECRECE (hasta 4.9.0 se congelaba: la serie
        // era monótona por construcción y este assert pineaba la igualdad).
        assert!(
            out.contributed_capital[246] > out.contributed_capital[245],
            "el mes 246 aún aporta"
        );
        assert!(
            out.contributed_capital[247] < out.contributed_capital[246],
            "desde el 247 se vende y la base aportada baja (#120)"
        );
        let final_ = out.net_worth[840].round_dp(2);
        assert!(
            (final_ - dec_s("1549432.92")).abs() < Decimal::ONE,
            "patrimonio final ≈1.549.432,92, got {final_}"
        );
    }

    /// INVERTIDO en la Ola 5 (#139) — el baseline congelado daba NW(K) = 1.000·K exacto.
    /// Escenario mínimo del spike §2.2: ingreso 3.000, gasto 2.000, activo al 0 % (multiplicador
    /// 1), inflación 3 %. Forma cerrada con q = 1,03^(1/12) (exponente (k−1)/12):
    /// NW(K) = 3.000·K − 2.000·(1,03^(K/12) − 1)/(q − 1), verificada a 50 dígitos contra el
    /// bucle: NW(12) = 11.671,7611861425; NW(24) = 22.613,6752078692;
    /// NW(120) = 81.104,0063772995. Tolerancia en céntimos: los sumandos individuales pasan por
    /// `powd` con exponentes fraccionarios (los pins existentes del engine hacen lo mismo).
    #[test]
    fn the_expense_is_indexed_to_installation_inflation() {
        let hucha = mk_asset(0xB2, Decimal::ZERO, true, None);
        let mut inp = base_input(
            120,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![hucha],
            vec![rule_remainder(0)],
        );
        inp.annual_inflation_percent = Decimal::from(3);
        let out = project_net_worth_series(&inp).unwrap();
        for (k, esperado) in [
            (12usize, "11671.7611861425"),
            (24, "22613.6752078692"),
            (120, "81104.0063772995"),
        ] {
            let got = out.net_worth[k];
            let want = dec_s(esperado);
            assert!(
                (got - want).abs() < dec_s("0.01"),
                "NW({k}): esperado {want}, obtenido {got}"
            );
        }
        // El mes 1 cobra el gasto BASE tal cual (f(1) = 1): lo que el usuario teclea no se mueve.
        assert_eq!(out.net_worth[1], Decimal::from(1_000), "mes 1 sin inflar");
    }

    /// #139, efecto de segundo orden ahora pineado (antes «known, unpinned behavior»): el techo
    /// de un cap `months_expense` CRECE con la inflación — «6 meses de gasto» son 6 meses del
    /// gasto REAL del mes corriente. Activo arrancando exactamente en su techo del mes 1
    /// (6 × 1.000 = 6.000): con inflación 0 la regla no vuelve a recibir nunca; con 3 % el techo
    /// sube cada mes y la regla sigue rellenando el colchón.
    #[test]
    fn months_expense_ceiling_grows_with_inflation() {
        let build = |inflacion: Decimal| {
            let colchon = mk_asset(0xC1, Decimal::from(6_000), true, None);
            let mut inp = base_input(
                24,
                Decimal::from(3_000),
                Decimal::from(1_000),
                vec![colchon],
                vec![rule_fixed(
                    0,
                    Decimal::from(500),
                    Some(AllocationCap::MonthsExpense(Decimal::from(6))),
                )],
            );
            inp.annual_inflation_percent = inflacion;
            project_net_worth_series(&inp).unwrap()
        };
        let plano = build(Decimal::ZERO);
        let inflado = build(Decimal::from(3));
        assert_eq!(
            plano.per_asset_series[0][24],
            Decimal::from(6_000),
            "sin inflación el techo es fijo y el colchón no crece"
        );
        // 4.12.1 (decisión 3): cascada de solo-topes ya llena ⇒ CapFull todos los meses y los
        // 2.000/mes × 24 quedan varados — cuantificados, fuera del balance.
        assert_eq!(plano.unallocated_savings_total, Decimal::from(48_000));
        assert!(
            inflado.per_asset_series[0][24] > Decimal::from(6_000),
            "con inflación el techo sube y la regla sigue rellenando: {}",
            inflado.per_asset_series[0][24]
        );
    }

    /// #146 (Ola 5): una inflación NEGATIVA hace DECRECER el objetivo — hasta 4.8.0 la rama
    /// `<= ZERO` lo aplanaba en silencio. Pins exactos (múltiplos de 12 ⇒ exponente entero ⇒
    /// `checked_powu`, sin `exp`/`ln`): base 863.652,80 € a −2 % anual:
    /// t(12) = 863.652,80 × 0,98 = 846.379,7440; t(120) = 863.652,80 × 0,98^10
    /// = 705.667,2174722891568870686720 (0,98^10 = 0,81707280688754689024, 20 decimales exactos;
    /// el producto cabe en los 28 dígitos de Decimal sin redondear).
    #[test]
    fn negative_inflation_shrinks_the_target_instead_of_flattening_it() {
        let ft = ft_flat(dec("863652.80"), Decimal::from(-2));
        let t = |m: u32| fire_target_at_month_index(Some(&ft), m).unwrap();
        assert_eq!(t(0), dec("863652.80"), "mes 0: la base tal cual");
        assert_eq!(t(12), dec_s("846379.7440"), "un año a −2 %: ×0,98 exacto");
        assert_eq!(
            t(120),
            dec_s("705667.2174722891568870686720"),
            "diez años: ×0,98^10, exacto por checked_powu"
        );
        assert!(
            t(1) < t(0) && t(6) < t(1) && t(13) < t(12),
            "estrictamente decreciente"
        );
    }

    /// #171 (Ola 6): la traza de `first_month_allocation` resuelve los techos con los escalares
    /// de la FASE del mes — no con los regulares. Cuatro casos, dos ramas (patrón
    /// context_fields): activo de 3.000, cuota de pasivo 500 (fixed, sin TIN, principal holgado),
    /// gasto regular 2.000 / de jubilación 1.500, ingreso regular 3.000 / de jubilación 2.500
    /// (superávit ⇒ rama `InRetirement`) o 1.000 (déficit ⇒ rama `else`, `NoCash`).
    /// months_expense(6): 6·(2.000+500) = 15.000 sin jubilar; 6·(1.500+500) = 12.000 jubilado
    /// (hoy publicaba 15.000 — la mentira). income_multiple(4): 4·3.000 = 12.000 sin jubilar;
    /// 4·1.000 = 4.000 jubilado con déficit (hoy 12.000, factor 3). Desde 4.12.1 (#175) la
    /// cascada corre TAMBIÉN jubilada, así que estos techos ya GOBIERNAN euros de verdad — la
    /// sustancia de #171 (los escalares de la fase) sobrevive intacta; lo que muere es que
    /// fueran solo explicativos.
    #[test]
    fn the_retirement_trace_resolves_caps_with_the_months_budget() {
        let build = |retired: bool, income_ret: Decimal, cap: AllocationCap| {
            let fondo = mk_asset(0xD1, Decimal::from(3_000), true, None);
            let mut inp = base_input(
                12,
                Decimal::from(3_000),
                Decimal::from(2_000),
                vec![fondo],
                vec![AllocationRule {
                    target_index: 0,
                    kind: AllocationKind::Remainder,
                    amount: None,
                    cap: Some(cap),
                }],
            );
            inp.liabilities = vec![liab(
                Decimal::from(100_000),
                Decimal::from(500),
                RepaymentModel::FixedPayments,
                None,
            )];
            inp.phase_plan.expense_retirement_monthly = Decimal::from(1_500);
            inp.phase_plan.income_retirement_monthly = income_ret;
            if retired {
                inp.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
            }
            first_month_allocation(&inp).unwrap()
        };
        let me6 = || AllocationCap::MonthsExpense(Decimal::from(6));
        let im4 = || AllocationCap::IncomeMultiple(Decimal::from(4));

        // A · sin jubilar, months_expense(6): techo 15.000, hueco 12.000.
        let a = build(false, Decimal::ZERO, me6());
        assert_eq!(a.rules[0].cap_ceiling, Some(Decimal::from(15_000)));
        assert_eq!(a.rules[0].cap_room, Some(Decimal::from(12_000)));

        // B · jubilado con SUPERÁVIT: desde 4.12.1 la cascada CORRE (500 de superávit al
        // sumidero, sin skipped_reason) y los techos de la fase lo gobiernan: 12.000 / 9.000.
        let b = build(true, Decimal::from(2_500), me6());
        assert_eq!(b.rules[0].skipped_reason, None);
        assert_eq!(b.rules[0].cap_ceiling, Some(Decimal::from(12_000)));
        assert_eq!(b.rules[0].cap_room, Some(Decimal::from(9_000)));

        // B' · jubilado con DÉFICIT (rama else, NoCash): mismos techos de jubilación.
        let b2 = build(true, Decimal::from(1_000), me6());
        assert_eq!(
            b2.rules[0].skipped_reason,
            Some(AllocationSkipReason::NoCash)
        );
        assert_eq!(b2.rules[0].cap_ceiling, Some(Decimal::from(12_000)));
        assert_eq!(b2.rules[0].cap_room, Some(Decimal::from(9_000)));

        // C/D · income_multiple(4): 12.000 sin jubilar; 4.000 jubilado (déficit).
        let c = build(false, Decimal::ZERO, im4());
        assert_eq!(c.rules[0].cap_ceiling, Some(Decimal::from(12_000)));
        let d = build(true, Decimal::from(1_000), im4());
        assert_eq!(d.rules[0].cap_ceiling, Some(Decimal::from(4_000)));
        assert_eq!(d.rules[0].cap_room, Some(Decimal::from(1_000)));
    }

    /// #120 (Ola 6) — la base baja proporcionalmente al VALOR drenado, por activo. A (10.000,
    /// base 4.000) se vacía entero → base 0 exacto; B (20.000, base 15.000) vende 5.000 →
    /// base 15.000·15.000/20.000 = 11.250. `contributed_capital` pasa de 19.000 a **11.250**
    /// (hasta 4.9.0 se quedaba en 19.000: la serie era monótona por construcción) — y ESO es
    /// también el guard-rail de contrato: la serie PUEDE decrecer. Plusvalía realizada del mes
    /// = 15.000 − 7.750 = 7.250 (la que la fase 2 de #140 querría gravar cuando g se derive).
    #[test]
    fn basis_falls_proportionally_to_the_value_drained_per_asset() {
        let mut a = mk_asset(0xE1, Decimal::from(10_000), true, None);
        a.purchase_price = Some(Decimal::from(4_000));
        let mut b = mk_asset(0xE2, Decimal::from(20_000), true, None);
        b.purchase_price = Some(Decimal::from(15_000));
        let inp = base_input(1, Decimal::ZERO, Decimal::from(15_000), vec![a, b], vec![]);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::from(19_000));
        assert_eq!(out.contributed_capital[1], Decimal::from(11_250));
        assert_eq!(out.net_worth[1], Decimal::from(15_000));
        assert!(
            out.contributed_capital[1] < out.contributed_capital[0],
            "la serie deja de ser monótona: vender BAJA lo aportado"
        );
        assert_eq!(
            out.per_asset_series[0][1],
            Decimal::ZERO,
            "A se vació entero"
        );
        assert_eq!(out.per_asset_series[1][1], Decimal::from(15_000));
    }

    /// INVERTIDO en 4.12.1 (antes: `retirement_surplus_counts_as_contributed`, el escenario
    /// H-capital-10 de #120, que pineaba «el sobrante jubilado va a surplus_cash y cuenta como
    /// aportado»). Con `surplus_cash` muerto y SIN NINGUNA REGLA, el superávit de pensión no
    /// tiene destino: no compone, no es aportado, no entra al NW — solo se cuantifica
    /// (24 × 1.000 = 24.000, decisión 3 del owner; el titular es que los euros NO desaparecen
    /// sin decirlo). El activo compone intocado: 1.000.000·1,05² = 1.102.500, y eso ES ahora
    /// el NW entero. El caso CON regla (el de producción, donde el superávit sí compone —
    /// 180.000 → 409.348,92 en el ancla del CHANGELOG) vive en
    /// `retirement_surplus_runs_the_users_cascade`. Nunca borrar este test.
    #[test]
    fn retirement_surplus_without_rules_is_stranded_and_quantified() {
        let fondo = mk_asset(0xE3, Decimal::from(1_000_000), true, Some(Decimal::from(5)));
        let mut inp = base_input(
            24,
            Decimal::from(3_000),
            Decimal::from(1_000),
            vec![fondo],
            vec![],
        );
        inp.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        inp.phase_plan.income_retirement_monthly = Decimal::from(2_000);
        inp.phase_plan.expense_retirement_monthly = Decimal::from(1_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.contributed_capital[0], Decimal::ZERO);
        assert_eq!(out.contributed_capital[24], Decimal::ZERO);
        assert_eq!(
            out.per_asset_series[0][24].round_dp(2),
            dec_s("1102500.00"),
            "1.000.000 × 1,05² — el activo compone intocado"
        );
        assert_eq!(out.net_worth[24].round_dp(2), dec_s("1102500.00"));
        assert_eq!(out.unallocated_savings_total, Decimal::from(24_000));
    }

    /// 4.12.1 (#175) — EL ancla del CHANGELOG, derivada del bucle real por triple fuente:
    /// jubilado desde el mes 1, pensión 1.500 / gasto 1.000 (superávit 500 €/mes), inflación 0,
    /// activo al 5 % CON la regla resto (el estado de producción: siembra + #176). La cascada
    /// corre también jubilada y el superávit COMPONE como renta prepagable:
    /// V(360) = 500·m·(m³⁶⁰−1)/(m−1) con m = 1,05^(1/12) ⇒ **409.348,92 €** — donde 4.12.0
    /// acumulaba 180.000,00 € muertos en caja (Δ = +229.348,92, el coste exacto que #175
    /// cifró). Y lo reinvertido ES base: contributed(360) = 180.000 exactos.
    #[test]
    fn retirement_surplus_runs_the_users_cascade() {
        let fondo = mk_asset(0xE4, Decimal::ZERO, true, Some(Decimal::from(5)));
        let mut inp = base_input(
            360,
            Decimal::from(3_000),
            Decimal::from(1_000),
            vec![fondo],
            vec![rule_remainder(0)],
        );
        inp.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        inp.phase_plan.income_retirement_monthly = Decimal::from(1_500);
        inp.phase_plan.expense_retirement_monthly = Decimal::from(1_000);
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[360].round_dp(2), dec_s("409348.92"));
        assert_eq!(out.contributed_capital[360], Decimal::from(180_000));
        assert_eq!(out.unallocated_savings_total, Decimal::ZERO);
    }

    /// #140 fase 1 — el escenario de coste del issue, pineado con el número CORREGIDO en el
    /// propio issue (réplica a 50 dígitos verificada por partida doble): cartera arrancando
    /// EXACTAMENTE en el objetivo (gross_up(24.000)/0,035 = 863.652,8029) al 5 %, jubilado
    /// desde el mes 1, gasto 2.000 €/mes, sin pensión, tramos ES. Sin impuestos el drenaje
    /// neto dejaba NW(360) = 2.095.261,95; grosseando la retirada (bruto mensual
    /// 30.227,8481/12 = 2.518,9873) queda **1.670.368,13** — el «1.670.367» que circulaba era
    /// un artefacto de resta entre un truncado y un redondeado. Caída: −424.893,82.
    #[test]
    fn the_simulated_withdrawal_also_pays_taxes() {
        let brackets = crate::tax::es_brackets_for_tests();
        let objetivo = crate::tax::gross_up_net_annual_fire(
            Decimal::from(24_000),
            &brackets,
            true,
            Decimal::ONE,
        ) / dec_s("0.035");
        let build = |taxed: bool| {
            let fondo = mk_asset(0xF7, objetivo, true, Some(Decimal::from(5)));
            let mut inp = base_input(
                360,
                Decimal::from(3_000),
                Decimal::from(2_000),
                vec![fondo],
                vec![],
            );
            inp.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
            inp.phase_plan.income_retirement_monthly = Decimal::ZERO;
            inp.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
            inp.tax_brackets = brackets.clone();
            inp.taxes_enabled = taxed;
            project_net_worth_series(&inp).unwrap()
        };
        let neto = build(false);
        let bruto = build(true);
        assert_eq!(neto.net_worth[360].round_dp(2), dec_s("2095261.95"));
        assert_eq!(bruto.net_worth[360].round_dp(2), dec_s("1670368.13"));
        assert_eq!(
            (neto.net_worth[360] - bruto.net_worth[360]).round_dp(2),
            dec_s("424893.82"),
            "la caída del titular de #140"
        );
    }

    /// #140 fase 1 sobre el pin de #119 — y de paso pinea D-3 (TODO drenaje tributa, también
    /// el pre-jubilación: aquí no hay jubilación ninguna) y D-4 (`undrained` NETO). 200.000 €
    /// al 0 % gastando 2.000 €/mes con tramos ES: la venta bruta mensual es
    /// gross_up(24.000)/12 = 2.518,9873, así que la cartera se vacía en el mes **80** (antes
    /// 100) y el descubierto acumulado a 360 es NETO: se necesitaban 720.000 € de gasto, lo
    /// vendido neteó 158.800 (79 × 2.000 + 800 del mes 80) ⇒ NW(360) = **−561.200,00** con
    /// identidad comprobable. El bruto habría publicado −706.835,44: impuesto sobre ventas que
    /// nunca ocurrieron.
    #[test]
    fn depletion_arrives_earlier_and_the_uncovered_deficit_is_net() {
        let hucha = mk_asset(0xF8, Decimal::from(200_000), true, None);
        let mut inp = base_input(
            360,
            Decimal::ZERO,
            Decimal::from(2_000),
            vec![hucha],
            vec![],
        );
        inp.tax_brackets = crate::tax::es_brackets_for_tests();
        inp.taxes_enabled = true;
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(
            out.assets_depleted_month_index,
            Some(80),
            "con bruto se agota antes"
        );
        assert_eq!(out.net_worth[360].round_dp(2), dec_s("-561200.00"));
        assert_eq!(out.uncovered_deficit_total.round_dp(2), dec_s("561200.00"));
    }

    /// #178 — el GEMELO del test anterior con el coste DECLARADO: mismos 200.000 € al 0 %
    /// gastando 2.000 €/mes, pero `purchase_price = 160.000` ⇒ `g = 0,2` constante (al 0 % de
    /// crecimiento ρ es invariante — y que el mes de agotamiento aguante 360 meses ES el pin
    /// del teorema de invariancia). A mano: bruto = gross_up(24.000, 0,2)/12 =
    /// 2.079,00207900…/mes; agotamiento en el mes 97 (con g=1 era el 80: +17 meses de verdad);
    /// el mes 97 vende los 415,80 restantes que netean 400,00 exactos, y el descubierto NETO
    /// acumulado a 360 es 1.600 + 263×2.000 = 527.600,00 (con g=1: 561.200).
    #[test]
    fn declared_cost_derives_g_and_delays_depletion() {
        let mut hucha = mk_asset(0xF9, Decimal::from(200_000), true, None);
        hucha.purchase_price = Some(Decimal::from(160_000));
        let mut inp = base_input(
            360,
            Decimal::ZERO,
            Decimal::from(2_000),
            vec![hucha],
            vec![],
        );
        inp.tax_brackets = crate::tax::es_brackets_for_tests();
        inp.taxes_enabled = true;
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(
            out.assets_depleted_month_index,
            Some(97),
            "g derivada retrasa el agotamiento"
        );
        assert_eq!(out.net_worth[360].round_dp(2), dec_s("-527600.00"));
        assert_eq!(out.uncovered_deficit_total.round_dp(2), dec_s("527600.00"));
    }

    /// #178 — la vía MIXTA de verdad (dos activos con `g` distinta): A(10.000, coste 8.000,
    /// 2 %) drena primero (líquido, menor rentabilidad) y el mes 1 entero cabe en él dentro
    /// del primer tramo: bruto = 1.000/0,962 = 1.039,5010395010… — el caso 5a del spike,
    /// verificado por tres fuentes. NW(1) = (10.000 − 1.039,5010…)·1,02^(1/12) +
    /// 5.000·1,05^(1/12) = 13.995,6686. B(coste 1.000, g=0,8) ni se toca.
    #[test]
    fn mixed_g_drains_the_cheap_asset_first_at_its_own_gain_ratio() {
        let mut a = mk_asset(0xFA, Decimal::from(10_000), true, Some(Decimal::from(2)));
        a.purchase_price = Some(Decimal::from(8_000));
        let mut b = mk_asset(0xFB, Decimal::from(5_000), true, Some(Decimal::from(5)));
        b.purchase_price = Some(Decimal::from(1_000));
        let mut inp = base_input(12, Decimal::ZERO, Decimal::from(1_000), vec![a, b], vec![]);
        inp.tax_brackets = crate::tax::es_brackets_for_tests();
        inp.taxes_enabled = true;
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.net_worth[1].round_dp(4), dec_s("13995.6686"));
    }

    /// #178 — la trayectoria del ancla del issue: 500.000 € con coste 400.000 (ρ₀ = 0,8) al
    /// 5 %, gasto 2.000 €/mes neto, inflación 0. La `g` derivada SUBE sola
    /// (`g_k = 1 − 0,8·m^{−(k−1)}`: 0,2 → 0,37 a 5 años → 0,81 a 30) porque el crecimiento
    /// añade ganancia mientras la base cae proporcional al vender. Agotamiento en el mes
    /// **561** (46,7 años) — con el default g=1 de 4.11.0 era el 403 (13,2 años antes) y con
    /// el escalar 0,2 estático que la ayuda invitaba a poner, el 916 (29,6 años DESPUÉS de la
    /// verdad). Verificado por tres fuentes (spike Opus + réplica Decimal-50 + este bucle).
    #[test]
    fn derived_g_rises_along_the_trajectory_and_sets_the_honest_depletion() {
        let mut cartera = mk_asset(0xFC, Decimal::from(500_000), true, Some(Decimal::from(5)));
        cartera.purchase_price = Some(Decimal::from(400_000));
        let mut inp = base_input(
            840,
            Decimal::ZERO,
            Decimal::from(2_000),
            vec![cartera],
            vec![],
        );
        inp.tax_brackets = crate::tax::es_brackets_for_tests();
        inp.taxes_enabled = true;
        let out = project_net_worth_series(&inp).unwrap();
        assert_eq!(out.assets_depleted_month_index, Some(561));

        // Contraste con el default 4.11.0 (sin coste declarado ⇒ escalar g = 1): mes 403.
        let sin_coste = mk_asset(0xFD, Decimal::from(500_000), true, Some(Decimal::from(5)));
        let mut inp_g1 = base_input(
            840,
            Decimal::ZERO,
            Decimal::from(2_000),
            vec![sin_coste],
            vec![],
        );
        inp_g1.tax_brackets = crate::tax::es_brackets_for_tests();
        inp_g1.taxes_enabled = true;
        let out_g1 = project_net_worth_series(&inp_g1).unwrap();
        assert_eq!(out_g1.assets_depleted_month_index, Some(403));
    }

    /// #208, **misma familia que el pánico del solver mixto pero en el bucle**: un activo con
    /// coste declarado y rentabilidad muy negativa acaba con el VALOR pegado al mínimo
    /// representable de `Decimal` (1e-28) mientras su BASE sigue entera — el crecimiento no
    /// toca `basis` (solo la cascada y el drenaje lo hacen), así que el cociente `b/v` no está
    /// acotado por nada. En el primer mes de déficit el motor evalúa `g_i = 1 − b_i/v_i` y
    /// `100 / 1e-28 = 1e30` DESBORDA el rango de `Decimal` (~7,9e28): `/` panicaba, y el pool
    /// blocking lo publicaba como un 400 `task_panic` opaco. Divisor derivado por el propio
    /// motor y guardado solo por `> 0`: exactamente la forma que `checked_div` cierra.
    ///
    /// El resultado con el arreglo es el que el `clamp` ya decía: `b/v` enorme ⇒ `1 − b/v`
    /// muy negativo ⇒ `g = 0` (activo todo coste, no tributa al venderlo). Ningún caso que hoy
    /// no desborda cambia de valor.
    #[test]
    fn a_denormal_asset_value_does_not_overflow_the_derived_gain_ratio() {
        // ILÍQUIDO a propósito: nadie lo vende mientras la hucha líquida cubra el déficit, así
        // que su valor decae ~175 meses sin que el drenaje toque su base.
        let mut ruina = mk_asset(0x208, Decimal::from(100), false, Some(Decimal::from(-99)));
        ruina.purchase_price = Some(Decimal::from(100));
        let hucha = mk_asset(0x209, Decimal::from(10_000_000), true, None);
        let inp = base_input(
            240,
            Decimal::ZERO,
            Decimal::from(100),
            vec![ruina, hucha],
            vec![],
        );
        let out = project_net_worth_series(&inp)
            .expect("un valor denormal no debe hacer panicar el motor");
        assert_eq!(out.net_worth.len(), 241);
        // Queda pegado al mínimo representable — positivo, no cero: por eso la guarda
        // `*v > ZERO` no basta y hace falta `checked_div`.
        let v_final = out.per_asset_series[0][240];
        assert!(
            v_final > Decimal::ZERO,
            "el activo no llega a cero: {v_final}"
        );
        assert!(v_final < Decimal::new(1, 20), "y es denormal: {v_final}");
    }

    /// #170 — el objetivo sigue la necesidad REAL cuando la pensión queda plana. Caso central
    /// del issue (E_ret 2.000, pensión 1.000, i = 2 %, SWR 3,5): sin impuestos,
    /// target(0) = 12·1.000/0,035 = 342.857,14 (LA MISMA cifra de siempre — k=0 degenera
    /// exacto y la vista previa del formulario no se mueve); target(120) = 493.024,75 (la
    /// fórmula vieja decía 417.940,94); target(240) = 676.078,21 — el Δ (+166.610,54) es
    /// EXACTAMENTE la primera fila de la tabla del issue. Con tramos ES: 429.656,42 /
    /// 619.741,99 / 851.455,24.
    #[test]
    fn the_target_tracks_the_real_need_when_the_pension_stays_flat() {
        let build = |taxed: bool| FireTarget {
            need: FireNeed::ExpenseMinusPension {
                expense_monthly: Decimal::from(2_000),
                pension_monthly: Decimal::from(1_000),
            },
            swr_pct: dec_s("3.5"),
            tax_brackets: if taxed {
                crate::tax::es_brackets_for_tests()
            } else {
                Vec::new()
            },
            taxes_enabled: taxed,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::from(2),
            debt_payments_remaining: Vec::new(),
        };
        let t = |ft: &FireTarget, k: u32| fire_target_at_month_index(Some(ft), k).unwrap();
        let sin = build(false);
        for (k, esperado) in [
            (0u32, "342857.1429"),
            (120, "493024.7451"),
            (240, "676078.2144"),
        ] {
            let got = t(&sin, k);
            assert!(
                (got - dec_s(esperado)).abs() < dec_s("0.01"),
                "sin impuestos t({k}): esperado {esperado}, got {got}"
            );
        }
        let es = build(true);
        for (k, esperado) in [
            (0u32, "429656.4195"),
            (120, "619741.9920"),
            (240, "851455.2442"),
        ] {
            let got = t(&es, k);
            assert!(
                (got - dec_s(esperado)).abs() < dec_s("0.01"),
                "ES t({k}): esperado {esperado}, got {got}"
            );
        }
    }

    /// #170, puerta D-8: una pensión que cubre el gasto HOY significa SIN objetivo — para toda
    /// la serie, no `target = 0` (que con `líquido ≥ 0` siempre cierto daría un cruce FIRE
    /// inmediato y falso). El caso 4 de fire-parity (pensión 2.000 > gasto 1.500 ⇒ expected
    /// null) es la regresión API de esta misma puerta.
    #[test]
    fn a_covering_pension_means_no_target_ever() {
        let ft = FireTarget {
            need: FireNeed::ExpenseMinusPension {
                expense_monthly: Decimal::from(1_500),
                pension_monthly: Decimal::from(2_000),
            },
            swr_pct: dec_s("3.5"),
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::from(2),
            debt_payments_remaining: vec![Decimal::from(9_999)],
        };
        for k in [0u32, 1, 120, 480] {
            assert_eq!(fire_target_at_month_index(Some(&ft), k), None, "k={k}");
        }
    }

    /// #170 × #146: con DEFLACIÓN la necesidad puede agotarse dentro del horizonte — el gasto
    /// decrece y la pensión plana lo alcanza. E 2.000 / pensión 1.900 / i = −2 %: la necesidad
    /// de HOY es positiva (pasa la puerta), y desde que 2.000·0,98^(k/12) ≤ 1.900 (k ≥ 31) el
    /// objetivo queda en SOLO el término de deuda: te jubilas cuando tu pensión deflactada
    /// cubre el gasto. En k = 36 (exponente entero: 0,98³ = 0,941192 exacto) la base es 0.
    #[test]
    fn deflation_can_retire_the_need_leaving_only_the_debt_tail() {
        let ft = FireTarget {
            need: FireNeed::ExpenseMinusPension {
                expense_monthly: Decimal::from(2_000),
                pension_monthly: Decimal::from(1_900),
            },
            swr_pct: dec_s("3.5"),
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::from(-2),
            debt_payments_remaining: vec![Decimal::from(5_000)],
        };
        let t0 = fire_target_at_month_index(Some(&ft), 0).unwrap();
        assert!(t0 > Decimal::from(5_000), "hoy hay necesidad + cola: {t0}");
        assert_eq!(
            fire_target_at_month_index(Some(&ft), 36).unwrap(),
            Decimal::from(5_000),
            "gasto deflactado ≤ pensión: queda solo la cola de deuda"
        );
    }

    /// #170, el hallazgo ancho: el fiscal drag existe SIN pensión. `gross_up` es afín (término
    /// independiente en todo tramo salvo el primero), no homogénea: `gross_up(n·f) >
    /// gross_up(n)·f` para f > 1 — los tramos son NOMINALES y retirar más euros nominales
    /// dentro de 30 años SÍ cae en tramos más altos. Indexed 24.000 €/año, ES, i = 2 %:
    /// t(360) = gross_up(24.000·1,02³⁰)/0,035 = **1.571.527,94**, mientras inflar el bruto de
    /// hoy daba 1.564.387,51 — **+7.140,43 €** que la fórmula vieja no veía.
    #[test]
    fn fiscal_drag_the_grossup_of_the_inflated_need_beats_the_inflated_grossup() {
        let ft = FireTarget {
            need: FireNeed::Indexed {
                annual_net_today: Decimal::from(24_000),
            },
            swr_pct: dec_s("3.5"),
            tax_brackets: crate::tax::es_brackets_for_tests(),
            taxes_enabled: true,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::from(2),
            debt_payments_remaining: Vec::new(),
        };
        let t0 = fire_target_at_month_index(Some(&ft), 0).unwrap();
        assert!(
            (t0 - dec_s("863652.8029")).abs() < dec_s("0.01"),
            "k=0 sin mover: {t0}"
        );
        let t360 = fire_target_at_month_index(Some(&ft), 360).unwrap();
        assert!(
            (t360 - dec_s("1571527.9413")).abs() < dec_s("0.01"),
            "t(360) con drag: {t360}"
        );
        let viejo = t0 * inflation_factor_at_month_index(Decimal::from(2), 360);
        assert!(
            (t360 - viejo - dec_s("7140.43")).abs() < dec_s("0.02"),
            "el drag es ≈ +7.140,43: nuevo {t360}, viejo {viejo}"
        );
    }

    /// **REGRESIÓN de la issue #209** (antes: PÁNICO «Multiplication overflowed»).
    ///
    /// La base de coste baja con lo vendido — `b' = b·v_post/v_pre` (#120) — y el producto
    /// intermedio `b·v` desborda el rango de `Decimal` (~7,9e28) mucho antes que `b` o `v` por
    /// separado. Reproductor del issue, verbatim: un activo líquido con
    /// `value = purchase_price = 99.999.999.999.999` (el techo de `NUMERIC(18,4)`) al 20 %/año,
    /// horizonte 840, gasto 1.000 €/mes.
    ///
    /// A mano: la base se queda pegada a ~1e14 (solo encoge) mientras el valor compone al 20 %,
    /// así que el producto pasa de 7,9e28 en cuanto `v > 7,9e14`, o sea `1,2^t > 7,9` ⇒
    /// `t ≈ 11,3 años` ⇒ **hacia el mes 136**. En producción eso era un 400 `task_panic` opaco y
    /// permanente para ese hogar; el motor es una función pura y la API acepta esos importes.
    ///
    /// El arreglo reordena a `b·(v_post/v_pre)` **solo cuando el producto no cabe** (el cociente
    /// es ≤ 1 y no puede desbordar), así que ningún caso pineado cambia un dígito — lo demuestra
    /// el pin dorado, que sigue verde.
    #[test]
    fn the_cost_basis_update_survives_an_asset_at_the_numeric_ceiling() {
        let techo = Decimal::from(99_999_999_999_999i64);
        let asset = SimAsset {
            id: Uuid::from_u128(0x209),
            value: techo,
            purchase_price: Some(techo),
            is_liquid: true,
            expected_annual_return_percent: Some(Decimal::from(20)),
        };
        let inp = base_input(
            840,
            Decimal::ZERO,
            Decimal::from(1_000),
            vec![asset],
            vec![],
        );
        let out = project_net_worth_series(&inp)
            .expect("un activo en el techo de la columna no puede hacer panicar al motor");

        assert_eq!(out.net_worth.len(), 841);
        // Y no basta con «no panica»: los números tienen que seguir siendo finitos y coherentes.
        // El mes 136 es el primero cuyo producto `b·v` desbordaba: aquí se comprueba que la serie
        // lo atraviesa y que la base sigue siendo positiva y ≤ el valor (nunca puede superarlo:
        // el drenaje la baja en proporción y el crecimiento no la toca).
        for k in [1usize, 136, 137, 500, 840] {
            assert!(
                out.net_worth[k] > Decimal::ZERO,
                "mes {k}: {}",
                out.net_worth[k]
            );
            assert!(
                out.contributed_capital[k] > Decimal::ZERO
                    && out.contributed_capital[k] <= out.per_asset_series[0][k],
                "mes {k}: base {} contra valor {}",
                out.contributed_capital[k],
                out.per_asset_series[0][k]
            );
        }
        assert_eq!(
            out.uncovered_deficit_total,
            Decimal::ZERO,
            "1e14 € cubren 1.000 €/mes"
        );
        assert_eq!(out.assets_depleted_month_index, None);
        // La base ENCOGE con cada venta y el valor CRECE: la plusvalía relativa `g = 1 − b/v`
        // tiende a 1, que es la dirección que #178 describe.
        assert!(out.contributed_capital[840] < out.contributed_capital[1]);
        assert!(out.per_asset_series[0][840] > out.per_asset_series[0][1]);
    }
}
