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

/// Media jornada: fase intermedia entre acumulación y jubilación total (P7). **Simulada desde
/// WP3**: desde `start_month` y hasta el mes efectivo de jubilación el hogar cobra
/// `income_monthly` (plano) en vez de su ingreso regular y gasta el que diga `expense_basis`,
/// indexado como todos los gastos del bucle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialPhase {
    /// Mes del bucle (1-based) desde el que rige la fase parcial. Termina en la jubilación total.
    pub start_month: u32,
    /// Ingreso mensual de la fase, PLANO (como los demás ingresos del motor, #139).
    pub income_monthly: Decimal,
    pub expense_basis: ExpenseBasis,
}

/// Pensión pública con FECHA (P2, D3/D8). **Simulada desde WP3**: es INGRESO en cualquier fase
/// desde `start_index` y, a la vez, entra en la NECESIDAD que el objetivo capitaliza (§B.3).
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

impl PensionSchedule {
    /// Importe mensual de la pensión en el índice **0-based** `i` (el que el bucle evalúa como
    /// `k−1`): `0` antes de `start_index`, `monthly_today·f(i)` si está indexada y
    /// `monthly_today` si es plana.
    ///
    /// `f` se pasa YA EVALUADO por el llamante — el bucle ya lo tiene en la mano (`expense_factor`)
    /// y el objetivo lo evalúa con SU inflación: duplicar aquí la llamada a
    /// `inflation_factor_at_month_index` volvería a crear la fórmula doble que #139 cerró.
    ///
    /// Un importe negativo se lee como 0: una pensión negativa no es una pensión.
    /// **5.0.0 WP5.5**: delega en el gemelo genérico
    /// ([`crate::sim::PensionScheduleG::monthly_at`]) — una sola definición de `P_m(i)`, la que
    /// el núcleo ejecuta. La cota `[0, 1]` de la fracción parcial vive allí por la misma razón.
    pub fn monthly_at(self, i: u32, inflation_factor: Decimal) -> Decimal {
        self.to_generic::<Decimal>()
            .monthly_at(i, inflation_factor)
    }
}

/// **Puerta de TASA INICIAL** (5.0.0 E1, correcciones C1/C2 del panel adversarial).
///
/// El SWR deja de ser una comprobación mensual sobre el saldo vivo y pasa a ser lo que la
/// literatura dice que es: la **tasa de retirada INICIAL máxima**, evaluada UNA sola vez, en el
/// mes `R` en que el hogar se jubila, sobre el patrimonio LÍQUIDO con el que entra (`L(R−1)`).
///
/// Se mide contra la **necesidad ORDINARIA anual** de ese mes —gasto de jubilación + retirada
/// extra − ingresos del mes (pensión con fecha incluida si ya se cobra)—, sin servicio de deuda
/// y sin «Próximos»: la regla del 4 % habla del gasto de vivir, no de una cuota que se extingue
/// ni de un ingreso puntual.
///
/// `None` ⇒ **no hay puerta**: ninguna comparación se ejecuta y ningún camino puede fallar por
/// F2. Es el valor de los dos constructores ([`PhasePlan::classic`] y [`PhasePlan::forced_at`]),
/// y por eso los pines de 4.15.0 no se mueven.
///
/// Medido (panel adversarial, 2026-09-06): la alternativa —comparar la venta ordinaria del mes
/// contra `SWR/12 · L(k−1)` TODOS los meses— ponía la fecha válida de la demo en la edad de la
/// pensión (72 años) y el capital necesario en 56 M€, porque cualquier caída del saldo convierte
/// un plan sano en un fallo retroactivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialRateGate {
    /// Tasa máxima ANUAL en % sobre el líquido (3,5 = 3,5 %). Es el `swr_pct` de la instalación.
    pub swr_pct: Decimal,
    /// Puente hasta la pensión (C2): un tope MAYOR con fecha límite. `None` ⇒ el SWR rige desde
    /// el primer mes jubilado.
    pub bridge: Option<BridgeCap>,
}

/// **El puente, como tope de tasa inicial con fecha límite** (C2).
///
/// Con una pensión con fecha a pocos años vista, exigir el SWR de una perpetuidad es exigir
/// capital para un gasto que la pensión va a cubrir. El puente sube el tope de `R` a `max_pct`
/// **solo** si la pensión llega dentro de `max_years` años. No hay tope mensual DURANTE el
/// puente: lo que quede después lo juzgan F1 y F3 mes a mes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeCap {
    /// Tasa inicial máxima ANUAL en % mientras el puente aplica (típicamente > `swr_pct`).
    pub max_pct: Decimal,
    /// Años máximos de puente. La pensión tiene que llegar dentro de esta ventana desde `R`.
    pub max_years: u32,
}

/// **Por qué falla UN camino** (5.0.0 E1, decisión M3 del owner corregida por C1).
///
/// El veredicto de un camino es binario —falla o no—, y su MOTIVO tiene tres formas que no
/// comparten remedio. Se evalúan solo en meses de jubilación (o de media jornada, y ahí solo
/// F1: supuesto S1, las reglas de retirada se anclan en `L(R−1)`, que durante la media jornada
/// todavía no existe), con prioridad F1 > F2 > F3 dentro del mismo mes.
///
/// **El veredicto de un camino no es el veredicto del PLAN.** El plan se juzga con la proporción
/// de caminos sin fallo (`crates/engine-stochastic`) contra el umbral del perfil; un camino
/// determinista que falla no dice más que «este escenario concreto no aguanta».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFailure {
    /// **F1** — la cartera no pudo cubrir la necesidad del mes: hubo venta sin fundar y quedó
    /// necesidad NETA sin cubrir. Es el fallo literal («te quedas sin dinero»).
    PortfolioDepleted,
    /// **F2** — la tasa de retirada INICIAL supera el tope ([`InitialRateGate`]) en el primer
    /// mes jubilado. No se vuelve a evaluar: es una propiedad de la FECHA, no del mes.
    InitialRateExceeded,
    /// **F3** — con una regla por saldo (`percent_of_balance`, `hybrid`, `guardrails`), lo que
    /// la regla PERMITE se queda por debajo de la necesidad ordinaria del mes: el hogar tendría
    /// que recortar su nivel de vida. No aplica a `fixed_real`, donde el permitido ES la
    /// necesidad por construcción.
    RuleBelowNeed,
}

impl PathFailure {
    /// Literal público y estable de cada motivo — el que la API publica. Vive aquí por la misma
    /// razón que [`EngineWarning::code`]: un `match` duplicado en `apps/api` se queda atrás.
    pub fn code(self) -> &'static str {
        match self {
            PathFailure::PortfolioDepleted => "portfolio_depleted",
            PathFailure::InitialRateExceeded => "initial_rate_exceeded",
            PathFailure::RuleBelowNeed => "rule_below_need",
        }
    }
}

/// Pausa de ingresos (P8.c): el ingreso GANADO del hogar se multiplica por `income_fraction`
/// durante `months` meses a partir de `from_month` (1-based, ventana SEMIABIERTA:
/// `from_month ≤ k < from_month + months`).
///
/// **La pensión NO se pausa**: una excedencia interrumpe el trabajo, no la pensión pública ni las
/// rentas que el plan declara como ingreso de jubilación… salvo que la pausa caiga en una fase
/// donde el ingreso ES el de jubilación, en cuyo caso sí lo multiplica (es el ingreso ganado de
/// esa fase). Lo que nunca escala es el término de pensión con fecha, que se suma DESPUÉS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncomePause {
    pub from_month: u32,
    pub months: u32,
    /// Multiplicador del ingreso durante la ventana (`0` = sin ingreso, `0,5` = media paga).
    /// Negativo se lee como 0: el motor es una función pura y su firma admite cualquier
    /// `Decimal`, pero un ingreso negativo no es una pausa.
    pub income_fraction: Decimal,
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
    /// Media jornada (WP3). `None` = el hogar pasa de acumular a jubilarse sin escala.
    pub partial: Option<PartialPhase>,
    /// Pensión con fecha (WP3). `None` = no hay pensión con calendario propio; la pensión PLANA
    /// de 4.15.0 sigue viajando dentro de `income_retirement_monthly` y de
    /// `FireNeed::ExpenseMinusPension`.
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
    // -----------------------------------------------------------------------------------------
    // WP3 (§B.3, §B.7, D17). Todos ADITIVOS y con default en los dos constructores: un plan
    // construido con `classic`/`forced_at` sigue siendo la semántica de 4.15.0 campo a campo.
    // -----------------------------------------------------------------------------------------
    /// **D17, «un solo trigger por simulación»**: con `true` el cruce `líquido(k−1) ≥ objetivo(k−1)`
    /// NO jubila — solo se anota como
    /// [`crate::ProjectionOutput::liquid_crossing_month_index`]—, y quien jubila es
    /// exclusivamente [`RetirementTrigger::AtMonth`].
    ///
    /// Existe porque las estrategias por edad SIGUEN necesitando el objetivo (el chart lo pinta y
    /// el infra-financiado se mide contra él): pasarle `fire_target: None` al motor para
    /// desactivar el cruce —la vía que WP1b anticipaba— tiraría también la lectura. `false` por
    /// defecto, y por eso `P10_jubilacion_forzada` (mes forzado, sin objetivo) sigue pineado.
    pub crossing_is_reading_only: bool,
    /// Techo mensual CONSTANTE de lo que la cascada puede invertir (§B.7). `None` = sin techo (el
    /// sobrante entero se reparte, como siempre). Con `Some(c)` el pool que llega a la cascada es
    /// `min(sobrante, c)` y el resto **no se invierte**: sale del balance y se publica en
    /// [`crate::ProjectionOutput::disposable_cash`].
    ///
    /// No es un ajuste de producto: es la palanca sobre la que bisecan los solves — desde E4 de
    /// 5.0.0, los ESTOCÁSTICOS (`crates/engine-stochastic`), que biseccionan sobre el umbral de
    /// éxito en vez de sobre un objetivo determinista.
    pub contribution_cap_monthly: Option<Decimal>,
    /// Mes (1-based) a partir del cual **no se aporta nada** — el techo efectivo pasa a 0 desde
    /// `k ≥ contributions_stop_month`. Es la palanca del solve de coast (que desde E4 vive en
    /// `crates/engine-stochastic`).
    pub contributions_stop_month: Option<u32>,
    /// Pausa de ingresos (P8.c). `None` = sin pausa, y entonces el ingreso del mes no pasa por
    /// ninguna multiplicación (bit-identidad).
    pub income_pause: Option<IncomePause>,
    /// **Puerta de tasa inicial** (E1, C1/C2). `None` ⇒ sin puerta: el bucle no ejecuta ninguna
    /// comparación y ningún camino puede fallar por [`PathFailure::InitialRateExceeded`]. Es el
    /// default de los dos constructores, y por eso los pines de 4.15.0 no se mueven.
    pub initial_rate: Option<InitialRateGate>,
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
            // WP3: todos los ejes nuevos, apagados. `classic` sigue siendo 4.15.0 campo a campo.
            crossing_is_reading_only: false,
            contribution_cap_monthly: None,
            contributions_stop_month: None,
            income_pause: None,
            // Sin puerta de tasa inicial: 4.15.0 no la tenía y ningún pin suyo puede moverse por
            // una comparación que no se ejecuta.
            initial_rate: None,
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

    // **5.0.0 WP5.5 — los tres helpers de comportamiento se mudaron al gemelo genérico**
    // (`crate::sim::PhasePlanG`): `ensure_supported`, `contribution_cap_at` y
    // `partial_expense_basis_monthly` son lo que EJECUTA el núcleo, y el núcleo ya no habla
    // `Decimal`. Este tipo se queda como lo que siempre fue de cara afuera: la forma pública del
    // plan, la que `apps/api` construye.
}

/// Avisos del motor. WP3 le puso tres variantes y el modelo v2 le quitó dos, las dos por la
/// misma razón: **un booleano colgado de un objetivo determinista no es un veredicto**.
///
/// - **E1 retiró `RetireAtAgeUnderfunded`**: «me jubilo por edad y el capital no llega» pasó a ser
///   una probabilidad —`1 − éxito(R)`, que mide el crate estocástico sobre miles de caminos—. Un
///   aviso que decía «no llegas» comparando el líquido con una perpetuidad no distinguía «no
///   llegas por poco» de «no llegas jamás», que es justo la distinción que el owner pidió.
/// - **E4 retiró `CoastNotReachable`**: su emisor era `coast_fire_month_index`, cuyo criterio
///   («líquido(R−1) ≥ T(R−1)») desapareció con el objetivo como decisión. El solve de coast vive
///   ahora en `crates/engine-stochastic` con su propio enum de avisos, porque quien puede decir
///   «no se puede parar» es el sorteo, no el motor determinista.
///
/// Los avisos de ensamblado (`birth_date_missing` y compañía) los añade el handler: el motor no
/// conoce fechas de nacimiento.
///
/// Un aviso NO es un error: la simulación se publica igual. Lo que dice es que el plan
/// configurado tiene una consecuencia que el usuario no vería mirando solo la curva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineWarning {
    /// Durante la media jornada el patrimonio LÍQUIDO bajó de un mes al siguiente: la fase se
    /// está comiendo el capital en vez de dejarlo crecer.
    PartialPhaseCapitalShrinking,
}

impl EngineWarning {
    /// Literal público y estable de cada aviso — el que la API publica en `warnings[]`.
    ///
    /// Vive AQUÍ y no en el handler para que el mapeo sea único: un `match` duplicado en
    /// `apps/api` se quedaría atrás en cuanto este enum crezca, y un aviso con dos nombres es un
    /// aviso que nadie puede buscar.
    pub fn code(self) -> &'static str {
        match self {
            EngineWarning::PartialPhaseCapitalShrinking => "partial_phase_capital_shrinking",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::EngineError;
    use crate::sim::PhasePlanG;

    /// **WP5.5**: el COMPORTAMIENTO del plan vive en el gemelo genérico (es lo que el núcleo
    /// ejecuta); este helper es la conversión, que es una copia campo a campo.
    fn g(p: &PhasePlan) -> PhasePlanG<Decimal> {
        PhasePlanG::from(p)
    }

    #[test]
    fn classic_is_the_4_15_semantics() {
        let p = PhasePlan::classic(Decimal::from(800), Decimal::from(2_000));
        assert_eq!(p.retirement_trigger, RetirementTrigger::LiquidCrossing);
        assert_eq!(p.retirement_trigger.forced_month(), None);
        assert_eq!(p.extra_monthly_withdrawal, Decimal::ZERO);
        assert_eq!(p.spend_mode, SpendMode::Ceiling);
        assert!(p.partial.is_none() && p.pension.is_none());
        assert_eq!(g(&p).ensure_supported(), Ok(()));
    }

    #[test]
    fn forced_at_keeps_the_crossing_alive() {
        let p = PhasePlan::forced_at(37, Decimal::ZERO, Decimal::from(2_000), Decimal::from(400));
        assert_eq!(p.retirement_trigger.forced_month(), Some(37));
        assert_eq!(p.extra_monthly_withdrawal, Decimal::from(400));
        assert_eq!(g(&p).ensure_supported(), Ok(()));
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
            assert_eq!(g(&p).ensure_supported(), Ok(()), "{rule:?} se simula desde WP2");
        }

        // Un porcentaje que no es un porcentaje: error TIPADO, no una simulación creativa.
        let mut p = base.clone();
        p.withdrawal = WithdrawalRule::PercentOfBalance { pct: Decimal::ZERO };
        assert_eq!(
            g(&p).ensure_supported(),
            Err(EngineError::InvalidWithdrawalRule),
            "un 0 % de retirada no es una regla, es una división del plan por cero"
        );

        // **WP3 movió la frontera otra vez**: las dos fases se simulan y ya no se rechazan.
        let mut p = base.clone();
        p.partial = Some(PartialPhase {
            start_month: 12,
            income_monthly: Decimal::from(1_000),
            expense_basis: ExpenseBasis::Retirement,
        });
        assert_eq!(g(&p).ensure_supported(), Ok(()));

        let mut p = base;
        p.pension = Some(PensionSchedule {
            start_index: 240,
            monthly_today: Decimal::from(1_200),
            indexed: true,
            fraction_while_partial: Decimal::ZERO,
        });
        assert_eq!(g(&p).ensure_supported(), Ok(()));
    }

    /// La rejilla de la pensión es 0-based y el corte es `i ≥ start_index` — no `>`.
    #[test]
    fn pension_starts_exactly_at_its_index() {
        let p = PensionSchedule {
            start_index: 240,
            monthly_today: Decimal::from(1_200),
            indexed: true,
            fraction_while_partial: d(5, 1),
        };
        assert_eq!(p.monthly_at(239, Decimal::from(2)), Decimal::ZERO);
        assert_eq!(p.monthly_at(240, Decimal::from(2)), Decimal::from(2_400));
        assert_eq!(p.monthly_at(241, Decimal::ONE), Decimal::from(1_200));

        let flat = PensionSchedule { indexed: false, ..p };
        assert_eq!(flat.monthly_at(240, Decimal::from(2)), Decimal::from(1_200));

        // Clamps declarados: fracción fuera de [0,1] e importe negativo.
        assert_eq!(p.to_generic::<Decimal>().partial_fraction(), d(5, 1));
        assert_eq!(
            PensionSchedule { fraction_while_partial: Decimal::from(3), ..p }.to_generic::<Decimal>().partial_fraction(),
            Decimal::ONE
        );
        assert_eq!(
            PensionSchedule { monthly_today: Decimal::from(-100), ..p }
                .monthly_at(240, Decimal::ONE),
            Decimal::ZERO
        );
    }

    /// El techo de aportación y el corte de coast, con el corte mandando sobre el techo.
    #[test]
    fn contribution_cap_and_stop_month() {
        let mut p = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000));
        assert_eq!(g(&p).contribution_cap_at(1), None, "sin techo por defecto");
        p.contribution_cap_monthly = Some(Decimal::from(500));
        assert_eq!(g(&p).contribution_cap_at(1), Some(Decimal::from(500)));
        p.contributions_stop_month = Some(60);
        assert_eq!(g(&p).contribution_cap_at(59), Some(Decimal::from(500)));
        assert_eq!(g(&p).contribution_cap_at(60), Some(Decimal::ZERO));
        // Un techo negativo se lee como 0, no como «sin techo».
        p.contributions_stop_month = None;
        p.contribution_cap_monthly = Some(Decimal::from(-1));
        assert_eq!(g(&p).contribution_cap_at(1), Some(Decimal::ZERO));
    }

    /// La ventana de la pausa es SEMIABIERTA y `None` fuera significa «no multipliques».
    #[test]
    fn income_pause_window_is_half_open() {
        let pause = IncomePause {
            from_month: 10,
            months: 3,
            income_fraction: d(5, 1),
        };
        assert_eq!(pause.to_generic::<Decimal>().factor_at(9), None);
        assert_eq!(pause.to_generic::<Decimal>().factor_at(10), Some(d(5, 1)));
        assert_eq!(pause.to_generic::<Decimal>().factor_at(12), Some(d(5, 1)));
        assert_eq!(pause.to_generic::<Decimal>().factor_at(13), None, "10, 11 y 12: tres meses, no cuatro");
        assert_eq!(
            IncomePause { months: 0, ..pause }.to_generic::<Decimal>().factor_at(10),
            None,
            "una pausa de cero meses no es una pausa"
        );
        assert_eq!(
            IncomePause { income_fraction: Decimal::from(-1), ..pause }.to_generic::<Decimal>().factor_at(10),
            Some(Decimal::ZERO)
        );
    }

    /// Los literales de los avisos son contrato público: si alguien renombra una variante, el
    /// literal NO puede cambiar sin que este test lo diga.
    #[test]
    fn warning_codes_are_stable_literals() {
        assert_eq!(
            EngineWarning::PartialPhaseCapitalShrinking.code(),
            "partial_phase_capital_shrinking"
        );
    }

    /// Los literales de los motivos de fallo son contrato público igual que los de los avisos:
    /// la API los publica tal cual y la SPA los traduce por su nombre.
    #[test]
    fn path_failure_codes_are_stable_literals() {
        assert_eq!(PathFailure::PortfolioDepleted.code(), "portfolio_depleted");
        assert_eq!(
            PathFailure::InitialRateExceeded.code(),
            "initial_rate_exceeded"
        );
        assert_eq!(PathFailure::RuleBelowNeed.code(), "rule_below_need");
    }

    fn d(m: i64, s: u32) -> Decimal {
        Decimal::new(m, s)
    }
}
