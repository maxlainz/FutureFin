//! **Reglas de retirada de la fase jubilada** (WP2 de 5.0.0, §B.2 del plan de la issue #207).
//!
//! Hasta 4.15.0 el motor tenía UNA regla y no la llamaba así: en cada mes jubilado con déficit
//! vendía exactamente lo que la caja no cubría. Eso es [`WithdrawalRule::FixedReal`] — «gasto
//! fijo en euros de hoy», sin techo. Este módulo añade las otras tres del catálogo (D6) y el
//! modo que decide cómo se relaciona la regla con el gasto declarado (D5):
//!
//! | Regla | Permitido BRUTO del mes `k` (jubilado) |
//! |---|---|
//! | `fixed_real` | la necesidad del mes, **sin techo** (`None` aquí: no hay regla que aplicar) |
//! | `percent_of_balance {pct}` | `pct/100 · L(k−1) / 12` |
//! | `hybrid {start,end}` | `start` hasta el latch, `end` después (ver [`WithdrawalPlanner`]) |
//! | `guardrails {pct,band,adjust}` | `W_R · mult · f(k−1)/f(R−1)`, con `mult` revisado cada 12 meses |
//!
//! **Convenciones de índice, y no son decorativas** (R9 del plan):
//!
//! - `L(k−1)` es el patrimonio LÍQUIDO al cierre del mes `k−1` — exactamente el mismo valor que
//!   el cruce FIRE consume ese mes (`liquid_prev` en el bucle), no el del cierre de `k`.
//! - `R` es el **primer mes jubilado** (1-based, la base de `retirement_month_index`), así que
//!   `L_R = L(R−1)` es el líquido con el que el hogar ENTRA en la jubilación.
//! - `f(i) = inflation_factor_at_month_index(inflación, i)`; el bucle evalúa el mes `k` con
//!   `f(k−1)`, así que el ancla de la jubilación es `f(R−1)`.
//! - **Los `pct` son BRUTOS de impuestos**, como el SWR (`gross/SWR`): el techo se aplica a la
//!   VENTA, no a los euros que llegan al bolsillo. Con impuestos encendidos, el neto obtenido de
//!   un techo del 4 % es menor que ese 4 %, y eso es el contrato, no un error de unidad.
//!
//! El módulo NO vende: solo dice CUÁNTO se puede vender. Quien ejecuta la venta (y quien decide
//! si además hay que vender en un mes de superávit, R7) es el bucle de `projection.rs`.

use rust_decimal::Decimal;

use crate::phases::WithdrawalRule;
use crate::projection::EngineError;

/// `pct/100 · balance / 12` — el permitido MENSUAL bruto de una regla de porcentaje.
///
/// Se calcula como `balance·pct/1200` (una sola división, el mínimo redondeo posible). Si el
/// producto no cabe en el rango de `Decimal` (~7,9e28) se reordena a `(balance/1200)·pct`, que
/// no puede desbordar para ningún `pct` sensato: misma disciplina que el `checked_mul` de la
/// base de coste (#209) y que los `checked_div` de #208 — la forma reordenada SOLO se ejecuta
/// donde la directa no cabe, así que ninguna entrada que hoy funciona cambia de valor.
///
/// Balance ≤ 0 (cartera vacía o en negativo) ⇒ permitido 0: un porcentaje de nada es nada, y
/// nunca una retirada negativa.
fn monthly_allowance(pct: Decimal, balance: Decimal) -> Decimal {
    if balance <= Decimal::ZERO || pct <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let twelve_hundred = Decimal::from(1_200u32);
    match balance.checked_mul(pct) {
        Some(p) => p / twelve_hundred,
        None => (balance / twelve_hundred) * pct,
    }
}

/// Ancla de la fase jubilada: el mes en que empezó y los dos escalares que las reglas con memoria
/// (hybrid, guardrails) necesitan para siempre.
#[derive(Debug, Clone, Copy)]
struct RetirementAnchor {
    /// `R`, 1-based (la base de `retirement_month_index`).
    month: u32,
    /// `L_R = L(R−1)`: líquido al cierre del mes anterior al primero jubilado.
    liquid: Decimal,
    /// `f(R−1)`: factor de inflación con el que el bucle indexó el gasto de ese primer mes.
    factor: Decimal,
}

/// Estado de la regla de retirada a lo largo de la simulación.
///
/// **Contrato de uso**: [`WithdrawalPlanner::allowed_gross`] se llama **exactamente una vez por
/// mes jubilado y en orden creciente de `k`**. No es una función pura por elección: `hybrid` y
/// `guardrails` tienen memoria (un latch y un multiplicador acumulado), y esa memoria es el
/// modelo — Guyton-Klinger sin ratchet acumulado no es Guyton-Klinger. Llamarla dos veces el
/// mismo mes revisaría el guardarraíl dos veces.
#[derive(Debug, Clone)]
pub(crate) struct WithdrawalPlanner {
    rule: WithdrawalRule,
    anchor: Option<RetirementAnchor>,
    /// `hybrid`: el latch ya cerró y rige `end_pct`. Monótono, como todos los latches del motor.
    hybrid_end_latched: bool,
    /// `guardrails`: producto acumulado de los ajustes disparados. Multiplica a `W_R`, NO al
    /// `W` del mes: así la indexación por inflación sigue funcionando sobre la base ajustada.
    guardrail_multiplier: Decimal,
    /// Último mes servido, solo para el `debug_assert` del contrato de uso.
    #[cfg(debug_assertions)]
    last_month: u32,
}

impl WithdrawalPlanner {
    pub(crate) fn new(rule: WithdrawalRule) -> Self {
        Self {
            rule,
            anchor: None,
            hybrid_end_latched: false,
            guardrail_multiplier: Decimal::ONE,
            #[cfg(debug_assertions)]
            last_month: 0,
        }
    }

    /// Fija el ancla de la jubilación la PRIMERA vez que se llama; después es no-op (el latch de
    /// jubilación del bucle es absorbente, #141, así que el ancla tampoco se mueve).
    pub(crate) fn anchor_retirement(&mut self, month: u32, liquid_prev: Decimal, factor: Decimal) {
        if self.anchor.is_none() {
            self.anchor = Some(RetirementAnchor {
                month,
                liquid: liquid_prev,
                factor,
            });
        }
    }

    /// Techo BRUTO del mes `k`, o `None` cuando la regla no pone techo (`fixed_real`) o el hogar
    /// todavía no se ha jubilado.
    ///
    /// `liquid_prev` = `L(k−1)`, `factor` = `f(k−1)`: los dos valores que el bucle ya tiene en la
    /// mano ese mes. Nada se recalcula aquí.
    pub(crate) fn allowed_gross(
        &mut self,
        month: u32,
        liquid_prev: Decimal,
        factor: Decimal,
    ) -> Option<Decimal> {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                month > self.last_month,
                "allowed_gross se llama una vez por mes jubilado y en orden creciente: \
                 {month} después de {}",
                self.last_month
            );
            self.last_month = month;
        }
        let anchor = self.anchor?;
        match self.rule {
            // Sin techo: el permitido ES la necesidad del mes, que el bucle ya conoce. Devolver
            // `None` (y no la necesidad) es lo que hace que la rama de déficit siga siendo la de
            // 4.15.0 operando a operando.
            WithdrawalRule::FixedReal => None,
            WithdrawalRule::PercentOfBalance { pct } => Some(monthly_allowance(pct, liquid_prev)),
            WithdrawalRule::Hybrid { start_pct, end_pct } => {
                if !self.hybrid_end_latched
                    && self.hybrid_switch_reached(start_pct, end_pct, liquid_prev, factor, &anchor)
                {
                    self.hybrid_end_latched = true;
                }
                let pct = if self.hybrid_end_latched {
                    end_pct
                } else {
                    start_pct
                };
                Some(monthly_allowance(pct, liquid_prev))
            }
            WithdrawalRule::Guardrails {
                pct,
                band_pct,
                adjust_pct,
            } => {
                // Revisión ANUAL en k = R+12, R+24, … sobre la retirada VIGENTE (la del
                // multiplicador de hoy) y el líquido de cierre del mes anterior.
                if month > anchor.month && (month - anchor.month) % 12 == 0 {
                    let current = self.guardrail_withdrawal(pct, factor, &anchor);
                    self.review_guardrails(pct, band_pct, adjust_pct, current, liquid_prev);
                }
                Some(self.guardrail_withdrawal(pct, factor, &anchor))
            }
        }
    }

    /// `end_pct · L(k−1) ≥ start_pct · L_R · f(k−1)/f(R−1)` — el latch de `hybrid`.
    ///
    /// En cristiano: se pasa al porcentaje final el primer mes en que la retirada que ese
    /// porcentaje produciría YA no es menor que la retirada inicial actualizada al IPC. Así el
    /// cambio de regla nunca RECORTA la retirada respecto de lo que el hogar venía sacando.
    ///
    /// La indexación se calcula como cociente `f(k−1)/f(R−1)` (no en cruz) para que las tres
    /// magnitudes se queden en el orden de euros: multiplicar `L·f` con carteras al borde del
    /// rango de `Decimal` desbordaría. Un cociente que no cabe (inflación degenerada, `f(R−1)`
    /// denormal) se lee como «sin indexación» en vez de panicar: el motor es una función pura.
    fn hybrid_switch_reached(
        &self,
        start_pct: Decimal,
        end_pct: Decimal,
        liquid_prev: Decimal,
        factor: Decimal,
        anchor: &RetirementAnchor,
    ) -> bool {
        let indexation = indexation_factor(factor, anchor.factor);
        let end_withdrawal = monthly_allowance(end_pct, liquid_prev);
        let start_withdrawal = monthly_allowance(start_pct, anchor.liquid);
        let start_indexed = match start_withdrawal.checked_mul(indexation) {
            Some(v) => v,
            // Una retirada inicial indexada que no cabe en `Decimal` no la alcanza ninguna
            // cartera representable: el latch no puede cerrarse todavía.
            None => return false,
        };
        end_withdrawal >= start_indexed
    }

    /// `W_k = W_R · mult · f(k−1)/f(R−1)` — la retirada de Guyton-Klinger del mes, en bruto.
    fn guardrail_withdrawal(
        &self,
        pct: Decimal,
        factor: Decimal,
        anchor: &RetirementAnchor,
    ) -> Decimal {
        let base = monthly_allowance(pct, anchor.liquid);
        let indexation = indexation_factor(factor, anchor.factor);
        base.checked_mul(self.guardrail_multiplier)
            .and_then(|v| v.checked_mul(indexation))
            // Un techo que no cabe en `Decimal` no ata ninguna venta real; saturar es la lectura
            // honesta («sin límite práctico») y no panica.
            .unwrap_or(Decimal::MAX)
            .max(Decimal::ZERO)
    }

    /// Las DOS reglas de Guyton-Klinger 2006 que este motor implementa, sobre la tasa efectiva
    /// del año `ratio = 12·W_k / L(k−1)` contra la inicial `ratio₀ = pct/100`:
    ///
    /// - **capital preservation**: `ratio > ratio₀·(1 + band/100)` ⇒ `W ·= (1 − adjust/100)`.
    /// - **prosperity**: `ratio < ratio₀·(1 − band/100)` ⇒ `W ·= (1 + adjust/100)`.
    ///
    /// **Las otras dos reglas del artículo original NO están implementadas, y se declara**: la
    /// *portfolio management rule* con su ventana de 15 años (que apaga el recorte cuando quedan
    /// menos de 15 años de plan) y la *inflation rule* (que salta la subida por IPC del año
    /// siguiente a un recorte). Ambas SUAVIZAN el modelo: omitirlas deja una versión más
    /// reactiva, que es la dirección prudente — y decirlo aquí es más barato que descubrirlo
    /// comparando con el artículo.
    ///
    /// En el camino DETERMINISTA con rentabilidad > SWR el líquido crece más deprisa que la
    /// retirada indexada, así que la prosperity dispara **todos los años** (ratchet). No es un
    /// bug: es lo que la regla dice sobre un camino sin volatilidad, y es exactamente por lo que
    /// los guardarraíles solo tienen sentido pleno con Monte Carlo (WP6).
    fn review_guardrails(
        &mut self,
        pct: Decimal,
        band_pct: Decimal,
        adjust_pct: Decimal,
        current_withdrawal: Decimal,
        liquid_prev: Decimal,
    ) {
        if liquid_prev <= Decimal::ZERO {
            // Sin cartera no hay tasa efectiva que medir (y `x/0` panica). El multiplicador se
            // queda como está: no se inventa un ajuste sobre una división imposible.
            return;
        }
        let hundred = Decimal::from(100u32);
        let annual = match current_withdrawal.checked_mul(Decimal::from(12u32)) {
            Some(v) => v,
            None => return,
        };
        let Some(ratio) = annual.checked_div(liquid_prev) else {
            return;
        };
        let ratio_0 = pct / hundred;
        let band = band_pct / hundred;
        if ratio > ratio_0 * (Decimal::ONE + band) {
            self.guardrail_multiplier *= Decimal::ONE - adjust_pct / hundred;
        } else if ratio < ratio_0 * (Decimal::ONE - band) {
            self.guardrail_multiplier *= Decimal::ONE + adjust_pct / hundred;
        }
    }
}

/// `f(k−1)/f(R−1)`, con la degradación declarada: si el ancla es 0 o el cociente no cabe en un
/// `Decimal`, se lee como 1 (sin indexación). Ninguna inflación que la API acepta ([−2, 50] %)
/// llega ahí — el factor de −2 % a 70 años sigue valiendo 0,24 —, pero el motor es una función
/// pura y no puede panicar con una entrada que su firma admite.
fn indexation_factor(factor: Decimal, anchor_factor: Decimal) -> Decimal {
    if anchor_factor <= Decimal::ZERO {
        return Decimal::ONE;
    }
    factor.checked_div(anchor_factor).unwrap_or(Decimal::ONE)
}

/// Cotas que el MOTOR exige a una regla. No duplican las de la API (`pct` en (0, 20],
/// `band`/`adjust` en (0, 50] — política de producto, `handlers/retirement_profile.rs`): aquí
/// solo se rechaza lo que haría la simulación absurda o imposible de interpretar. El motor es una
/// función pura: **rechaza con un error tipado, nunca panica ni degrada en silencio a otra
/// regla** (una regla aceptada y simulada como otra publicaría el patrimonio de un plan que nadie
/// configuró).
pub(crate) fn validate_rule(rule: WithdrawalRule) -> Result<(), EngineError> {
    let positive = |x: Decimal| x > Decimal::ZERO;
    let ok = match rule {
        WithdrawalRule::FixedReal => true,
        WithdrawalRule::PercentOfBalance { pct } => positive(pct),
        WithdrawalRule::Hybrid { start_pct, end_pct } => positive(start_pct) && positive(end_pct),
        WithdrawalRule::Guardrails {
            pct,
            band_pct,
            adjust_pct,
        } => {
            positive(pct)
                && positive(band_pct)
                && positive(adjust_pct)
                // Un ajuste ≥ 100 % dejaría la retirada en cero (o negativa) para siempre al
                // primer recorte: no es un guardarraíl, es un interruptor.
                && adjust_pct < Decimal::from(100u32)
        }
    };
    if ok {
        Ok(())
    } else {
        Err(EngineError::InvalidWithdrawalRule)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::{PhasePlan, RetirementTrigger, SpendMode};
    use crate::projection::{
        project_net_worth_series, AllocationKind, AllocationRule, ProjectionInput,
        ProjectionOutput, SimAsset,
    };
    use chrono::NaiveDate;
    use uuid::Uuid;

    // ── Utillería de los casos de extremo a extremo ──────────────────────────────────────────

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn mk_asset(id: u128, value: Decimal, rate: Option<Decimal>) -> SimAsset {
        SimAsset {
            id: Uuid::from_u128(id),
            value,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: rate,
        }
    }

    fn mk_asset_with_basis(
        id: u128,
        value: Decimal,
        rate: Option<Decimal>,
        cost: Decimal,
    ) -> SimAsset {
        SimAsset {
            id: Uuid::from_u128(id),
            value,
            purchase_price: Some(cost),
            is_liquid: true,
            expected_annual_return_percent: rate,
        }
    }

    /// Hogar jubilado DESDE EL MES 1 (`AtMonth(1)`), sin inflación, sin impuestos y con un solo
    /// activo al 0 %: todo lo que se mueve es la regla. Es el banco de pruebas de §B.2.
    fn retired_from_month_one(
        horizon: u32,
        liquid: Decimal,
        income_retirement: Decimal,
        expense_retirement: Decimal,
        rule: WithdrawalRule,
        spend_mode: SpendMode,
    ) -> ProjectionInput {
        let mut plan = PhasePlan::classic(income_retirement, expense_retirement);
        plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        plan.withdrawal = rule;
        plan.spend_mode = spend_mode;
        ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            horizon_months: horizon,
            annual_inflation_percent: Decimal::ZERO,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            income_regular_monthly: income_retirement,
            expense_regular_monthly: expense_retirement,
            assets: vec![mk_asset(1, liquid, None)],
            allocation_rules: vec![AllocationRule {
                target_index: 0,
                kind: AllocationKind::Remainder,
                amount: None,
                cap: None,
            }],
            liabilities: vec![],
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
            phase_plan: plan,
            fire_target: None,
        }
    }

    fn run(input: &ProjectionInput) -> ProjectionOutput {
        project_net_worth_series(input).expect("la simulación no debe fallar")
    }

    // ── Cotas ───────────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_rule_with_a_non_positive_percentage_is_rejected_not_simulated() {
        for rule in [
            WithdrawalRule::PercentOfBalance { pct: Decimal::ZERO },
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(-4),
            },
            WithdrawalRule::Hybrid {
                start_pct: Decimal::from(4),
                end_pct: Decimal::ZERO,
            },
            WithdrawalRule::Guardrails {
                pct: Decimal::from(4),
                band_pct: Decimal::ZERO,
                adjust_pct: Decimal::from(10),
            },
            WithdrawalRule::Guardrails {
                pct: Decimal::from(4),
                band_pct: Decimal::from(20),
                adjust_pct: Decimal::from(100),
            },
        ] {
            assert_eq!(
                validate_rule(rule),
                Err(EngineError::InvalidWithdrawalRule),
                "{rule:?} no puede aceptarse"
            );
            let mut input = retired_from_month_one(
                12,
                Decimal::from(100_000),
                Decimal::ZERO,
                Decimal::from(1_000),
                rule,
                SpendMode::Ceiling,
            );
            input.phase_plan.withdrawal = rule;
            assert_eq!(
                project_net_worth_series(&input).err(),
                Some(EngineError::InvalidWithdrawalRule),
                "y el bucle tampoco la simula: {rule:?}"
            );
        }
        assert_eq!(validate_rule(WithdrawalRule::FixedReal), Ok(()));
    }

    // ── `percent_of_balance` ────────────────────────────────────────────────────────────────

    /// **PREDICCIÓN (a mano, antes de correr nada).** 300.000 € líquidos al 0 %, gasto 2.000
    /// €/mes, sin ingreso, sin impuestos ni inflación, `percent_of_balance` al 4 % con techo:
    ///
    /// - mes 1: `L(0) = 300.000` ⇒ permitido = `0,04·300.000/12 = 1.000` < necesidad 2.000 ⇒
    ///   retirada **1.000**, recorte **1.000**, y `L(1) = 299.000`.
    /// - mes 2: `L(1) = 299.000` ⇒ permitido = `299.000·4/1200 = 996,666…` (periódico: se afirma
    ///   a 4 decimales) ⇒ retirada 996,6667, recorte 1.003,3333, `L(2) = 298.003,3333…`.
    /// - El patrimonio NO cae 2.000 €/mes: el recorte **no resta** (D22/D24). Frente a
    ///   `fixed_real` (que sí drena 2.000) el líquido es estrictamente mayor desde el mes 1.
    #[test]
    fn percent_of_balance_caps_the_sale_and_publishes_the_cut() {
        let input = retired_from_month_one(
            24,
            Decimal::from(300_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(4),
            },
            SpendMode::Ceiling,
        );
        let out = run(&input);

        assert_eq!(out.retirement_month_index, Some(1));
        assert_eq!(
            out.withdrawal[1],
            Decimal::from(1_000),
            "4 % anual de 300.000 / 12"
        );
        assert_eq!(out.withdrawal_shortfall[1], Decimal::from(1_000));
        assert_eq!(out.withdrawal_excess[1], Decimal::ZERO);
        assert_eq!(out.liquid_worth[1], Decimal::from(299_000));
        assert_eq!(out.net_worth[1], Decimal::from(299_000));

        assert_eq!(out.withdrawal[2].round_dp(4), dec("996.6667"));
        assert_eq!(out.withdrawal_shortfall[2].round_dp(4), dec("1003.3333"));
        assert_eq!(out.liquid_worth[2].round_dp(4), dec("298003.3333"));

        // El recorte es informativo: jamás entra en el descubierto ni resta patrimonio.
        assert_eq!(out.uncovered_deficit_total, Decimal::ZERO);
        assert_eq!(out.assets_depleted_month_index, None);

        // Y el contraste con `fixed_real`, que sí vende los 2.000 €.
        let mut fixed = input.clone();
        fixed.phase_plan.withdrawal = WithdrawalRule::FixedReal;
        let fixed = run(&fixed);
        assert_eq!(fixed.withdrawal[1], Decimal::from(2_000));
        assert_eq!(fixed.liquid_worth[1], Decimal::from(298_000));
        for k in 1..=24usize {
            assert!(
                out.net_worth[k] > fixed.net_worth[k],
                "el techo deja MÁS patrimonio que el drenaje sin techo (mes {k})"
            );
        }
    }

    /// **PREDICCIÓN.** Mismo hogar con `percent_of_balance` al **12 %**: permitido
    /// `300.000·12/1200 = 3.000` > necesidad 2.000.
    ///
    /// - `ceiling` ⇒ se vende la NECESIDAD: retirada 2.000, recorte 0, sobrante 0 y
    ///   `L(k) = 300.000 − 2.000k` (idéntico a `fixed_real`).
    /// - `rule_is_spend` ⇒ se vende el PERMITIDO: retirada 3.000, sobrante 1.000, recorte 0, y
    ///   `L(1) = 297.000`; mes 2 permitido `297.000/100 = 2.970` ⇒ `L(2) = 294.030`; mes 3
    ///   permitido 2.940,30 ⇒ `L(3) = 291.089,70`. Todo exacto (dividir por 100).
    #[test]
    fn the_two_spend_modes_split_when_the_rule_allows_more_than_the_need() {
        let ceiling = run(&retired_from_month_one(
            6,
            Decimal::from(300_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(12),
            },
            SpendMode::Ceiling,
        ));
        assert_eq!(ceiling.withdrawal[1], Decimal::from(2_000));
        assert_eq!(ceiling.withdrawal_excess[1], Decimal::ZERO);
        assert_eq!(ceiling.withdrawal_shortfall[1], Decimal::ZERO);
        assert_eq!(ceiling.liquid_worth[3], Decimal::from(294_000));

        let spend = run(&retired_from_month_one(
            6,
            Decimal::from(300_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(12),
            },
            SpendMode::RuleIsSpend,
        ));
        assert_eq!(spend.withdrawal[1], Decimal::from(3_000));
        assert_eq!(spend.withdrawal_excess[1], Decimal::from(1_000));
        assert_eq!(spend.withdrawal_shortfall[1], Decimal::ZERO);
        assert_eq!(spend.liquid_worth[1], Decimal::from(297_000));
        assert_eq!(spend.withdrawal[2], Decimal::from(2_970));
        assert_eq!(spend.liquid_worth[2], dec("294030"));
        assert_eq!(spend.withdrawal[3], dec("2940.30"));
        assert_eq!(spend.liquid_worth[3], dec("291089.70"));
    }

    /// Con `fixed_real` los dos modos COINCIDEN — y no por casualidad: el permitido se define
    /// como la necesidad, así que no hay techo que recortar ni sobrante que gastar. Es la
    /// propiedad que hace que 4.15.0 siga siendo bit-idéntico bajo cualquiera de los dos modos.
    #[test]
    fn under_fixed_real_both_spend_modes_are_the_same_simulation() {
        for (income, expense) in [
            (Decimal::ZERO, Decimal::from(2_000)),        // déficit puro
            (Decimal::from(3_000), Decimal::from(2_000)), // superávit puro
            (Decimal::from(2_000), Decimal::from(2_000)), // caja exactamente 0
        ] {
            let mut a = retired_from_month_one(
                36,
                Decimal::from(120_000),
                income,
                expense,
                WithdrawalRule::FixedReal,
                SpendMode::Ceiling,
            );
            a.annual_inflation_percent = dec("2.5");
            let mut b = a.clone();
            b.phase_plan.spend_mode = SpendMode::RuleIsSpend;
            let (a, b) = (run(&a), run(&b));
            assert_eq!(
                a.net_worth, b.net_worth,
                "income={income} expense={expense}"
            );
            assert_eq!(a.withdrawal, b.withdrawal);
            assert_eq!(a.withdrawal_shortfall, b.withdrawal_shortfall);
            assert_eq!(a.withdrawal_excess, b.withdrawal_excess);
            assert!(a.withdrawal_shortfall.iter().all(|v| *v == Decimal::ZERO));
            assert!(a.withdrawal_excess.iter().all(|v| *v == Decimal::ZERO));
        }
    }

    // ── `hybrid` ────────────────────────────────────────────────────────────────────────────

    /// **PREDICCIÓN.** Jubilado con 30.000 € líquidos, rentas de 5.000 €/mes y gasto de 2.000
    /// (superávit 3.000 que la cascada reinvierte), regla `hybrid` 12 % → 9 % en modo
    /// `rule_is_spend` (vende TODOS los meses, R7). Sin inflación ⇒ el latch es
    /// `9·L(k−1) ≥ 12·30.000` ⇒ **`L ≥ 40.000`**.
    ///
    /// Mes a mes (`L(k) = L(k−1) + 3.000 − permitido`, todo exacto porque `pct/1200` es 1/100 y
    /// 3/400):
    ///
    /// | k | L(k−1) | pct | permitido | L(k) |
    /// |---|---|---|---|---|
    /// | 1 | 30.000 | 12 | 300 | 32.700 |
    /// | 2 | 32.700 | 12 | 327 | 35.373 |
    /// | 3 | 35.373 | 12 | 353,73 | 38.019,27 |
    /// | 4 | 38.019,27 | 12 | 380,1927 | 40.639,0773 |
    /// | 5 | **40.639,0773 ≥ 40.000** | **9** | 304,79307975 | 43.334,28422025 |
    ///
    /// Y desde el mes 5 el latch no se abre nunca más (aunque el líquido cayera).
    #[test]
    fn hybrid_switches_the_moment_the_end_rule_matches_the_indexed_start() {
        let input = retired_from_month_one(
            12,
            Decimal::from(30_000),
            Decimal::from(5_000),
            Decimal::from(2_000),
            WithdrawalRule::Hybrid {
                start_pct: Decimal::from(12),
                end_pct: Decimal::from(9),
            },
            SpendMode::RuleIsSpend,
        );
        let out = run(&input);

        assert_eq!(out.withdrawal[1], Decimal::from(300));
        assert_eq!(out.liquid_worth[1], Decimal::from(32_700));
        assert_eq!(out.withdrawal[2], Decimal::from(327));
        assert_eq!(out.liquid_worth[2], Decimal::from(35_373));
        assert_eq!(out.withdrawal[3], dec("353.73"));
        assert_eq!(out.liquid_worth[3], dec("38019.27"));
        assert_eq!(out.withdrawal[4], dec("380.1927"));
        assert_eq!(out.liquid_worth[4], dec("40639.0773"));
        // El latch: el mes 5 es el primero cuyo `L(k−1)` llega a 40.000.
        assert_eq!(out.withdrawal[5], dec("304.79307975"));
        assert_eq!(out.liquid_worth[5], dec("43334.28422025"));

        // La regla ES el gasto: la necesidad es 0 (superávit) ⇒ todo lo vendido es sobrante.
        for k in 1..=12usize {
            assert_eq!(out.withdrawal_excess[k], out.withdrawal[k], "mes {k}");
            assert_eq!(out.withdrawal_shortfall[k], Decimal::ZERO);
        }
        // Y la cascada siguió invirtiendo los 3.000 € ANTES de la venta de la regla: nada quedó
        // varado (el sumidero se lo lleva todo).
        assert_eq!(out.unallocated_savings_total, Decimal::ZERO);
    }

    /// El latch de `hybrid` es MONÓTONO: una vez en `end_pct`, un desplome de la cartera no
    /// devuelve al hogar al porcentaje inicial. (Mismo criterio que el latch de jubilación,
    /// #141: nadie «se desjubila» porque un mes vaya mal.)
    #[test]
    fn the_hybrid_latch_never_reopens() {
        let mut input = retired_from_month_one(
            24,
            Decimal::from(30_000),
            Decimal::from(5_000),
            Decimal::from(2_000),
            WithdrawalRule::Hybrid {
                start_pct: Decimal::from(12),
                end_pct: Decimal::from(9),
            },
            SpendMode::RuleIsSpend,
        );
        // Una salida RECURRENTE de 3.000 €/mes desde el mes 6 se come el superávit entero: la
        // caja queda a cero, la cascada no aporta y el líquido decae al ritmo de la propia
        // regla (`L(k) = L(k−1)·0,9925`). Un socavón puntual NO serviría, y comprobarlo costó
        // una predicción fallida: con techo, la venta del mes está acotada y un −20.000 € de
        // caja no mueve el patrimonio — que es exactamente lo que estas reglas hacen.
        for i in 5..24usize {
            input.planning_monthly_cash_adjustment[i] = Decimal::from(-3_000);
        }
        let out = run(&input);
        // Mes 5: ya latcheado al 9 % (ver el test anterior).
        assert_eq!(out.withdrawal[5], dec("304.79307975"));
        // Desde 43.334,28 al 0,75 % mensual hacen falta 11 meses para bajar de 40.000
        // (`0,9925^11 = 0,9207 < 40.000/43.334,28 = 0,9231`), así que el mes 17 es el primero
        // que mira un líquido por debajo del umbral. Y AUN ASÍ sigue el 9 %:
        // `permitido = L(k−1)·9/1200 = L(k−1)·0,0075`.
        for k in 17..=24usize {
            assert!(
                out.liquid_worth[k - 1] < Decimal::from(40_000),
                "el mes {k} debería mirar un líquido hundido: {}",
                out.liquid_worth[k - 1]
            );
            // Tolerancia de 1e-18 y no `assert_eq!`: el motor calcula `L·9/1200` y aquí se
            // recomputa `L·0,0075` — el mismo número por dos caminos que redondean el dígito 28
            // en sitios distintos. Lo que este test afirma es el PORCENTAJE (0,75 %/mes = 9 %
            // anual), no la última cifra de `rust_decimal`.
            assert!(
                (out.withdrawal[k] - out.liquid_worth[k - 1] * dec("0.0075")).abs()
                    < dec("0.000000000000000001"),
                "el mes {k} sigue al 9 %, no vuelve al 12 %: {}",
                out.withdrawal[k]
            );
        }
    }

    // ── `guardrails` ────────────────────────────────────────────────────────────────────────

    /// **PREDICCIÓN — la regla de PROSPERIDAD dispara en k = 13.** Jubilado con 300.000 €
    /// líquidos, rentas 9.500 y gasto 2.000 (superávit 7.500), `guardrails` 4 / 20 / 10 en modo
    /// `rule_is_spend`:
    ///
    /// - `W_R = 4 %·300.000/12 = 1.000 €/mes`; sin inflación, `W_k = 1.000` mientras no se ajuste.
    /// - Cada mes: `L(k) = L(k−1) + 7.500 − 1.000` ⇒ `L(k) = 300.000 + 6.500k`; `L(12) = 378.000`.
    /// - Revisión en k = 13: `ratio = 12·1.000/378.000 = 0,031746…` < `0,04·(1−0,20) = 0,032`
    ///   ⇒ **prosperidad** ⇒ `W ·= 1,1` ⇒ **1.100 €**. `L(13) = 378.000 + 7.500 − 1.100 = 384.400`.
    /// - Meses 14–24 a 1.100 ⇒ `L(24) = 384.400 + 11·6.400 = 454.800`.
    /// - Revisión en k = 25: `ratio = 13.200/454.800 = 0,029023…` < 0,032 ⇒ dispara otra vez ⇒
    ///   **1.210 €** (el *ratchet* anual del camino determinista, documentado en
    ///   [`WithdrawalPlanner::review_guardrails`]).
    #[test]
    fn guardrails_prosperity_rule_ratchets_the_withdrawal_up_every_year() {
        let input = retired_from_month_one(
            30,
            Decimal::from(300_000),
            Decimal::from(9_500),
            Decimal::from(2_000),
            WithdrawalRule::Guardrails {
                pct: Decimal::from(4),
                band_pct: Decimal::from(20),
                adjust_pct: Decimal::from(10),
            },
            SpendMode::RuleIsSpend,
        );
        let out = run(&input);

        for k in 1..=12usize {
            assert_eq!(out.withdrawal[k], Decimal::from(1_000), "mes {k}");
        }
        assert_eq!(out.liquid_worth[12], Decimal::from(378_000));
        assert_eq!(
            out.withdrawal[13],
            Decimal::from(1_100),
            "prosperidad: ×1,1"
        );
        assert_eq!(out.liquid_worth[13], Decimal::from(384_400));
        for k in 14..=24usize {
            assert_eq!(out.withdrawal[k], Decimal::from(1_100), "mes {k}");
        }
        assert_eq!(out.liquid_worth[24], Decimal::from(454_800));
        assert_eq!(out.withdrawal[25], Decimal::from(1_210), "segundo ratchet");
    }

    /// **PREDICCIÓN — la regla de CAPITAL-PRESERVATION dispara en k = 61.** Mismo hogar sin
    /// rentas: gasto 2.000, `guardrails` 4 / 20 / 10 con techo (`ceiling`).
    ///
    /// - `W_R = 1.000 €/mes` y la necesidad es 2.000 ⇒ se vende el TECHO (1.000) todos los meses
    ///   y el recorte informativo es 1.000 €/mes.
    /// - `L(k) = 300.000 − 1.000k` (el recorte no resta patrimonio: D22/D24).
    /// - Revisiones: k = 13 ⇒ `12.000/288.000 = 0,041666…`; k = 25 ⇒ 0,043478…; k = 37 ⇒
    ///   0,045454…; k = 49 ⇒ `12.000/252.000 = 0,047619…` — todas **dentro** de la banda
    ///   (0,032; 0,048). k = 61 ⇒ `12.000/240.000 = 0,05 > 0,048` ⇒ **recorte** ⇒ `W = 900`.
    /// - `L(61) = 240.000 − 900 = 239.100`.
    #[test]
    fn guardrails_capital_preservation_rule_cuts_the_withdrawal() {
        let input = retired_from_month_one(
            72,
            Decimal::from(300_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::Guardrails {
                pct: Decimal::from(4),
                band_pct: Decimal::from(20),
                adjust_pct: Decimal::from(10),
            },
            SpendMode::Ceiling,
        );
        let out = run(&input);

        for k in 1..=60usize {
            assert_eq!(out.withdrawal[k], Decimal::from(1_000), "mes {k}");
            assert_eq!(out.withdrawal_shortfall[k], Decimal::from(1_000), "mes {k}");
            assert_eq!(
                out.liquid_worth[k],
                Decimal::from(300_000) - Decimal::from(1_000) * Decimal::from(k as u32),
                "mes {k}"
            );
        }
        assert_eq!(out.withdrawal[61], Decimal::from(900), "recorte: ×0,9");
        assert_eq!(out.withdrawal_shortfall[61], Decimal::from(1_100));
        assert_eq!(out.liquid_worth[61], Decimal::from(239_100));
        // Ni un euro de descubierto: el recorte NO es un impago.
        assert_eq!(out.uncovered_deficit_total, Decimal::ZERO);
    }

    /// La retirada de los guardarraíles se INDEXA al IPC (`W_R·mult·f(k−1)/f(R−1)`), que es la
    /// diferencia entre Guyton-Klinger y un porcentaje del saldo. Con 3 % de inflación y
    /// jubilación en el mes 1 (`f(R−1) = f(0) = 1`), la retirada del mes 13 es
    /// `1.000·1,03^(12/12) = 1.030` — pero el mes 13 es también una revisión, así que se afirma
    /// el mes 12: `1.000·1,03^(11/12) = 1.027,47…` (±0,01 €, hay `powd` de por medio).
    ///
    /// (La predicción se escribió primero como «1.027,50» y el motor devolvió 1.027,4660: el
    /// error estaba en la cuenta a mano —`exp(11/12·ln 1,03) = 1,0274661`—, no en el motor. Se
    /// anota porque una predicción corregida vale más que una que nunca falló.)
    #[test]
    fn the_guardrails_withdrawal_is_indexed_to_inflation() {
        let mut input = retired_from_month_one(
            12,
            Decimal::from(300_000),
            Decimal::ZERO,
            Decimal::from(5_000),
            WithdrawalRule::Guardrails {
                pct: Decimal::from(4),
                band_pct: Decimal::from(20),
                adjust_pct: Decimal::from(10),
            },
            SpendMode::Ceiling,
        );
        input.annual_inflation_percent = Decimal::from(3);
        let out = run(&input);
        assert_eq!(out.withdrawal[1], Decimal::from(1_000), "f(0) = 1 exacto");
        let esperado = Decimal::from(1_000)
            * crate::projection::inflation_factor_at_month_index(Decimal::from(3), 11);
        assert!(
            (out.withdrawal[12] - esperado).abs() < dec("0.01"),
            "mes 12: {} vs {esperado}",
            out.withdrawal[12]
        );
        assert!(
            (out.withdrawal[12] - dec("1027.47")).abs() < dec("0.01"),
            "y el número a mano: 1.000·1,03^(11/12) = 1.027,47 — {}",
            out.withdrawal[12]
        );
    }

    // ── Impuestos: el techo es BRUTO (R9) ───────────────────────────────────────────────────

    /// **PREDICCIÓN.** El `pct` de la regla es BRUTO, como el SWR. Jubilado con 300.000 € (sin
    /// coste declarado ⇒ `g` uniforme = 1), gasto 5.000 €/mes, escala ES, `percent_of_balance`
    /// al 4 % ⇒ permitido = **1.000 € BRUTOS**.
    ///
    /// - Impuesto M1 sobre 12.000 € anuales: `6.000·19 % + 6.000·21 % = 1.140 + 1.260 = 2.400`
    ///   ⇒ 200 €/mes ⇒ **neto obtenido 800 €**, que es lo que la serie `withdrawal` publica.
    /// - Recorte = `5.000 − 800 = 4.200` (la necesidad menos lo que la regla dejó netear).
    /// - El líquido baja el BRUTO: `L(1) = 300.000 − 1.000 = 299.000`.
    #[test]
    fn the_percentage_caps_the_gross_sale_not_the_net_cash() {
        let mut input = retired_from_month_one(
            6,
            Decimal::from(300_000),
            Decimal::ZERO,
            Decimal::from(5_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(4),
            },
            SpendMode::Ceiling,
        );
        input.tax_brackets = crate::tax::es_brackets_for_tests();
        input.taxes_enabled = true;
        let out = run(&input);

        assert_eq!(
            out.liquid_worth[1],
            Decimal::from(299_000),
            "el BRUTO sale de la cartera"
        );
        assert_eq!(
            out.withdrawal[1],
            Decimal::from(800),
            "y solo 800 llegan al bolsillo"
        );
        assert_eq!(out.withdrawal_shortfall[1], Decimal::from(4_200));
        assert_eq!(out.withdrawal_excess[1], Decimal::ZERO);
        assert_eq!(out.uncovered_deficit_total, Decimal::ZERO);
    }

    /// **PREDICCIÓN — techo BRUTO con `g` MIXTA, paseo exacto (§B.2 / #178).** Dos activos
    /// líquidos:
    ///
    /// - A: valor 1.000, coste 800 ⇒ `g = 0,2` (drena primero, sin rentabilidad).
    /// - B: valor 200.000, coste 100.000 ⇒ `g = 0,5` (ídem: `liquid_worth[k]` se publica
    ///   DESPUÉS del crecimiento del mes, y cualquier rendimiento taparía la venta exacta).
    ///
    /// `L(0) = 201.000`, `percent_of_balance` al 12 % ⇒ permitido = `201.000/100 = 2.010` brutos.
    /// El paseo directo, en unidades ANUALES (M1: `12·2.010 = 24.120`):
    ///
    /// | tramo | `g` | tipo | venta | base acumulada | neto |
    /// |---|---|---|---|---|---|
    /// | A (cap. 12.000) | 0,2 | 19 % | 12.000 | 2.400 | `12.000·0,962 = 11.544` |
    /// | B hasta llenar el tramo | 0,5 | 19 % | 7.200 | 6.000 | `7.200·0,905 = 6.516` |
    /// | B, resto del techo | 0,5 | 21 % | 4.920 | 8.460 | `4.920·0,895 = 4.403,40` |
    ///
    /// Bruto 24.120 (= el techo, EXACTO), neto 22.463,40 ⇒ **1.871,95 €/mes**. Partida doble con
    /// la escala: `tax(8.460) = 6.000·19 % + 2.460·21 % = 1.656,60`, y `24.120 − 1.656,60 =
    /// 22.463,40`. ✓
    ///
    /// Reparto: A entero (1.000) y B `12.120/12 = 1.010`. Bases: A → 0; B →
    /// `100.000·198.990/200.000 = 99.495`.
    #[test]
    fn the_gross_cap_is_walked_exactly_across_mixed_gain_ratios() {
        let mut input = retired_from_month_one(
            1,
            Decimal::ZERO, // se sustituyen los activos justo debajo
            Decimal::ZERO,
            Decimal::from(5_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(12),
            },
            SpendMode::Ceiling,
        );
        input.assets = vec![
            mk_asset_with_basis(1, Decimal::from(1_000), None, Decimal::from(800)),
            mk_asset_with_basis(2, Decimal::from(200_000), None, Decimal::from(100_000)),
        ];
        input.allocation_rules = vec![];
        input.tax_brackets = crate::tax::es_brackets_for_tests();
        input.taxes_enabled = true;
        let out = run(&input);

        assert_eq!(out.liquid_worth[0], Decimal::from(201_000));
        // El BRUTO vendido es exactamente el techo.
        assert_eq!(
            out.liquid_worth[1],
            dec("198990"),
            "201.000 − 2.010 de venta bruta"
        );
        assert_eq!(
            out.per_asset_series[0][1],
            Decimal::ZERO,
            "A se vende entero"
        );
        assert_eq!(out.per_asset_series[1][1], dec("198990"), "B pone 1.010");
        assert_eq!(
            out.withdrawal[1],
            dec("1871.95"),
            "el neto del paseo exacto"
        );
        assert_eq!(out.withdrawal_shortfall[1], dec("3128.05"));
        assert_eq!(
            out.withdrawal[1] + out.withdrawal_shortfall[1],
            Decimal::from(5_000),
            "retirada + recorte = necesidad"
        );
        // Base de coste: A a cero exacto, B en proporción al valor.
        assert_eq!(out.contributed_capital[1], dec("99495"));
        assert_eq!(out.uncovered_deficit_total, Decimal::ZERO);
    }

    // ── Las TRES magnitudes separadas (B.1.5, D22/D24) ──────────────────────────────────────

    /// El recorte NO resta patrimonio y NO es el descubierto: cuando la cartera se agota de
    /// verdad, lo que crece es `uncovered_deficit_total` (y `net_worth` cae), no el recorte.
    ///
    /// Hogar jubilado con 5.000 € líquidos, gasto 2.000 y `percent_of_balance` al 120 % (techo
    /// enorme a propósito: `5.000·120/1200 = 500` el primer mes, pero el techo baja con el
    /// saldo). Aquí el techo SÍ ata al principio y la cartera nunca se vacía del todo —
    /// el complemento del caso anterior.
    #[test]
    fn the_three_magnitudes_do_not_contaminate_each_other() {
        let out = run(&retired_from_month_one(
            36,
            Decimal::from(5_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(120),
            },
            SpendMode::Ceiling,
        ));
        for k in 1..=36usize {
            // Nunca hay descubierto: el techo siempre es menor que lo vendible.
            assert!(out.withdrawal[k] > Decimal::ZERO, "mes {k}");
            assert_eq!(
                out.withdrawal[k] + out.withdrawal_shortfall[k],
                Decimal::from(2_000),
                "mes {k}: retirada + recorte = necesidad"
            );
            assert_eq!(out.withdrawal_excess[k], Decimal::ZERO);
        }
        assert_eq!(out.uncovered_deficit_total, Decimal::ZERO);
        assert_eq!(out.assets_depleted_month_index, None);
        assert!(out.net_worth[36] > Decimal::ZERO);

        // Y el control: con `fixed_real` el mismo hogar SÍ se arruina — 5.000/2.000 ⇒ el mes 3
        // la venta necesaria (2.000) iguala o supera lo vendible (1.000) ⇒ agotamiento, y desde
        // ahí el descubierto acumula.
        let mut fixed = retired_from_month_one(
            36,
            Decimal::from(5_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::FixedReal,
            SpendMode::Ceiling,
        );
        fixed.phase_plan.withdrawal = WithdrawalRule::FixedReal;
        let fixed = run(&fixed);
        assert_eq!(fixed.assets_depleted_month_index, Some(3));
        assert!(fixed.uncovered_deficit_total > Decimal::ZERO);
        assert!(fixed
            .withdrawal_shortfall
            .iter()
            .all(|v| *v == Decimal::ZERO));
        assert!(fixed.net_worth[36] < Decimal::ZERO);
    }

    /// El recorte de una regla deja SIEMPRE más patrimonio que `fixed_real` — es el teorema que
    /// hace que las tres magnitudes no se pisen: lo que no se vende, se queda.
    #[test]
    fn a_capped_run_never_has_less_net_worth_than_the_uncapped_one() {
        let base = |rule| {
            retired_from_month_one(
                120,
                Decimal::from(250_000),
                Decimal::from(500),
                Decimal::from(2_500),
                rule,
                SpendMode::Ceiling,
            )
        };
        let mut capped = base(WithdrawalRule::PercentOfBalance { pct: dec("3.5") });
        capped.annual_inflation_percent = Decimal::from(2);
        let mut uncapped = capped.clone();
        uncapped.phase_plan.withdrawal = WithdrawalRule::FixedReal;
        let (capped, uncapped) = (run(&capped), run(&uncapped));
        for k in 0..=120usize {
            assert!(
                capped.net_worth[k] >= uncapped.net_worth[k],
                "mes {k}: {} < {}",
                capped.net_worth[k],
                uncapped.net_worth[k]
            );
        }
        assert!(capped.uncovered_deficit_total < uncapped.uncovered_deficit_total);
    }

    /// La regla solo gobierna la fase JUBILADA: antes del cruce un mes con déficit drena como
    /// siempre (sin techo), porque un hogar que trabaja y tiene un mes malo no está aplicando
    /// una política de retirada.
    #[test]
    fn the_rule_does_not_touch_the_accumulation_phase() {
        let mut input = retired_from_month_one(
            6,
            Decimal::from(100_000),
            Decimal::ZERO,
            Decimal::from(2_000),
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(1),
            },
            SpendMode::RuleIsSpend,
        );
        // Sin trigger y sin objetivo: el hogar NUNCA se jubila.
        input.phase_plan.retirement_trigger = RetirementTrigger::LiquidCrossing;
        let out = run(&input);
        assert_eq!(out.retirement_month_index, None);
        for k in 1..=6usize {
            assert_eq!(
                out.withdrawal[k],
                Decimal::from(2_000),
                "mes {k}: drenaje sin techo"
            );
            assert_eq!(out.withdrawal_shortfall[k], Decimal::ZERO);
            assert_eq!(out.withdrawal_excess[k], Decimal::ZERO);
        }
    }
}
