//! **Plan de fases de la simulación** (WP1b de 5.0.0, §B.1 del plan de #207).
//!
//! Hasta 4.15.0 la jubilación vivía en cuatro campos sueltos de [`ProjectionInput`]
//! (`retirement_start_month`, `income_retirement_monthly`, `expense_retirement_monthly`,
//! `retirement_monthly_withdrawal`) que dos sitios distintos del motor interpretaban por su
//! cuenta: el bucle de simulación y `first_month_allocation`. Este módulo los absorbe en UN
//! objeto —[`PhasePlan`]— que ambos consumen, para que las fases que llegan después (media
//! jornada, pensión con fecha, reglas de retirada) tengan un único sitio donde declararse en vez
//! de multiplicar los `if` por el bucle.
//!
//! **WP1b no cambió una sola cifra**: el plan que el handler construye hoy
//! ([`PhasePlan::classic`]) tiene exactamente la semántica de 4.15.0. Lo que aún no está
//! implementado (fase parcial y pensión con fecha, WP3) se rechaza con un error TIPADO —nunca se
//! ignora en silencio: un plan aceptado y no simulado publicaría un patrimonio plausible y
//! equivocado, que es justo la clase de fallo que esta casa no publica.
//!
//! **WP2 añadió las reglas de retirada**: las cuatro de [`WithdrawalRule`] y los dos
//! [`SpendMode`] se simulan de verdad, y su aritmética vive en
//! [`crate::withdrawal`](../withdrawal/index.html).

use rust_decimal::Decimal;

use crate::projection::EngineError;

/// Qué dispara la jubilación TOTAL.
///
/// **Un solo trigger por simulación** es una regla de ESTRATEGIA (D17), no del motor: las
/// estrategias por edad se jubilan en `R` aunque el capital no llegue, y el cruce pasa a ser una
/// lectura ([`crate::ProjectionOutput::liquid_crossing_month_index`]). Quien la hace cumplir es el
/// HANDLER (WP3): para una estrategia por edad pasará `fire_target: None`, y entonces el cruce no
/// puede dispararse porque no hay objetivo contra el que cruzar.
///
/// El motor, a propósito, conserva la UNIÓN de 4.15.0 —`retired || cruce || k ≥ s`— porque es lo
/// que el pin dorado tiene fotografiado: `P10_jubilacion_forzada` fuerza el mes 37 y el caso
/// existe precisamente porque la API nunca rellenó ese campo (ningún test de integración lo
/// cubre). Cambiar aquí la unión por una exclusión movería ese caso sin que nadie lo pidiera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementTrigger {
    /// Jubilación por CRUCE: `líquido(k−1) ≥ objetivo(k−1)`. Sin `fire_target` no ocurre nunca.
    LiquidCrossing,
    /// Jubilación FORZADA: jubilado desde el mes `k` del bucle (1-based, la misma base que
    /// `assets_depleted_month_index`) en adelante, con la MISMA condición `k >= s` que el difunto
    /// `retirement_start_month`. El cruce sigue evaluándose y sigue pudiendo adelantar la
    /// jubilación: `min(cruce, s)`, exactamente como en 4.15.0.
    AtMonth(u32),
}

impl RetirementTrigger {
    /// El mes forzado, si lo hay. Sustituye a `input.retirement_start_month` en los dos sitios
    /// que lo miraban.
    pub fn forced_month(self) -> Option<u32> {
        match self {
            RetirementTrigger::LiquidCrossing => None,
            RetirementTrigger::AtMonth(s) => Some(s),
        }
    }
}

/// Cómo se relaciona la regla de retirada con el gasto declarado (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendMode {
    /// La regla es un TECHO: se retira `min(necesidad, permitido)`, y solo en meses con déficit.
    Ceiling,
    /// La regla ES el gasto del patrimonio: se retira `permitido` todos los meses jubilados (R7).
    RuleIsSpend,
}

/// Catálogo de reglas de retirada (D6). **Las cuatro se simulan desde WP2**
/// (`crates/engine/src/withdrawal.rs`, §B.2 del plan de #207); lo que el motor rechaza ahora son
/// los PARÁMETROS imposibles (un porcentaje ≤ 0, un ajuste ≥ 100 %) con
/// [`EngineError::InvalidWithdrawalRule`]. Rechazar es la única salida honesta: aceptar una regla
/// y simular otra publicaría números creíbles de un plan que nadie configuró.
///
/// Los `pct` son **BRUTOS de impuestos**, igual que el SWR (R9): el techo se aplica a la VENTA,
/// no a los euros que llegan al bolsillo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalRule {
    /// «Gasto fijo en euros de hoy» (R1): el permitido ES la necesidad del mes, indexada por el
    /// gasto del bucle y sin techo. Es el drenaje de 4.15.0, bit a bit.
    ///
    /// Con esta regla los dos [`SpendMode`] COINCIDEN, y no por casualidad: el permitido se define
    /// como el déficit del mes, así que en un mes sin déficit no hay nada que gastar del
    /// patrimonio (`RuleIsSpend` retiraría un importe no positivo, que no es una retirada). Es la
    /// propiedad que mantiene 4.15.0 bit-idéntico bajo cualquiera de los dos modos, y tiene test
    /// propio (`under_fixed_real_both_spend_modes_are_the_same_simulation`).
    FixedReal,
    /// `pct/100 · líquido(k−1) / 12`: porcentaje anual del líquido de cierre del mes anterior.
    PercentOfBalance { pct: Decimal },
    /// `start_pct` hasta el latch de §B.2 (`end_pct·L(k−1) ≥ start_pct·L(R−1)·f(k−1)/f(R−1)`),
    /// luego `end_pct` para siempre.
    Hybrid {
        start_pct: Decimal,
        end_pct: Decimal,
    },
    /// Guyton-Klinger 2006, **solo capital-preservation y prosperity**: la regla de la ventana de
    /// 15 años y el salto de inflación tras un recorte NO están implementados y se declara así en
    /// [`crate::withdrawal`]. Omitirlas deja un modelo más reactivo — la dirección prudente.
    Guardrails {
        pct: Decimal,
        band_pct: Decimal,
        adjust_pct: Decimal,
    },
}

/// Qué gasto rige durante la fase parcial (D10). WP3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpenseBasis {
    /// El gasto de jubilación (default del perfil).
    Retirement,
    /// El gasto regular.
    Regular,
}

/// Media jornada: fase intermedia entre acumulación y jubilación total (P7). **Tipo declarado en
/// WP1b, simulado en WP3**: un plan con `partial: Some(..)` devuelve
/// [`EngineError::UnsupportedPhase`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialPhase {
    /// Mes del bucle (1-based) desde el que rige la fase parcial. Termina en la jubilación total.
    pub start_month: u32,
    /// Ingreso mensual de la fase, PLANO (como los demás ingresos del motor, #139).
    pub income_monthly: Decimal,
    pub expense_basis: ExpenseBasis,
}

/// Pensión pública con FECHA (P2, D3/D8). **Tipo declarado en WP1b, simulada en WP3**: un plan con
/// `pension: Some(..)` devuelve [`EngineError::UnsupportedPhase`].
///
/// Ojo con la rejilla: `start_index` es 0-based —la de `fire_target_at_month_index`, contra la que
/// el bucle evalúa el mes `k` como `k−1`—, NO la 1-based de [`RetirementTrigger::AtMonth`]. Es la
/// asimetría que §B.3 del plan declara; WP3 la mantiene explícita en vez de convertirla a la
/// callada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PensionSchedule {
    pub start_index: u32,
    /// Importe mensual en euros de HOY.
    pub monthly_today: Decimal,
    /// `true` ⇒ se indexa con el mismo factor que el gasto; `false` ⇒ plana (la pensión de
    /// 4.15.0, `FireNeed::ExpenseMinusPension`).
    pub indexed: bool,
    /// Fracción [0,1] de la pensión que se cobra durante la fase parcial.
    pub fraction_while_partial: Decimal,
}

/// Fases del bucle, en orden monótono: una vez avanzada, no se vuelve atrás (latch #141
/// generalizado).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Accumulating,
    Partial,
    Retired,
}

/// El plan de fases de UNA simulación.
///
/// Sustituye a los cuatro campos de jubilación de `ProjectionInput`. Lo consumen el bucle
/// (`project_net_worth_series`) y `first_month_allocation`, que hasta 4.15.0 duplicaban el mismo
/// `if` con dos redacciones distintas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhasePlan {
    pub retirement_trigger: RetirementTrigger,
    /// WP3. `Some(..)` ⇒ [`EngineError::UnsupportedPhase`] en WP1b.
    pub partial: Option<PartialPhase>,
    /// WP3. `Some(..)` ⇒ [`EngineError::UnsupportedPhase`] en WP1b.
    pub pension: Option<PensionSchedule>,
    /// WP2 salvo `FixedReal`.
    pub withdrawal: WithdrawalRule,
    pub spend_mode: SpendMode,
    /// Ingreso que PERSISTE tras la jubilación (rentas, pensión sin fecha declarada). Sustituye a
    /// `income_regular_monthly` desde el primer mes jubilado. Plano (#139).
    pub income_retirement_monthly: Decimal,
    /// Gasto que sigue vivo tras la jubilación (las partidas sin `ends_at_retirement`). Sustituye
    /// a `expense_regular_monthly` desde el primer mes jubilado, y se indexa con el MISMO factor
    /// que el regular.
    pub expense_retirement_monthly: Decimal,
    /// Retirada extra mensual sobre el presupuesto — el antiguo `retirement_monthly_withdrawal`.
    /// **No se retira**: el motor lo soporta desde siempre y `P10_jubilacion_forzada` lo tiene
    /// pineado, aunque la API pase 0 desde que la caída de ingresos es el mecanismo de drenaje.
    pub extra_monthly_withdrawal: Decimal,
}

impl PhasePlan {
    /// El plan que el handler construye HOY: jubilación por cruce, `fixed_real` con techo, sin
    /// fase parcial ni pensión con fecha, sin retirada extra. Bit-idéntico a 4.15.0.
    pub fn classic(
        income_retirement_monthly: Decimal,
        expense_retirement_monthly: Decimal,
    ) -> Self {
        Self {
            retirement_trigger: RetirementTrigger::LiquidCrossing,
            partial: None,
            pension: None,
            withdrawal: WithdrawalRule::FixedReal,
            spend_mode: SpendMode::Ceiling,
            income_retirement_monthly,
            expense_retirement_monthly,
            extra_monthly_withdrawal: Decimal::ZERO,
        }
    }

    /// El plan del antiguo `retirement_start_month = Some(start_month)`: igual que
    /// [`PhasePlan::classic`] pero con jubilación forzada desde ese mes (el cruce sigue vivo y
    /// puede adelantarla) y con la retirada extra que aquel campo acompañaba.
    pub fn forced_at(
        start_month: u32,
        income_retirement_monthly: Decimal,
        expense_retirement_monthly: Decimal,
        extra_monthly_withdrawal: Decimal,
    ) -> Self {
        Self {
            retirement_trigger: RetirementTrigger::AtMonth(start_month),
            extra_monthly_withdrawal,
            ..Self::classic(income_retirement_monthly, expense_retirement_monthly)
        }
    }

    /// Puerta de entrada de las dos funciones que simulan (`project_net_worth_series` y
    /// `first_month_allocation`): lo que WP1b no sabe ejecutar NO se ejecuta.
    pub(crate) fn ensure_supported(&self) -> Result<(), EngineError> {
        if self.partial.is_some() || self.pension.is_some() {
            return Err(EngineError::UnsupportedPhase);
        }
        // WP2: las cuatro reglas se simulan; lo que no pasa son los parámetros imposibles.
        crate::withdrawal::validate_rule(self.withdrawal)
    }
}

/// Avisos del motor. Enum VACÍO a propósito: la lista existe en la salida desde WP1b (para que el
/// handler y los tests no cambien de forma cuando WP3 empiece a llenarla), pero hoy no hay ni un
/// aviso que emitir y un enum con variantes muertas sería una promesa sin código detrás.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineWarning {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_is_the_4_15_semantics() {
        let p = PhasePlan::classic(Decimal::from(800), Decimal::from(2_000));
        assert_eq!(p.retirement_trigger, RetirementTrigger::LiquidCrossing);
        assert_eq!(p.retirement_trigger.forced_month(), None);
        assert_eq!(p.extra_monthly_withdrawal, Decimal::ZERO);
        assert_eq!(p.spend_mode, SpendMode::Ceiling);
        assert!(p.partial.is_none() && p.pension.is_none());
        assert_eq!(p.ensure_supported(), Ok(()));
    }

    #[test]
    fn forced_at_keeps_the_crossing_alive() {
        let p = PhasePlan::forced_at(37, Decimal::ZERO, Decimal::from(2_000), Decimal::from(400));
        assert_eq!(p.retirement_trigger.forced_month(), Some(37));
        assert_eq!(p.extra_monthly_withdrawal, Decimal::from(400));
        assert_eq!(p.ensure_supported(), Ok(()));
    }

    /// Lo no implementado FALLA, no se ignora. Sin este control negativo, `ensure_supported`
    /// sería decorativo y WP3 podría entregar un plan que el motor simula a medias.
    ///
    /// **WP2 movió la frontera**: las cuatro reglas se aceptan (se simulan), y lo que se rechaza
    /// son los parámetros imposibles. Las fases de WP3 siguen fuera.
    #[test]
    fn unsupported_rules_and_phases_are_rejected() {
        let base = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000));

        for rule in [
            WithdrawalRule::PercentOfBalance {
                pct: Decimal::from(4),
            },
            WithdrawalRule::Hybrid {
                start_pct: Decimal::from(5),
                end_pct: Decimal::from(3),
            },
            WithdrawalRule::Guardrails {
                pct: Decimal::from(5),
                band_pct: Decimal::from(20),
                adjust_pct: Decimal::from(10),
            },
        ] {
            let mut p = base.clone();
            p.withdrawal = rule;
            assert_eq!(p.ensure_supported(), Ok(()), "{rule:?} se simula desde WP2");
        }

        // Un porcentaje que no es un porcentaje: error TIPADO, no una simulación creativa.
        let mut p = base.clone();
        p.withdrawal = WithdrawalRule::PercentOfBalance { pct: Decimal::ZERO };
        assert_eq!(
            p.ensure_supported(),
            Err(EngineError::InvalidWithdrawalRule),
            "un 0 % de retirada no es una regla, es una división del plan por cero"
        );

        let mut p = base.clone();
        p.partial = Some(PartialPhase {
            start_month: 12,
            income_monthly: Decimal::from(1_000),
            expense_basis: ExpenseBasis::Retirement,
        });
        assert_eq!(p.ensure_supported(), Err(EngineError::UnsupportedPhase));

        let mut p = base;
        p.pension = Some(PensionSchedule {
            start_index: 240,
            monthly_today: Decimal::from(1_200),
            indexed: true,
            fraction_while_partial: Decimal::ZERO,
        });
        assert_eq!(p.ensure_supported(), Err(EngineError::UnsupportedPhase));
    }
}
