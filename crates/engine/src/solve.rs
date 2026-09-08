//! **Inversas del motor** (WP3 de 5.0.0, §B.7 del plan de la issue #207; podadas en E4).
//!
//! Todas las preguntas de esta familia tienen la misma forma: «¿qué valor de X hace que la
//! simulación cumpla Y?». Y todas se responden igual — **biseccionando sobre el motor entero**,
//! no sobre una fórmula cerrada que lo aproxime.
//!
//! Esa decisión es el hallazgo **M8** de la revisión adversarial y sigue en pie. Lo que E4 cambió
//! es el CRITERIO: los dos solves que preguntaban «¿llego a `T(R−1)`?» —`required_contribution_monthly`
//! y `coast_fire_month_index`— murieron con el objetivo como decisión (M4). Sus preguntas siguen
//! vivas, pero se responden contra el **umbral de éxito** sobre miles de caminos y viven en
//! `crates/engine-stochastic` (`solve_mc.rs`). Aquí quedan las dos que no miran ningún objetivo:
//! [`max_extra_monthly_expense_keeping_date`] y [`retirement_delay_months`], las dos sobre
//! `retirement_month_index`.
//!
//! Los dos **motores de escenario** ([`run_with_cap`] y [`run_stopping_at`]) se quedan y son
//! públicos: son la plantilla de bisección que el crate estocástico reutiliza —mutar un eje del
//! `PhasePlan` y volver a simular—, y duplicarlos allí sería exactamente la copia que diverge al
//! primer cambio.
//!
//! # Convenciones
//!
//! - `R` es un mes del BUCLE, 1-based, igual que `retirement_month_index`. Un criterio evaluado
//!   «en `R`» se lee en el índice `R−1` de las series (`liquid_worth[R−1]`), el cierre del mes
//!   anterior.
//! - `Ok(None)` significa **«no hay pregunta que responder»**. Nunca es un cero: un cero aquí se
//!   leería como una respuesta, y es la contraria.
//!
//! # La bisección, y qué garantiza de verdad
//!
//! Cada solve mantiene el invariante clásico: **un extremo siempre verificado como bueno y el
//! otro siempre verificado como malo**, y devuelve el extremo BUENO. Eso es más fuerte que
//! confiar en la monotonía: aunque la función objetivo tuviera un tramo no monótono (y la sección
//! de cada solve dice por qué no debería), el valor devuelto está *comprobado* — se ejecutó una
//! simulación completa con él y cumplió el criterio. Lo que la monotonía aporta es la MINIMALIDAD;
//! sin ella, el resultado sigue siendo válido, solo puede no ser el mínimo absoluto.
//!
//! [`MAX_SOLVE_ITERATIONS`] iteraciones = el intervalo se divide por `2²⁴` ≈ 1,7e7: sobre un
//! sobrante de 10.000 €/mes eso es una resolución de 0,0006 €. Más iteraciones no compran nada y
//! cada una cuesta una proyección entera (medido: ~12 ms a 840 meses).

use rust_decimal::Decimal;

use crate::phases::IncomePause;
use crate::projection::{
    first_month_allocation, project_net_worth_series, EngineError, ProjectionInput,
    ProjectionOutput,
};

/// Tope de iteraciones de cualquier bisección de este módulo (§B.7). No es un umbral de
/// convergencia: es un PRESUPUESTO. El coste de un solve es exactamente este número de
/// proyecciones, y el handler lo paga una vez y lo guarda en la entrada de cache (M4).
pub const MAX_SOLVE_ITERATIONS: u32 = 24;

const TWO: Decimal = Decimal::from_parts(2, 0, 0, false, 0);

/// Resultado de [`retirement_delay_months`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetirementDelay {
    pub baseline_month_index: Option<u32>,
    pub paused_month_index: Option<u32>,
    /// `paused − baseline` en meses. **`None` cuando alguno de los dos no se jubila dentro del
    /// horizonte**: «la pausa te saca del horizonte» es una respuesta, pero no es un número de
    /// meses, y publicarlo como un retraso enorme sería inventarse una cifra.
    pub delay_months: Option<i64>,
}

/// El «sobrante» de R5: el neto recurrente del MES 1 (`ingreso − gasto − servicio de deuda`),
/// clampado a ≥ 0. Es lo que una persona reconoce como «lo que me sobra al mes» y lo que el
/// handler publica al lado del resultado.
fn monthly_headroom(input: &ProjectionInput) -> Result<Decimal, EngineError> {
    Ok(first_month_allocation(input)?
        .recurring_net
        .max(Decimal::ZERO))
}

/// **La cota superior de la búsqueda**: el techo por encima del cual poner techo ES no ponerlo.
///
/// `max(sobrante del mes 1, max_k sobrante(k))`, con el sobrante de CADA mes leído de la ejecución
/// con techo 0 — donde `disposable_cash(k)` es, por construcción, la caja positiva entera del mes.
/// No cuesta una proyección extra: es la misma sonda del extremo bajo que el solve ya ejecuta.
///
/// # Por qué NO basta el sobrante del mes 1 (R5), con la medición delante
///
/// R5 fija el «sobrante» como el neto recurrente del mes 1, y para PUBLICARLO es el número
/// correcto. Como cota de búsqueda **no lo es**, y el plan ya dejaba la puerta abierta a decidirlo
/// con evidencia. La evidencia: en P9 (`tests/common/cases.rs`, el hogar realista) el neto
/// recurrente del mes 1 son 500 €/mes, pero su caja mensual crece muy por encima de eso cuando
/// los pasivos se extinguen y los «Próximos» entran. Medido a 600 meses:
///
/// | ejecución | `líquido(599)` |
/// |---|---|
/// | techo = 500 €/mes (el sobrante de R5) | 91.444 € |
/// | sin techo (la cascada de verdad) | 725.197 € |
///
/// Con el techo de R5 como cota, un solve concluiría «ni ahorrando todo llegas» en hogares cuya
/// simulación REAL sí llega: un rojo falso, que es exactamente la clase de número que esta casa
/// no publica. La cota tiene que CONTENER la respuesta, y la única que lo garantiza es un techo
/// que ningún mes llega a atar.
///
/// El sobrante del mes 1 se conserva como SUELO de la cota (nunca la reduce), para que el
/// intervalo siga conteniendo el caso trivial de un hogar con caja constante.
///
/// **Supuesto declarado**: el sobrante de un mes no depende del valor de la cartera —es
/// `ingreso − gasto − deuda + próximos`—, salvo por la FASE, que sí puede cambiar si el cruce
/// jubila.
///
/// Desde E4 su único consumidor en este crate es [`max_extra_monthly_expense_keeping_date`]; los
/// dos solves que biseccionaban sobre un objetivo determinista se retiraron con él (M4).
fn search_ceiling(
    input: &ProjectionInput,
    zero_cap_run: &ProjectionOutput,
) -> Result<Decimal, EngineError> {
    let per_month_max = zero_cap_run
        .disposable_cash
        .iter()
        .copied()
        .fold(Decimal::ZERO, |a, b| a.max(b));
    Ok(monthly_headroom(input)?.max(per_month_max))
}

/// **Una ejecución con el techo de aportación fijado a `cap`.**
///
/// Pública desde E4: es la plantilla de bisección sobre el eje «cuánto aporto» que reutiliza el
/// solve estocástico de aportación mínima (`crates/engine-stochastic`). Una copia allí divergiría
/// del bucle al primer campo nuevo del `PhasePlan`.
pub fn run_with_cap(input: &ProjectionInput, cap: Decimal) -> Result<ProjectionOutput, EngineError> {
    let mut scenario = input.clone();
    scenario.phase_plan.contribution_cap_monthly = Some(cap);
    project_net_worth_series(&scenario)
}

/// **Una ejecución que deja de aportar desde el mes `stop`.**
///
/// Pública desde E4 por la misma razón que [`run_with_cap`]: es el eje sobre el que bisecciona el
/// solve de coast, que ahora vive en `crates/engine-stochastic` con el umbral de éxito como
/// criterio en vez de «líquido(R−1) ≥ T(R−1)».
pub fn run_stopping_at(input: &ProjectionInput, stop: u32) -> Result<ProjectionOutput, EngineError> {
    let mut scenario = input.clone();
    scenario.phase_plan.contributions_stop_month = Some(stop);
    project_net_worth_series(&scenario)
}

/// **Cuánto más puedo gastar sin mover la fecha** (P8.b): el mayor gasto mensual extra CONSTANTE
/// —en euros de hoy, indexado como cualquier gasto del bucle— que deja
/// `retirement_month_index ≤ base + 1`.
///
/// # Qué gasto sube, y qué NO
///
/// El extra se suma a **`expense_regular_monthly`** —el gasto de la fase de ACUMULACIÓN— y a nada
/// más: ni al gasto de jubilación ni a la necesidad que el objetivo FIRE capitaliza.
///
/// Es una decisión, y va declarada: si el gasto extra fuera permanente subiría también la
/// necesidad que el objetivo capitaliza, el objetivo se movería hacia arriba y la respuesta sería
/// mucho menor. La pregunta que P8.b responde es «¿cuánto margen tengo AHORA?» —el margen del
/// hogar mientras trabaja—, no «¿cuánto puedo subir mi nivel de vida para siempre?». La segunda
/// es una pregunta legítima y distinta, y se responde cambiando el presupuesto.
///
/// # Cota superior
///
/// La misma de `search_ceiling`: el máximo sobrante mensual del horizonte. Si ni gastándoselo
/// entero se mueve la fecha —lo normal cuando el trigger es una EDAD, que no depende del gasto—,
/// se devuelve esa cota: es un **suelo honesto** («al menos esto»), no un infinito inventado. Un
/// hogar con capital de sobra podría gastar todavía más y seguir jubilándose el mismo mes; la
/// respuesta no miente sobre eso, simplemente no explora más allá de lo que su caja produce.
///
/// `Ok(None)` = el escenario base no se jubila dentro del horizonte: no hay fecha que conservar.
pub fn max_extra_monthly_expense_keeping_date(
    input: &ProjectionInput,
) -> Result<Option<Decimal>, EngineError> {
    let baseline = project_net_worth_series(input)?;
    let Some(base_month) = baseline.retirement_month_index else {
        return Ok(None);
    };
    let ceiling = base_month.saturating_add(1);

    let keeps_date = |extra: Decimal| -> Result<bool, EngineError> {
        let mut scenario = input.clone();
        scenario.expense_regular_monthly += extra;
        let out = project_net_worth_series(&scenario)?;
        Ok(out.retirement_month_index.is_some_and(|m| m <= ceiling))
    };

    let zero_out = run_with_cap(input, Decimal::ZERO)?;
    let ceiling = search_ceiling(input, &zero_out)?;
    if ceiling <= Decimal::ZERO {
        return Ok(Some(Decimal::ZERO));
    }
    if keeps_date(ceiling)? {
        return Ok(Some(ceiling));
    }

    // Invariante INVERTIDO respecto a los otros solves: aquí `lo` es el bueno (más gasto es
    // peor), así que se devuelve `lo`. `lo = 0` cumple por construcción (`base_month ≤ base+1`).
    let mut lo = Decimal::ZERO;
    let mut hi = ceiling;
    for _ in 0..MAX_SOLVE_ITERATIONS {
        let mid = (lo + hi) / TWO;
        if mid <= lo || mid >= hi {
            break;
        }
        if keeps_date(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(Some(lo))
}

/// **Cuánto retrasa la jubilación una pausa de ingresos** (P8.c): dos simulaciones, la de base y
/// la que multiplica el ingreso ganado por `pause.income_fraction` durante su ventana.
///
/// No hay bisección: la pregunta ya trae el valor de la incógnita. Lo que se publica son los dos
/// meses y su diferencia, para que nadie tenga que deducir de un delta si alguno de los dos
/// escenarios simplemente no se jubila.
pub fn retirement_delay_months(
    input: &ProjectionInput,
    pause: IncomePause,
) -> Result<RetirementDelay, EngineError> {
    let baseline = project_net_worth_series(input)?.retirement_month_index;
    let mut scenario = input.clone();
    scenario.phase_plan.income_pause = Some(pause);
    let paused = project_net_worth_series(&scenario)?.retirement_month_index;
    let delay_months = match (baseline, paused) {
        (Some(a), Some(b)) => Some(i64::from(b) - i64::from(a)),
        _ => None,
    };
    Ok(RetirementDelay {
        baseline_month_index: baseline,
        paused_month_index: paused,
        delay_months,
    })
}
