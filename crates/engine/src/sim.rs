//! **Entrada y salida del núcleo GENÉRICO de simulación** (WP5.5 de 5.0.0, §B.4 del plan de #207).
//!
//! El motor tiene dos superficies desde WP5.5:
//!
//! - la **pública**, en `Decimal` — [`crate::ProjectionInput`], [`crate::ProjectionOutput`],
//!   [`crate::PhasePlan`]… — que es la que `apps/api` consume y que **no ha cambiado**;
//! - el **núcleo**, parametrizado por [`MoneyOps`], que es donde vive la aritmética.
//!
//! Este módulo son los tipos del núcleo y las conversiones entre las dos superficies. Las
//! conversiones son **copias literales campo a campo: cero aritmética**. Eso es lo que hace que
//! `project_net_worth_series` siga siendo bit a bit lo que era — no «un resultado equivalente»,
//! sino la misma secuencia de operaciones sobre los mismos operandos.
//!
//! # Por qué tipos MIRROR y no los públicos hechos genéricos
//!
//! Hacer genérico `PhasePlan` (con `M = Decimal` por defecto) habría ahorrado estas ~200 líneas,
//! pero cambia la INFERENCIA de todos sus usos: `WithdrawalRule::FixedReal` deja de tener un tipo
//! deducible por sí solo, y `apps/api` construye esos valores en decenas de sitios. La regla del
//! WP era que ningún ítem público del motor cambiara de forma. Los mirrors la cumplen sin
//! discusión, y las conversiones son exhaustivas campo a campo: **olvidar un campo es un error de
//! compilación**, no una divergencia silenciosa.
//!
//! # `growth_overrides`: el gancho de Monte Carlo
//!
//! [`SimInput::growth_overrides`] es el único campo del núcleo que NO tiene contrapartida en la
//! superficie pública, y es deliberado: es por donde WP6 inyecta los factores de crecimiento
//! sorteados. `None` —el único valor que produce la conversión desde `ProjectionInput`— deja el
//! bucle exactamente donde estaba (el multiplicador hoisted por activo).

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::money::MoneyOps;
use crate::phases::{
    EngineWarning, ExpenseBasis, IncomePause, PartialPhase, PensionSchedule, Phase, PhasePlan,
    RetirementTrigger, SpendMode, TargetBasis, WithdrawalRule,
};
use crate::projection::{
    AllocationCap, AllocationKind, AllocationRule, AllocationSkipReason, EarlyRepaymentEffect,
    EngineError, FireNeed, FireTarget, FirstMonthAllocation, ProjectionInput,
    ProjectionLiabilityInput, ProjectionOutput, RepaymentModel, RuleOutcome, SimAsset,
};
use crate::tax::TaxBracket;

// =============================================================================================
// Fiscalidad
// =============================================================================================

/// Un tramo de la escala del ahorro, en el tipo del núcleo.
///
/// El tipo PÚBLICO sigue siendo [`TaxBracket`] (`Decimal`, con la serde histórica del JSONB de
/// `fire_settings` intacta). Este es su gemelo aritmético; la conversión es una copia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaxBracketG<M> {
    /// `None` = tramo abierto (el último por contrato).
    pub up_to: Option<M>,
    pub pct: M,
}

impl<M: MoneyOps> TaxBracketG<M> {
    /// Convierte la escala pública al tipo del núcleo. Copia pura.
    pub fn from_decimal_slice(brackets: &[TaxBracket]) -> Vec<Self> {
        brackets
            .iter()
            .map(|b| TaxBracketG {
                up_to: b.up_to.map(M::from_decimal),
                pct: M::from_decimal(b.pct),
            })
            .collect()
    }
}

// =============================================================================================
// Plan de fases
// =============================================================================================

/// Media jornada, en el tipo del núcleo. Gemelo de [`PartialPhase`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartialPhaseG<M> {
    pub start_month: u32,
    pub income_monthly: M,
    pub expense_basis: ExpenseBasis,
}

/// Pensión con fecha, en el tipo del núcleo. Gemelo de [`PensionSchedule`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PensionScheduleG<M> {
    pub start_index: u32,
    pub monthly_today: M,
    pub indexed: bool,
    pub fraction_while_partial: M,
}

impl<M: MoneyOps> PensionScheduleG<M> {
    /// `P_m(i)`: `0` antes de `start_index`, `monthly_today·f(i)` si está indexada, plana si no.
    /// `f` llega YA evaluado (el bucle lo tiene en la mano) — duplicar aquí la llamada al factor
    /// recrearía la fórmula doble que #139 cerró.
    pub(crate) fn monthly_at(self, i: u32, inflation_factor: M) -> M {
        if i < self.start_index {
            return M::zero();
        }
        let base = self.monthly_today.max(M::zero());
        if self.indexed {
            base * inflation_factor
        } else {
            base
        }
    }

    /// La fracción cobrada durante [`Phase::Partial`], clampada a `[0, 1]` (degradación
    /// declarada, no silencio: la firma admite cualquier valor).
    pub(crate) fn partial_fraction(self) -> M {
        self.fraction_while_partial.clamp(M::zero(), M::one())
    }
}

/// Pausa de ingresos, en el tipo del núcleo. Gemelo de [`IncomePause`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IncomePauseG<M> {
    pub from_month: u32,
    pub months: u32,
    pub income_fraction: M,
}

impl<M: MoneyOps> IncomePauseG<M> {
    /// `Some(fracción)` dentro de la ventana semiabierta, `None` fuera — y `None` significa «no
    /// multipliques», no «multiplica por 1»: así el mes fuera de la ventana ejecuta EXACTAMENTE
    /// las mismas operaciones que sin pausa.
    pub(crate) fn factor_at(&self, k: u32) -> Option<M> {
        if self.months == 0 {
            return None;
        }
        let end = self.from_month.saturating_add(self.months);
        (k >= self.from_month && k < end).then(|| self.income_fraction.max(M::zero()))
    }
}

/// Catálogo de reglas de retirada, en el tipo del núcleo. Gemelo de [`WithdrawalRule`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WithdrawalRuleG<M> {
    FixedReal,
    PercentOfBalance { pct: M },
    Hybrid { start_pct: M, end_pct: M },
    Guardrails { pct: M, band_pct: M, adjust_pct: M },
}

/// El plan de fases, en el tipo del núcleo. Gemelo de [`PhasePlan`], campo a campo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhasePlanG<M> {
    pub retirement_trigger: RetirementTrigger,
    pub partial: Option<PartialPhaseG<M>>,
    pub pension: Option<PensionScheduleG<M>>,
    pub withdrawal: WithdrawalRuleG<M>,
    pub spend_mode: SpendMode,
    pub income_retirement_monthly: M,
    pub expense_retirement_monthly: M,
    pub extra_monthly_withdrawal: M,
    pub target_basis: TargetBasis,
    pub bridge_discount_annual_pct: M,
    pub crossing_is_reading_only: bool,
    pub contribution_cap_monthly: Option<M>,
    pub contributions_stop_month: Option<u32>,
    pub income_pause: Option<IncomePauseG<M>>,
}

impl<M: MoneyOps> PhasePlanG<M> {
    /// Puerta de entrada de las dos funciones que simulan: lo que el motor no sabe ejecutar NO
    /// se ejecuta. Hoy solo rechaza PARÁMETROS imposibles de una regla de retirada.
    pub(crate) fn ensure_supported(&self) -> Result<(), EngineError> {
        crate::withdrawal::validate_rule(self.withdrawal)
    }

    /// Techo de aportación EFECTIVO del mes `k` (1-based). `None` = sin techo. El corte de
    /// `contributions_stop_month` manda sobre el techo constante.
    pub(crate) fn contribution_cap_at(&self, k: u32) -> Option<M> {
        if self.contributions_stop_month.is_some_and(|s| k >= s) {
            return Some(M::zero());
        }
        self.contribution_cap_monthly.map(|c| c.max(M::zero()))
    }

    /// El gasto (sin indexar) que rige en la fase parcial. `None` si no hay fase parcial.
    pub(crate) fn partial_expense_basis_monthly(&self, expense_regular: M) -> Option<M> {
        self.partial.map(|p| match p.expense_basis {
            ExpenseBasis::Retirement => self.expense_retirement_monthly,
            ExpenseBasis::Regular => expense_regular,
        })
    }
}

// =============================================================================================
// Objetivo FIRE
// =============================================================================================

/// Estructura de la necesidad por modo FIRE, en el tipo del núcleo. Gemelo de [`FireNeed`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FireNeedG<M> {
    Indexed {
        annual_net_today: M,
    },
    ExpenseMinusPension {
        expense_monthly: M,
        pension_monthly: M,
    },
}

impl<M: MoneyOps> FireNeedG<M> {
    /// Necesidad neta ANUAL con el factor de inflación `f` ya evaluado. Única implementación:
    /// la fórmula duplicada es la trampa que #170 ya pagó una vez.
    pub(crate) fn annual_net_at(&self, f: M) -> M {
        match self {
            FireNeedG::Indexed { annual_net_today } => *annual_net_today * f,
            FireNeedG::ExpenseMinusPension {
                expense_monthly,
                pension_monthly,
            } => (*expense_monthly * f - *pension_monthly).max(M::zero()) * M::from_u32(12),
        }
    }
}

/// El objetivo FIRE, en el tipo del núcleo. Gemelo de [`FireTarget`].
#[derive(Debug, Clone, PartialEq)]
pub struct FireTargetG<M> {
    pub need: FireNeedG<M>,
    pub swr_pct: M,
    pub tax_brackets: Vec<TaxBracketG<M>>,
    pub taxes_enabled: bool,
    pub taxable_gain_ratio: M,
    pub annual_inflation_percent: M,
    pub debt_payments_remaining: Vec<M>,
}

/// **Vista PRESTADA de un objetivo FIRE**, en el tipo del núcleo.
///
/// Existe por una razón medida: `fire_target_at_month_index` la llama el handler **una vez por
/// mes** para construir `fire_target_series`, y [`FireTargetG`] contiene el vector de
/// `debt_payments_remaining` (hasta 841 números). Convertirlo entero en cada evaluación sería
/// copiar ~11 MB por request para leer UN elemento. La vista presta los dos vectores y copia solo
/// los escalares — que son `Copy`.
#[derive(Debug, Clone, Copy)]
pub struct FireTargetView<'a, M> {
    pub need: FireNeedG<M>,
    pub swr_pct: M,
    pub tax_brackets: &'a [TaxBracketG<M>],
    pub taxes_enabled: bool,
    pub taxable_gain_ratio: M,
    pub annual_inflation_percent: M,
    pub debt_payments_remaining: &'a [M],
}

impl<M: MoneyOps> FireTargetG<M> {
    /// La vista prestada de este objetivo. Copia cuatro escalares y presta dos vectores.
    pub fn view(&self) -> FireTargetView<'_, M> {
        FireTargetView {
            need: self.need,
            swr_pct: self.swr_pct,
            tax_brackets: &self.tax_brackets,
            taxes_enabled: self.taxes_enabled,
            taxable_gain_ratio: self.taxable_gain_ratio,
            annual_inflation_percent: self.annual_inflation_percent,
            debt_payments_remaining: &self.debt_payments_remaining,
        }
    }
}

// =============================================================================================
// Entrada del núcleo
// =============================================================================================

/// Un activo, en el tipo del núcleo. Gemelo de [`SimAsset`] **sin el `id`**: el motor nunca lo
/// lee (las series son por índice y la identidad la resuelve el handler), y arrastrarlo por el
/// núcleo sería llevar un `Uuid` a un bucle de Monte Carlo para nada.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimAssetG<M> {
    pub value: M,
    pub purchase_price: Option<M>,
    pub is_liquid: bool,
    pub expected_annual_return_percent: Option<M>,
}

/// Un pasivo, en el tipo del núcleo. Gemelo de [`ProjectionLiabilityInput`].
#[derive(Debug, Clone, PartialEq)]
pub struct SimLiability<M> {
    pub principal: M,
    pub monthly_payment: M,
    pub payment_end: Option<NaiveDate>,
    pub repayment_model: RepaymentModel,
    pub apr_percent: Option<M>,
    pub min_payment_pct: Option<M>,
    pub min_payment_eur: Option<M>,
    pub extra_principal_monthly: M,
    pub extra_principal_lump_sums: Vec<(u32, M)>,
    pub early_repayment_fee_pct: Option<M>,
    pub early_repayment_effect: EarlyRepaymentEffect,
}

/// Tope de una regla de la cascada, en el tipo del núcleo. Gemelo de [`AllocationCap`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AllocationCapG<M> {
    Amount(M),
    MonthsExpense(M),
    IncomeMultiple(M),
}

/// Una regla de la cascada, en el tipo del núcleo. Gemelo de [`AllocationRule`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AllocationRuleG<M> {
    pub target_index: usize,
    pub kind: AllocationKind,
    pub amount: Option<M>,
    pub cap: Option<AllocationCapG<M>>,
}

/// **El colchón de caja** (P4, §B.6 del plan de #207): el activo que absorbe la retirada y se
/// rellena vendiendo del resto de la cartera en los meses autorizados.
///
/// # Por qué vive en el núcleo y no en la capa de Monte Carlo
///
/// El relleno **es una venta**: realiza plusvalía, paga su impuesto por tramos y baja la base de
/// coste del activo vendido. Recolocar valor entre series DESPUÉS de simular —la única
/// alternativa sin tocar el motor— sería un colchón que se rellena sin pagar plusvalías, es
/// decir, un número mejor que la realidad. Por eso el gancho está aquí, dentro del mes, junto a
/// la venta que ya sabe hacer esa aritmética.
///
/// # Qué NO sabe el motor
///
/// Este plan **no menciona la volatilidad**: el motor no tiene noción de `σ` (vive en
/// `crates/engine-stochastic`) y no debe tenerla. Quien decide QUÉ meses autorizan relleno —en
/// Monte Carlo, los de shock positivo— y **si el colchón se instala siquiera** es el llamante. En
/// el camino determinista este campo es `None` y todo esto es código que no se ejecuta.
/// **Qué tamaño intenta mantener el colchón** (P4), y con qué convención se indexa.
///
/// Son dos magnitudes de naturaleza distinta y confundirlas sobrevalora la protección en
/// silencio:
///
/// - [`Months(n)`](CashBufferTarget::Months) — `n` meses del gasto **YA INDEXADO** del mes (el
///   mismo que el bucle acaba de gastar, no el declarado). El objetivo CRECE con la inflación,
///   igual que el gasto que cubre.
/// - [`Amount(a)`](CashBufferTarget::Amount) — un importe **NOMINAL FIJO**, que **no se indexa
///   nunca**. Es exactamente el euro que persigue el tope `amount` de una regla de la cascada
///   (`resolve_cap_ceiling_g`), y por eso existe: cuando el colchón se DERIVA del tope de una
///   regla de ahorro (5.0.0), **la misma regla gobierna las dos fases** —acumular hasta X y, ya
///   jubilado, mantener X—. Convertir ese tope a meses a mes 0 y dejar que se indexe sería otra
///   cosa: un tope de 10.000 € leído como «≈ 8 meses» valdría ~16.400 € nominales veinte años
///   después, 1,6× lo que el usuario escribió.
///
/// El motor no elige la variante: la elige el llamante, igual que elige el índice del colchón y
/// los meses autorizados.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CashBufferTarget<M> {
    /// `n` meses del gasto ya indexado del mes.
    Months(M),
    /// Un importe nominal fijo, sin indexar.
    Amount(M),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CashBufferPlan<M> {
    /// El activo que HACE de colchón: el líquido de menor rentabilidad esperada, que por
    /// construcción es el primero del orden de drenaje ([`crate::cash_buffer_index`]) y por tanto
    /// el que la retirada del mes vacía primero, sin que haga falta ninguna regla nueva.
    ///
    /// Fuera de rango ⇒ el colchón no actúa (el motor es una función pura y no panica por una
    /// entrada mal dimensionada, misma política que `growth_overrides`).
    pub buffer_index: usize,
    /// Cuánto intenta mantener el colchón. El objetivo del mes es
    /// `max(0, objetivo − valor_del_colchón)`, con el objetivo resuelto según la variante de
    /// [`CashBufferTarget`].
    pub target: CashBufferTarget<M>,
    /// `[k−1]` = ¿está autorizado el relleno en el mes `k` (1-based)?
    ///
    /// En Monte Carlo es `z_k > 0`: se rellena vendiendo **después** de que el mercado suba, no
    /// después de que baje — que es todo lo que el colchón pretende. Un índice fuera del vector
    /// cuenta como «no autorizado»: menos relleno, nunca más.
    pub refill_months: Vec<bool>,
}

/// **La entrada del núcleo de simulación.** Gemelo de [`ProjectionInput`] más el gancho de
/// Monte Carlo.
#[derive(Debug, Clone, PartialEq)]
pub struct SimInput<M> {
    pub ref_date: NaiveDate,
    pub horizon_months: u32,
    pub annual_inflation_percent: M,
    pub tax_brackets: Vec<TaxBracketG<M>>,
    pub taxes_enabled: bool,
    pub taxable_gain_ratio: M,
    pub income_regular_monthly: M,
    pub expense_regular_monthly: M,
    pub assets: Vec<SimAssetG<M>>,
    pub allocation_rules: Vec<AllocationRuleG<M>>,
    pub liabilities: Vec<SimLiability<M>>,
    pub planning_monthly_cash_adjustment: Vec<M>,
    pub phase_plan: PhasePlanG<M>,
    pub fire_target: Option<FireTargetG<M>>,
    /// **Factores de crecimiento por mes y por activo** (`[k−1][i]`, `k` 1-based), el gancho de
    /// Monte Carlo (WP6).
    ///
    /// `None` —lo único que produce la conversión desde [`ProjectionInput`]— deja el bucle usando
    /// el multiplicador hoisted por activo, que es el camino de 4.15.0 y el que el pin dorado
    /// hashea. Con `Some`, la fila `k−1` sustituye a ese multiplicador **solo si tiene tantos
    /// elementos como activos**: una fila mal dimensionada se ignora en vez de panicar, porque el
    /// motor es una función pura.
    pub growth_overrides: Option<Vec<Vec<M>>>,
    /// **El colchón de caja** (P4). `None` —lo único que produce la conversión desde
    /// [`ProjectionInput`]— deja el mes exactamente donde estaba: el bucle ni evalúa el objetivo.
    pub cash_buffer: Option<CashBufferPlan<M>>,
}

// =============================================================================================
// Salida del núcleo
// =============================================================================================

/// Traza de UNA regla en la cascada de un mes, en el tipo del núcleo. Gemelo de [`RuleOutcome`].
#[derive(Debug, Clone, PartialEq)]
pub struct RuleOutcomeG<M> {
    pub rule_index: usize,
    pub target_index: usize,
    pub amount_intent: M,
    pub amount_resolved: M,
    pub cap_ceiling: Option<M>,
    pub cap_room: Option<M>,
    pub skipped_reason: Option<AllocationSkipReason>,
}

/// Resolución de la cascada del primer mes, en el tipo del núcleo. Gemelo de
/// [`FirstMonthAllocation`].
#[derive(Debug, Clone, PartialEq)]
pub struct FirstMonthAllocationG<M> {
    pub per_asset: Vec<M>,
    pub base_cash: M,
    pub recurring_net: M,
    pub planning_component: M,
    pub debt_service: M,
    pub leftover: M,
    pub disposable: M,
    pub rules: Vec<RuleOutcomeG<M>>,
}

/// **La salida del núcleo de simulación.** Gemelo de [`ProjectionOutput`], campo a campo: la
/// semántica de cada uno vive allí y no se duplica aquí.
#[derive(Debug, Clone, PartialEq)]
pub struct SimOutput<M> {
    pub net_worth: Vec<M>,
    pub contributed_capital: Vec<M>,
    pub per_asset_series: Vec<Vec<M>>,
    pub assets_depleted_month_index: Option<u32>,
    pub uncovered_deficit_total: M,
    pub unallocated_savings_total: M,
    pub liquid_worth: Vec<M>,
    pub retirement_month_index: Option<u32>,
    pub liquid_crossing_month_index: Option<u32>,
    pub phase_transitions: Vec<(Phase, u32)>,
    pub withdrawal: Vec<M>,
    pub withdrawal_shortfall: Vec<M>,
    pub withdrawal_excess: Vec<M>,
    pub unmet_need: Vec<M>,
    pub pension_start_month_index: Option<u32>,
    pub partial_retirement_month_index: Option<u32>,
    pub warnings: Vec<EngineWarning>,
    pub bridge_effective_withdrawal_pct: Option<M>,
    pub pension_coverage_ratio: Option<M>,
    pub partial_gap_target: Option<M>,
    pub partial_phase_capital_growing: bool,
    pub disposable_cash: Vec<M>,
    pub disposable_cash_total: M,
    /// **Neto movido al colchón cada mes** (P4), `[k]` con el mismo eje que las demás series
    /// (índice 0 = estado inicial = 0). Todo ceros sin colchón.
    ///
    /// Es un TRASVASE, no un ingreso: el euro sale de otro activo. Lo único que el trasvase
    /// destruye es el impuesto de la venta, y eso ya lo cuenta el patrimonio.
    pub buffer_refill_net: Vec<M>,
    /// Cuántos meses hubo relleno EFECTIVO (neto > 0). `0` sin colchón, y también con colchón que
    /// nunca tuvo de dónde vender: las dos cosas son «no pasó nada», y quien distingue si el
    /// colchón siquiera se simuló es el llamante (en Monte Carlo, `buffer_active`).
    pub buffer_refill_months: u32,
}

// =============================================================================================
// Conversiones — copias literales, CERO aritmética
// =============================================================================================

impl<M: MoneyOps> From<&PhasePlan> for PhasePlanG<M> {
    fn from(p: &PhasePlan) -> Self {
        PhasePlanG {
            retirement_trigger: p.retirement_trigger,
            partial: p.partial.map(PartialPhase::to_generic),
            pension: p.pension.map(PensionSchedule::to_generic),
            withdrawal: p.withdrawal.to_generic(),
            spend_mode: p.spend_mode,
            income_retirement_monthly: M::from_decimal(p.income_retirement_monthly),
            expense_retirement_monthly: M::from_decimal(p.expense_retirement_monthly),
            extra_monthly_withdrawal: M::from_decimal(p.extra_monthly_withdrawal),
            target_basis: p.target_basis,
            bridge_discount_annual_pct: M::from_decimal(p.bridge_discount_annual_pct),
            crossing_is_reading_only: p.crossing_is_reading_only,
            contribution_cap_monthly: p.contribution_cap_monthly.map(M::from_decimal),
            contributions_stop_month: p.contributions_stop_month,
            income_pause: p.income_pause.map(IncomePause::to_generic),
        }
    }
}

impl<M: MoneyOps> From<&FireNeed> for FireNeedG<M> {
    fn from(n: &FireNeed) -> Self {
        match n {
            FireNeed::Indexed { annual_net_today } => FireNeedG::Indexed {
                annual_net_today: M::from_decimal(*annual_net_today),
            },
            FireNeed::ExpenseMinusPension {
                expense_monthly,
                pension_monthly,
            } => FireNeedG::ExpenseMinusPension {
                expense_monthly: M::from_decimal(*expense_monthly),
                pension_monthly: M::from_decimal(*pension_monthly),
            },
        }
    }
}

impl<M: MoneyOps> From<&FireTarget> for FireTargetG<M> {
    fn from(ft: &FireTarget) -> Self {
        FireTargetG {
            need: FireNeedG::from(&ft.need),
            swr_pct: M::from_decimal(ft.swr_pct),
            tax_brackets: TaxBracketG::from_decimal_slice(&ft.tax_brackets),
            taxes_enabled: ft.taxes_enabled,
            taxable_gain_ratio: M::from_decimal(ft.taxable_gain_ratio),
            annual_inflation_percent: M::from_decimal(ft.annual_inflation_percent),
            debt_payments_remaining: ft
                .debt_payments_remaining
                .iter()
                .copied()
                .map(M::from_decimal)
                .collect(),
        }
    }
}

impl<M: MoneyOps> From<&SimAsset> for SimAssetG<M> {
    fn from(a: &SimAsset) -> Self {
        SimAssetG {
            value: M::from_decimal(a.value),
            purchase_price: a.purchase_price.map(M::from_decimal),
            is_liquid: a.is_liquid,
            expected_annual_return_percent: a.expected_annual_return_percent.map(M::from_decimal),
        }
    }
}

impl<M: MoneyOps> From<&ProjectionLiabilityInput> for SimLiability<M> {
    fn from(l: &ProjectionLiabilityInput) -> Self {
        SimLiability {
            principal: M::from_decimal(l.principal),
            monthly_payment: M::from_decimal(l.monthly_payment),
            payment_end: l.payment_end,
            repayment_model: l.repayment_model,
            apr_percent: l.apr_percent.map(M::from_decimal),
            min_payment_pct: l.min_payment_pct.map(M::from_decimal),
            min_payment_eur: l.min_payment_eur.map(M::from_decimal),
            extra_principal_monthly: M::from_decimal(l.extra_principal_monthly),
            extra_principal_lump_sums: l
                .extra_principal_lump_sums
                .iter()
                .map(|(m, v)| (*m, M::from_decimal(*v)))
                .collect(),
            early_repayment_fee_pct: l.early_repayment_fee_pct.map(M::from_decimal),
            early_repayment_effect: l.early_repayment_effect,
        }
    }
}

impl<M: MoneyOps> From<&AllocationRule> for AllocationRuleG<M> {
    fn from(r: &AllocationRule) -> Self {
        AllocationRuleG {
            target_index: r.target_index,
            kind: r.kind,
            amount: r.amount.map(M::from_decimal),
            cap: r.cap.map(|c| match c {
                AllocationCap::Amount(v) => AllocationCapG::Amount(M::from_decimal(v)),
                AllocationCap::MonthsExpense(v) => {
                    AllocationCapG::MonthsExpense(M::from_decimal(v))
                }
                AllocationCap::IncomeMultiple(v) => {
                    AllocationCapG::IncomeMultiple(M::from_decimal(v))
                }
            }),
        }
    }
}

impl<M: MoneyOps> From<&ProjectionInput> for SimInput<M> {
    fn from(input: &ProjectionInput) -> Self {
        SimInput {
            ref_date: input.ref_date,
            horizon_months: input.horizon_months,
            annual_inflation_percent: M::from_decimal(input.annual_inflation_percent),
            tax_brackets: TaxBracketG::from_decimal_slice(&input.tax_brackets),
            taxes_enabled: input.taxes_enabled,
            taxable_gain_ratio: M::from_decimal(input.taxable_gain_ratio),
            income_regular_monthly: M::from_decimal(input.income_regular_monthly),
            expense_regular_monthly: M::from_decimal(input.expense_regular_monthly),
            assets: input.assets.iter().map(SimAssetG::from).collect(),
            allocation_rules: input
                .allocation_rules
                .iter()
                .map(AllocationRuleG::from)
                .collect(),
            liabilities: input.liabilities.iter().map(SimLiability::from).collect(),
            planning_monthly_cash_adjustment: input
                .planning_monthly_cash_adjustment
                .iter()
                .copied()
                .map(M::from_decimal)
                .collect(),
            phase_plan: PhasePlanG::from(&input.phase_plan),
            fire_target: input.fire_target.as_ref().map(FireTargetG::from),
            growth_overrides: None,
            cash_buffer: None,
        }
    }
}

impl From<SimOutput<Decimal>> for ProjectionOutput {
    /// Movimiento puro: con `M = Decimal` los vectores son ya del tipo publicado y **no se copia
    /// ni un número**.
    fn from(o: SimOutput<Decimal>) -> Self {
        ProjectionOutput {
            net_worth: o.net_worth,
            contributed_capital: o.contributed_capital,
            per_asset_series: o.per_asset_series,
            assets_depleted_month_index: o.assets_depleted_month_index,
            uncovered_deficit_total: o.uncovered_deficit_total,
            unallocated_savings_total: o.unallocated_savings_total,
            liquid_worth: o.liquid_worth,
            retirement_month_index: o.retirement_month_index,
            liquid_crossing_month_index: o.liquid_crossing_month_index,
            phase_transitions: o.phase_transitions,
            withdrawal: o.withdrawal,
            withdrawal_shortfall: o.withdrawal_shortfall,
            withdrawal_excess: o.withdrawal_excess,
            unmet_need: o.unmet_need,
            pension_start_month_index: o.pension_start_month_index,
            partial_retirement_month_index: o.partial_retirement_month_index,
            warnings: o.warnings,
            bridge_effective_withdrawal_pct: o.bridge_effective_withdrawal_pct,
            pension_coverage_ratio: o.pension_coverage_ratio,
            partial_gap_target: o.partial_gap_target,
            partial_phase_capital_growing: o.partial_phase_capital_growing,
            disposable_cash: o.disposable_cash,
            disposable_cash_total: o.disposable_cash_total,
            // `buffer_refill_net` y `buffer_refill_months` se QUEDAN AQUÍ a propósito: el colchón
            // (P4) solo actúa en Monte Carlo y en el camino determinista es todo ceros. Publicarlo
            // en `ProjectionOutput` sería añadir a la superficie pública una serie de 841 ceros
            // que ningún cliente puede interpretar. Quien lo lee es `crates/engine-stochastic`,
            // que consume `SimOutput` directamente.
        }
    }
}

impl From<RuleOutcomeG<Decimal>> for RuleOutcome {
    fn from(r: RuleOutcomeG<Decimal>) -> Self {
        RuleOutcome {
            rule_index: r.rule_index,
            target_index: r.target_index,
            amount_intent: r.amount_intent,
            amount_resolved: r.amount_resolved,
            cap_ceiling: r.cap_ceiling,
            cap_room: r.cap_room,
            skipped_reason: r.skipped_reason,
        }
    }
}

impl From<FirstMonthAllocationG<Decimal>> for FirstMonthAllocation {
    fn from(a: FirstMonthAllocationG<Decimal>) -> Self {
        FirstMonthAllocation {
            per_asset: a.per_asset,
            base_cash: a.base_cash,
            recurring_net: a.recurring_net,
            planning_component: a.planning_component,
            debt_service: a.debt_service,
            leftover: a.leftover,
            disposable: a.disposable,
            rules: a.rules.into_iter().map(RuleOutcome::from).collect(),
        }
    }
}

/// Gemelo de [`PensionSchedule::monthly_at`] para el mirror: la superficie pública sigue
/// existiendo y delega aquí, así que hay UNA sola definición de `P_m(i)`.
impl PensionSchedule {
    pub(crate) fn to_generic<M: MoneyOps>(self) -> PensionScheduleG<M> {
        PensionScheduleG {
            start_index: self.start_index,
            monthly_today: M::from_decimal(self.monthly_today),
            indexed: self.indexed,
            fraction_while_partial: M::from_decimal(self.fraction_while_partial),
        }
    }
}

/// Gemelo de [`PartialPhase`] — usado por las conversiones de arriba y por el evaluador público
/// del objetivo, que necesita el plan en el tipo del núcleo.
impl PartialPhase {
    pub(crate) fn to_generic<M: MoneyOps>(self) -> PartialPhaseG<M> {
        PartialPhaseG {
            start_month: self.start_month,
            income_monthly: M::from_decimal(self.income_monthly),
            expense_basis: self.expense_basis,
        }
    }
}

/// Gemelo de [`WithdrawalRule`] — la conversión que el mirror del plan usa, expuesta aparte
/// porque los tests del módulo de reglas validan la regla PÚBLICA.
impl WithdrawalRule {
    pub(crate) fn to_generic<M: MoneyOps>(self) -> WithdrawalRuleG<M> {
        match self {
            WithdrawalRule::FixedReal => WithdrawalRuleG::FixedReal,
            WithdrawalRule::PercentOfBalance { pct } => WithdrawalRuleG::PercentOfBalance {
                pct: M::from_decimal(pct),
            },
            WithdrawalRule::Hybrid { start_pct, end_pct } => WithdrawalRuleG::Hybrid {
                start_pct: M::from_decimal(start_pct),
                end_pct: M::from_decimal(end_pct),
            },
            WithdrawalRule::Guardrails {
                pct,
                band_pct,
                adjust_pct,
            } => WithdrawalRuleG::Guardrails {
                pct: M::from_decimal(pct),
                band_pct: M::from_decimal(band_pct),
                adjust_pct: M::from_decimal(adjust_pct),
            },
        }
    }
}

/// Gemelo de [`IncomePause`].
impl IncomePause {
    pub(crate) fn to_generic<M: MoneyOps>(self) -> IncomePauseG<M> {
        IncomePauseG {
            from_month: self.from_month,
            months: self.months,
            income_fraction: M::from_decimal(self.income_fraction),
        }
    }
}
