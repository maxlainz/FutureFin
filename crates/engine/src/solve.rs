//! **Inversas del motor** (WP3 de 5.0.0, §B.7 del plan de la issue #207).
//!
//! Todas las preguntas de esta familia tienen la misma forma: «¿qué valor de X hace que la
//! simulación cumpla Y?». Y todas se responden igual — **biseccionando sobre el motor entero**,
//! no sobre una fórmula cerrada que lo aproxime.
//!
//! Esa decisión es el hallazgo **M8** de la revisión adversarial. Un «capital necesario» calculado
//! descontando el objetivo a una tasa escalar es un número plausible que NINGUNA simulación
//! produce: ignora la cascada, los topes de las reglas, el servicio de deuda, los `Próximos`, la
//! fiscalidad del drenaje y el propio latch de jubilación. Aquí cada evaluación de la función
//! objetivo es una `project_net_worth_series` de verdad, y lo que se publica como serie
//! (`required_capital_path`, `coast_path`) es **la serie líquida de esa ejecución**, no una curva
//! dibujada aparte.
//!
//! # Convenciones
//!
//! - `R` (`target_month`) es un mes del BUCLE, 1-based, igual que `retirement_month_index`. El
//!   criterio se evalúa en el índice `R−1` de las series (`liquid_worth[R−1]`), que es el cierre
//!   del mes anterior: exactamente el par `(líquido, objetivo)` con el que el bucle decide el
//!   cruce ese mes.
//! - El objetivo `T(R−1)` sale del evaluador CONSCIENTE DEL PLAN
//!   ([`crate::fire_target_at_month_index_with_plan`]): en las estrategias por edad el objetivo
//!   sigue viajando en el input aunque no dispare nada (`crossing_is_reading_only`), y es contra
//!   ese objetivo contra el que se mide si el hogar llega.
//! - `Ok(None)` significa **«no hay pregunta que responder»** (no hay objetivo evaluable, o `R`
//!   cae fuera de la serie). Nunca es un cero: un cero aquí se leería como «no necesitas aportar
//!   nada», que es la respuesta contraria.
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

use crate::phases::{EngineWarning, IncomePause};
use crate::projection::{
    first_month_allocation, project_net_worth_series, EngineError, ProjectionInput,
    ProjectionOutput,
};
use crate::target::fire_target_at_month_index_with_plan;

/// Tope de iteraciones de cualquier bisección de este módulo (§B.7). No es un umbral de
/// convergencia: es un PRESUPUESTO. El coste de un solve es exactamente este número de
/// proyecciones, y el handler lo paga una vez y lo guarda en la entrada de cache (M4).
pub const MAX_SOLVE_ITERATIONS: u32 = 24;

const TWO: Decimal = Decimal::from_parts(2, 0, 0, false, 0);

/// Resultado de [`required_contribution_monthly`].
#[derive(Debug, Clone)]
pub struct SolveResult {
    /// La aportación mensual constante buscada, en euros nominales. Es un TECHO sobre lo que la
    /// cascada puede invertir cada mes, no un importe que se aporte pase lo que pase: en un mes
    /// con menos sobrante que `contribution`, se aporta el sobrante (R5).
    pub contribution: Decimal,
    /// `true` ⟺ **ni invirtiendo cada euro de sobrante** se alcanza `T(R−1)`. Entonces
    /// `contribution` es [`SolveResult::search_ceiling`]: la respuesta honesta es «todo lo que
    /// tienes, y aun así no llega» (D17, rojo). No es un error: la simulación existe y se publica.
    pub underfunded: bool,
    /// El techo por encima del cual poner techo ES no ponerlo (ver la función privada
    /// `search_ceiling`): el máximo sobrante mensual del horizonte, con el neto recurrente del
    /// mes 1 como suelo. Se publica para que el llamante no tenga que deducirlo — es el
    /// denominador natural de «cuánto de mi margen se está llevando el plan».
    pub search_ceiling: Decimal,
    /// Iteraciones de bisección realmente ejecutadas (0 cuando la respuesta salió de una de las
    /// dos sondas de los extremos).
    pub iterations: u32,
    /// **Serie líquida de la ejecución con `contribution`**, mes a mes (`len == horizon+1`). Es
    /// el «capital necesario» de §B.7, y es una serie SIMULADA: el handler publica
    /// `disponible(k) = líquido_real(k) − required_capital_path(k)`.
    pub required_capital_path: Vec<Decimal>,
    pub warnings: Vec<EngineWarning>,
}

/// Resultado de [`coast_fire_month_index`].
#[derive(Debug, Clone)]
pub struct CoastSolve {
    /// Primer mes del BUCLE (1-based) a partir del cual se puede dejar de aportar y aun así
    /// alcanzar `T(R−1)`. `None` = no existe ninguno **ni siquiera aportando siempre**: el plan
    /// no llega, y se emite [`EngineWarning::CoastNotReachable`].
    pub coast_month_index: Option<u32>,
    /// **El «número coast»**: el patrimonio LÍQUIDO con el que el hogar ENTRA en el mes de coast,
    /// es decir `coast_path[coast_month_index − 1]` — el cierre del mes anterior. Desde ahí, sin
    /// un euro más invertido, la cartera sola alcanza el objetivo en `R−1`.
    ///
    /// Se publica el valor de la SERIE simulada, no un descuento cerrado del objetivo: es el
    /// mismo criterio que `required_capital_path` (M8).
    pub coast_number: Option<Decimal>,
    /// Serie líquida de la ejecución que deja de aportar en `coast_month_index` — la que el chart
    /// dibuja discontinua («si dejas de aportar aquí»). Cuando no hay coast alcanzable, es la
    /// serie de la ejecución que aporta TODOS los meses: la mejor que el plan da.
    pub coast_path: Vec<Decimal>,
    pub iterations: u32,
    pub warnings: Vec<EngineWarning>,
}

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

/// El objetivo del plan en el índice `i`, con el `PhasePlan` del propio input.
fn target_at(input: &ProjectionInput, i: u32) -> Option<Decimal> {
    fire_target_at_month_index_with_plan(input.fire_target.as_ref(), &input.phase_plan, i)
}

fn liquid_at(out: &ProjectionOutput, i: u32) -> Option<Decimal> {
    out.liquid_worth.get(i as usize).copied()
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
/// Con el techo de R5 como cota, `underfunded` se activaría —«ni ahorrando todo llegas»— en
/// hogares cuya simulación REAL sí llega: un rojo falso de D17, que es exactamente la clase de
/// número que esta casa no publica. La cota tiene que CONTENER la respuesta, y la única que lo
/// garantiza es un techo que ningún mes llega a atar.
///
/// El sobrante del mes 1 se conserva como SUELO de la cota (nunca la reduce), para que el
/// intervalo siga conteniendo el caso trivial de un hogar con caja constante.
///
/// **Supuesto declarado**: el sobrante de un mes no depende del valor de la cartera —es
/// `ingreso − gasto − deuda + próximos`—, salvo por la FASE, que sí puede cambiar si el cruce
/// jubila. Por eso este solve pertenece a las estrategias por edad (`crossing_is_reading_only`),
/// donde la fase de cada mes es la misma corra el techo que corra.
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

/// Una ejecución con el techo de aportación fijado a `cap`.
fn run_with_cap(input: &ProjectionInput, cap: Decimal) -> Result<ProjectionOutput, EngineError> {
    let mut scenario = input.clone();
    scenario.phase_plan.contribution_cap_monthly = Some(cap);
    project_net_worth_series(&scenario)
}

/// Una ejecución que deja de aportar desde el mes `stop`.
fn run_stopping_at(input: &ProjectionInput, stop: u32) -> Result<ProjectionOutput, EngineError> {
    let mut scenario = input.clone();
    scenario.phase_plan.contributions_stop_month = Some(stop);
    project_net_worth_series(&scenario)
}

/// **Aportación mensual mínima para llegar al objetivo en `R`** (§B.7, estrategias `retire_at_age`
/// y `partial`).
///
/// Busca la menor `c` tal que, limitando a `c` lo que la cascada invierte cada mes,
/// `líquido(R−1) ≥ T(R−1)`. El intervalo de búsqueda es `[0, techo]` con el techo de
/// `search_ceiling` — el máximo sobrante mensual, **no** el neto recurrente del mes 1: ahí está
/// la decisión que R5 dejaba abierta, tomada con la medición de P9 delante (ver esa función).
///
/// # Monotonía
///
/// `líquido(R−1)` es no decreciente en `c`: subir el techo solo puede aumentar —nunca reducir— el
/// pool que llega a la cascada cada mes (`min(sobrante, c)` es no decreciente en `c`), y dentro de
/// la cascada cada regla resuelve un importe no decreciente en el pool (`Fixed` → `min(a, resto)`,
/// `Percent` → `resto·p/100`, `Remainder` → `resto`, todas no decrecientes, y los topes por activo
/// solo recortan hacia arriba). Más euros invertidos ⇒ valores por activo mayores o iguales en
/// todos los meses siguientes, porque el crecimiento es multiplicativo y positivo y el drenaje
/// solo puede ser menor o igual con más saldo.
///
/// **El argumento de arriba es sobre VALORES, y el criterio es líquido POST-IMPUESTOS.** Ahí se
/// rompe. Tres rendijas declaradas, y por eso la bisección devuelve un extremo VERIFICADO en vez
/// de fiarse del teorema:
///
/// 1. Si la cascada dirige el sobrante a un activo con `is_liquid = false`, aportar más no sube
///    `líquido`: la función se aplana.
/// 2. Sin `crossing_is_reading_only`, aportar más puede ADELANTAR el cruce y con él la caída de
///    ingresos. Esta es la razón por la que el solve pertenece a las estrategias por edad, donde
///    el cruce es lectura.
/// 3. **Se INVIERTE**, y esto lo midió la revisión adversarial contra la afirmación anterior («se
///    aplana, no se invierte»). Subir el techo cambia el MES en que cada tope por activo
///    (`IncomeMultiple`, `MonthsExpense`, `Amount`) se llena, y con él la trayectoria de la BASE
///    DE COSTE. El drenaje tributa a `g_i = clamp(1 − b_i/v_i, 0, 1)`: dos ejecuciones con el
///    mismo valor por activo y distinta base pagan distinto impuesto por el mismo neto, así que
///    aportar MÁS puede dejar el líquido post-impuestos por DEBAJO. Medido en un barrido de 320
///    hogares aleatorios: **35 violaciones de 270 barridos del techo, la peor de 3,4416 €** —
///    unas 5.700 veces la resolución de la bisección (~0,0006 € sobre un techo de 10.000 €/mes).
///    Hacen falta impuestos activados **y** al menos un activo ilíquido: apagar cualquiera de las
///    dos cosas hace desaparecer la inversión en los tres casos peores.
///
/// **El invariante que de verdad se garantiza no es la monotonía: es el extremo verificado.** La
/// bisección devuelve `hi` solo después de comprobar que `hi` CUMPLE, así que el resultado nunca
/// es un falso positivo. Lo que la inversión pone en duda es la MINIMALIDAD, y eso es lo que hay
/// que leer aquí: `c` es la mínima observada por la bisección, no la mínima demostrable. En el
/// eje de `contributions_stop_month` la monotonía sí aguanta: las 41 violaciones medidas son
/// ≤ 3,3e-24, cola de redondeo.
pub fn required_contribution_monthly(
    input: &ProjectionInput,
    target_month: u32,
) -> Result<Option<SolveResult>, EngineError> {
    if target_month == 0 || target_month - 1 > input.horizon_months {
        return Ok(None);
    }
    let i = target_month - 1;
    let Some(t) = target_at(input, i) else {
        return Ok(None);
    };

    let feasible = |out: &ProjectionOutput| liquid_at(out, i).is_some_and(|l| l >= t);

    // Sonda del extremo BAJO: ¿hace falta aportar algo? Su `disposable_cash` es además el
    // sobrante mes a mes, del que sale la cota superior sin pagar otra proyección.
    let zero_out = run_with_cap(input, Decimal::ZERO)?;
    let ceiling = search_ceiling(input, &zero_out)?;
    if feasible(&zero_out) {
        return Ok(Some(SolveResult {
            contribution: Decimal::ZERO,
            underfunded: false,
            search_ceiling: ceiling,
            iterations: 0,
            required_capital_path: zero_out.liquid_worth,
            warnings: Vec::new(),
        }));
    }

    // Sonda del extremo ALTO: ¿llega invirtiendo cada euro de sobrante?
    let full_out = run_with_cap(input, ceiling)?;
    if !feasible(&full_out) {
        return Ok(Some(SolveResult {
            contribution: ceiling,
            underfunded: true,
            search_ceiling: ceiling,
            iterations: 0,
            required_capital_path: full_out.liquid_worth,
            warnings: vec![EngineWarning::RetireAtAgeUnderfunded],
        }));
    }

    // Invariante: `lo` NO cumple, `hi` SÍ cumple. Se devuelve `hi`.
    let mut lo = Decimal::ZERO;
    let mut hi = ceiling;
    let mut best = full_out;
    let mut iterations = 0;
    for _ in 0..MAX_SOLVE_ITERATIONS {
        let mid = (lo + hi) / TWO;
        if mid <= lo || mid >= hi {
            // El intervalo ya no se puede partir en `Decimal`: seguir sería girar en vacío.
            break;
        }
        iterations += 1;
        let out = run_with_cap(input, mid)?;
        if feasible(&out) {
            hi = mid;
            best = out;
        } else {
            lo = mid;
        }
    }

    Ok(Some(SolveResult {
        contribution: hi,
        underfunded: false,
        search_ceiling: ceiling,
        iterations,
        required_capital_path: best.liquid_worth,
        warnings: Vec::new(),
    }))
}

/// **Mes de coast** (§B.7, estrategia `coast`): el primer mes a partir del cual se puede dejar de
/// aportar y aun así alcanzar `T(R−1)`.
///
/// # Monotonía
///
/// `líquido(R−1)` es no decreciente en el mes de corte `k`: parar más tarde significa aportar en
/// un superconjunto de meses, y vale el mismo argumento que en
/// [`required_contribution_monthly`], con las mismas dos rendijas declaradas.
///
/// # `k = R` ES «aportar siempre»
///
/// El criterio se evalúa en `líquido(R−1)`, el cierre del mes `R−1`; lo que se aporte EN el mes
/// `R` ya no entra en ese valor. Por eso la sonda del extremo alto es `stop = R` y no «sin corte»:
/// son la misma ejecución para esta pregunta, y usar `R` mantiene las dos sondas dentro del mismo
/// eje de búsqueda.
pub fn coast_fire_month_index(
    input: &ProjectionInput,
    target_month: u32,
) -> Result<Option<CoastSolve>, EngineError> {
    if target_month == 0 || target_month - 1 > input.horizon_months {
        return Ok(None);
    }
    let i = target_month - 1;
    let Some(t) = target_at(input, i) else {
        return Ok(None);
    };
    let feasible = |out: &ProjectionOutput| liquid_at(out, i).is_some_and(|l| l >= t);

    // Extremo ALTO: aportar todos los meses hasta `R`.
    let full = run_stopping_at(input, target_month)?;
    if !feasible(&full) {
        return Ok(Some(CoastSolve {
            coast_month_index: None,
            coast_number: None,
            coast_path: full.liquid_worth,
            iterations: 0,
            warnings: vec![EngineWarning::CoastNotReachable],
        }));
    }

    // Extremo BAJO: no aportar NUNCA (`stop = 1`, el primer mes del bucle).
    let never = run_stopping_at(input, 1)?;
    if feasible(&never) {
        let coast_number = never.liquid_worth.first().copied();
        return Ok(Some(CoastSolve {
            coast_month_index: Some(1),
            coast_number,
            coast_path: never.liquid_worth,
            iterations: 0,
            warnings: Vec::new(),
        }));
    }

    // Invariante: `lo` NO cumple, `hi` SÍ cumple, y se devuelve `hi`.
    let mut lo = 1u32;
    let mut hi = target_month;
    let mut best = full;
    let mut iterations = 0;
    while hi - lo > 1 && iterations < MAX_SOLVE_ITERATIONS {
        let mid = lo + (hi - lo) / 2;
        iterations += 1;
        let out = run_stopping_at(input, mid)?;
        if feasible(&out) {
            hi = mid;
            best = out;
        } else {
            lo = mid;
        }
    }

    // El número coast: el líquido con el que se ENTRA en el mes de corte, o sea el cierre de
    // `hi − 1`. Con `hi = 1` es el patrimonio de partida (índice 0).
    let coast_number = best.liquid_worth.get((hi - 1) as usize).copied();
    Ok(Some(CoastSolve {
        coast_month_index: Some(hi),
        coast_number,
        coast_path: best.liquid_worth,
        iterations,
        warnings: Vec::new(),
    }))
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
