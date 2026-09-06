//! **Los tres solves de ESTRATEGIA** (WP E8 de 5.0.0; decisiones M10/M11/M12 del owner y
//! corrección C8 del panel adversarial de 2026-09-06).
//!
//! `solve_mc` responde «¿cuándo me puedo jubilar?». Aquí viven las tres preguntas que una
//! estrategia concreta añade a esa, y las tres se responden con el MISMO criterio —el umbral de
//! éxito sobre miles de caminos, [`SuccessAt::meets`]— y la MISMA doctrina de bisección:
//!
//! | pregunta | estrategia | eje sobre el que se bisecciona | función |
//! |---|---|---|---|
//! | «me jubilo a los 60: ¿cuánto me falta aportar?» | `retire_at_age`, `coast` (modo A) | un extra mensual `c` inyectado en «Próximos» | [`minimum_extra_contribution`] |
//! | «¿cuándo puedo dejar de aportar?» | `coast` (modo A) | el mes `C` de `contributions_stop_month` | [`coast_stop_month`] |
//! | «¿cuándo puedo bajar a media jornada?» | `partial` (modo «en cuanto pueda») | el mes `S` de `PartialPhase::start_month` | [`earliest_partial_start`] |
//!
//! # La doctrina, que no cambia
//!
//! 1. **Bisección sobre el motor entero** (hallazgo M8): ninguna de las tres tiene forma cerrada y
//!    es deliberado. Un «cuánto te falta» descontado a una tasa escalar ignora la cascada, los
//!    topes por regla, el servicio de deuda, los «Próximos», la fiscalidad del drenaje y el propio
//!    latch de fases — sería un número plausible que ninguna simulación produce.
//! 2. **El extremo VERIFICADO**: cada bisección mantiene «`lo` comprobado MALO, `hi` comprobado
//!    BUENO» y devuelve `hi`. Lo que se publica se ejecutó y cumplió. Agotar el presupuesto no
//!    invalida nada: cuesta MINIMALIDAD, no validez.
//! 3. **Presupuesto de iteraciones, no umbral de convergencia**: el coste de cada solve es un
//!    número conocido de sorteos, y los cuatro presupuestos están escritos como constantes de este
//!    módulo.
//! 4. **La monotonía no se supone.** Vale aquí todo lo que dice el doc de `solve_mc`, y encima
//!    hay una vía propia: subir el techo de aportación cambia el MES en que cada tope por activo
//!    se llena y con él la trayectoria de la BASE DE COSTE, así que con impuestos activos y algún
//!    activo ilíquido el líquido post-impuestos puede retroceder (medido: 35 violaciones de 270
//!    barridos, la peor de 3,44 €). No compromete nada porque el extremo devuelto está comprobado.
//!
//! # Una bisección, escrita UNA vez
//!
//! Los tres solves comparten [`bisect`], genérico sobre el eje: el mes lo parte [`month_mid`] y el
//! importe [`amount_mid`]. `solve_mc::bisect_month` no se reutiliza porque es **privado** de ese
//! módulo y E8 no puede tocarlo; lo que sí se reutiliza —y es lo que importa— es su INVARIANTE,
//! escrito aquí una sola vez para los tres ejes en vez de tres veces.
//!
//! # De aquí SÍ salen euros, y hay que decir de dónde
//!
//! `solve_mc` cierra su doc con «de aquí no sale un euro». **Este módulo publica tres importes**
//! ([`ContributionSolve::extra_monthly`], [`ContributionSolve::search_ceiling`] y
//! [`CoastSolve::freed_saving_monthly`]) y la regla del crate sigue intacta, porque ninguno de los
//! tres sale del camino de coma flotante:
//!
//! - `extra_monthly` y `search_ceiling` son `Decimal` que **este módulo construye** (el suelo de
//!   100 €, el sobrante del mes 1 leído de `first_month_allocation`, doblajes y medias exactas).
//!   El sorteo no los calcula: los JUZGA. Lo que la coma flotante decide es *qué escenario cumple*,
//!   nunca *cuánto vale*.
//! - `freed_saving_monthly` sale de una ejecución **determinista** del motor
//!   (`run_stopping_at`, aritmética `Decimal` de principio a fin), no de un camino sorteado.
//!
//! Dicho al revés: si alguien borrara [`crate::F64Money`] y evaluara el criterio a mano, los tres
//! importes serían **los mismos**. Por eso no hay excepción que declarar en D4.
//!
//! # Coste
//!
//! Con los presupuestos del plan (buscar 500 caminos, confirmar 2.500; ≈ 105 ms y ≈ 455 ms por
//! sorteo sobre P9 a 840 meses):
//!
//! ```text
//!   aportación mínima   típico ~16 búsqueda + 1 confirmación ≈ 2,1 s   (cota: 26 + 7 ≈ 5,9 s)
//!   coast               típico  ~8 búsqueda + 1 confirmación ≈ 1,3 s   (cota: 14 + 7 ≈ 4,7 s)
//!   jornada reducida    típico ~10 + 1 propios + UNA fecha   ≈ 3,4 s   (cota: 14 + 7 + 39)
//! ```
//!
//! Lo mide, sin afirmarlo, `tests/timing_mc.rs::the_three_strategy_solves_cost_what_the_plan_says`.
//!
//! **La maquinaria del sorteo se reconstruye en cada evaluación, y aquí no hay alternativa.**
//! `solve_mc` puede sostener un `PathEngine` durante todo un solve porque entre dos evaluaciones
//! solo cambia `retirement_trigger`, que se reescribe en sitio. Los tres ejes de este módulo
//! —«Próximos», `contributions_stop_month`, `PartialPhase::start_month`— viven en el `SimInput`
//! CONVERTIDO, así que cada candidato es una entrada distinta: se pasa por [`success_at_month`],
//! que construye el motor y lo tira. El sobrecoste es la conversión de la entrada más un buffer de
//! `meses × activos`, frente a 500 caminos de simulación: por debajo del 1 %.

use futurefin_engine::{first_month_allocation, run_stopping_at, ProjectionInput};
use rust_decimal::Decimal;

use crate::mc::PathEngine;
use crate::solve_mc::{
    retiring_at, success_at_month, valid_retirement_month, RetirementDateSolve, SuccessAt,
};
use crate::{McConfig, McError};

// =================================================================================================
// Presupuestos y constantes
// =================================================================================================

/// Doblajes máximos del extremo ALTO de [`minimum_extra_contribution`]. Desde el suelo de
/// [`CONTRIBUTION_SEED_FLOOR`] eso son `100 · 2¹² = 409.600 €/mes`: un techo que ningún hogar de
/// esta app alcanza, y que existe para que «no llegas» sea una conclusión ACOTADA y no un bucle.
pub const MAX_CONTRIBUTION_DOUBLINGS: u32 = 12;

/// Pasos máximos de la bisección sobre el IMPORTE. Doce halvings dividen el intervalo por 4.096:
/// sobre un bracket de 8.000 €/mes eso es una resolución de 1,95 €, muy por debajo de la decena de
/// euro con la que se publica ([`round_up_to_tens`]).
pub const MAX_CONTRIBUTION_BISECTION_DRAWS: u32 = 12;

/// Pasos máximos de la bisección sobre el MES, en [`coast_stop_month`] y en
/// [`earliest_partial_start`]. `log₂ 840 ≈ 9,7`: doce cubren el horizonte entero de la app con
/// margen y dejan el intervalo en un solo mes.
pub const MAX_MONTH_BISECTION_DRAWS: u32 = 12;

/// **Avances de la CONFIRMACIÓN**, comunes a los tres solves: cuando el presupuesto grande
/// desmiente al de búsqueda, se avanza hasta seis veces hacia el extremo que la búsqueda verificó
/// como bueno (+5 % en el importe, +1 mes en los dos ejes de mes) y se confirma otra vez.
///
/// Es la fase D de `valid_retirement_month` con otro nombre y otro eje, y existe por la misma
/// razón: los 500 caminos de la búsqueda son un **prefijo** de los 2.500 de la confirmación
/// (números aleatorios comunes), así que la confirmación puede desmentir a la búsqueda sin
/// contradecirla. Si tampoco cierra, se publica la última medición **tal cual**: el llamante ve
/// que `success_at_solution` no cumple el umbral, que es la verdad.
pub const MAX_STRATEGY_CONFIRMATION_ADVANCES: u32 = 6;

/// Suelo del primer extremo alto de la aportación, en euros/mes. Un hogar sin sobrante ninguno
/// (`recurring_net ≤ 0`) tiene que empezar a doblar desde ALGO, y 100 €/mes es una aportación que
/// una persona reconoce; empezar en el sobrante puro dejaría el arranque en 0 y el doblaje sería
/// un bucle que no avanza.
pub const CONTRIBUTION_SEED_FLOOR: Decimal = Decimal::from_parts(100, 0, 0, false, 0);

/// Paso de PUBLICACIÓN de la aportación: la decena de euro. Se redondea **hacia arriba** (nunca
/// hacia el número más cercano) y se CONFIRMA después de redondear, para que lo publicado sea
/// exactamente lo medido — no una versión bonita de otra cifra.
pub const CONTRIBUTION_ROUNDING_STEP: Decimal = Decimal::from_parts(10, 0, 0, false, 0);

const TWO: Decimal = Decimal::from_parts(2, 0, 0, false, 0);
/// `1,05` — el avance del +5 % de la confirmación del importe.
const ONE_PLUS_FIVE_PERCENT: Decimal = Decimal::from_parts(105, 0, 0, false, 2);

// =================================================================================================
// Avisos
// =================================================================================================

/// **Los cuatro avisos de estrategia.** Un aviso NO es un error: el solve se publica igual. Lo que
/// dice es que la estrategia configurada tiene una consecuencia que el usuario no vería mirando
/// solo la cifra.
///
/// Los literales de [`StrategySolveWarning::code`] son **contrato de cable**: la API los publica
/// tal cual en `warnings[]` y la SPA los traduce. Viven aquí —y no en el handler— por la misma
/// razón que `EngineWarning::code`: un `match` duplicado en `apps/api` se queda atrás en cuanto el
/// enum crece, y un aviso con dos nombres es un aviso que nadie puede buscar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategySolveWarning {
    /// **No se puede dejar de aportar**: ni aportando durante toda la acumulación (`C = R`) el
    /// plan cumple el umbral. Lo emite [`coast_stop_month`].
    CoastNotReachable,
    /// **La media jornada no puede empezar**: la fase falla incluso empezando en el último mes del
    /// horizonte. Lo emite [`earliest_partial_start`].
    PartialNeverStarts,
    /// **Se puede empezar la media jornada, pero no jubilarse del todo**: hay un `S` válido y, con
    /// la fase dentro, ningún mes del horizonte cumple el umbral. Lo emite
    /// [`earliest_partial_start`] a partir de la fecha anidada.
    PartialNeverFullyRetires,
    /// **Me jubilo a esa edad y el capital no llega**: ni el techo de búsqueda de aportación hace
    /// que el plan cumpla el umbral. Lo deriva [`ContributionSolve::warning`] de
    /// [`ContributionSolve::underfunded`] — es el sucesor probabilístico del `EngineWarning`
    /// homónimo que E1 retiró del motor por comparar un líquido contra una perpetuidad.
    RetireAtAgeUnderfunded,
}

impl StrategySolveWarning {
    /// Literal público y estable. **Contrato de cable** — no se renombra sin migrar la SPA.
    pub fn code(self) -> &'static str {
        match self {
            StrategySolveWarning::CoastNotReachable => "coast_not_reachable",
            StrategySolveWarning::PartialNeverStarts => "partial_never_starts",
            StrategySolveWarning::PartialNeverFullyRetires => "partial_never_fully_retires",
            StrategySolveWarning::RetireAtAgeUnderfunded => "retire_at_age_underfunded",
        }
    }
}

// =================================================================================================
// La bisección, escrita UNA vez para los tres ejes
// =================================================================================================

/// **La bisección con extremo verificado, genérica sobre el eje.**
///
/// Invariante de entrada y de salida: `lo_fails` está comprobado MALO y `hi_ok` comprobado BUENO.
/// El bucle solo mueve `hi` a un punto que acaba de comprobar BUENO y `lo` a uno que acaba de
/// comprobar MALO, así que **lo que se devuelve siempre se ejecutó y cumplió**.
///
/// `midpoint` devuelve `None` cuando entre los dos extremos ya no queda nada que probar (meses
/// contiguos, o una media que por resolución decimal coincide con un extremo): ahí se para sola,
/// sin gastar el presupuesto restante.
fn bisect<T, Mid, F>(
    lo_fails: T,
    hi_ok: T,
    max_draws: u32,
    midpoint: Mid,
    mut meets: F,
) -> Result<T, McError>
where
    T: Copy,
    Mid: Fn(T, T) -> Option<T>,
    F: FnMut(T) -> Result<bool, McError>,
{
    let (mut lo, mut hi) = (lo_fails, hi_ok);
    let mut budget = max_draws;
    while budget > 0 {
        let Some(mid) = midpoint(lo, hi) else {
            break;
        };
        if meets(mid)? {
            hi = mid;
        } else {
            lo = mid;
        }
        budget -= 1;
    }
    Ok(hi)
}

/// Punto medio del eje MES. `None` cuando `lo` y `hi` son contiguos: ya no hay candidato interior.
fn month_mid(lo: u32, hi: u32) -> Option<u32> {
    (hi > lo + 1).then(|| lo + (hi - lo) / 2)
}

/// Punto medio del eje IMPORTE. `None` cuando la media coincide con un extremo — la resolución de
/// `Decimal` es finita y sin esta guarda la bisección gastaría el presupuesto probando `lo`.
fn amount_mid(lo: Decimal, hi: Decimal) -> Option<Decimal> {
    let mid = (lo + hi) / TWO;
    (mid > lo && mid < hi).then_some(mid)
}

/// Redondeo **hacia arriba a decenas de euro**. Un importe no positivo publica cero: una
/// aportación negativa no es una aportación.
fn round_up_to_tens(v: Decimal) -> Decimal {
    if v <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    (v / CONTRIBUTION_ROUNDING_STEP).ceil() * CONTRIBUTION_ROUNDING_STEP
}

/// El avance de la confirmación sobre el eje IMPORTE: **+5 %, con un suelo de una decena de euro**.
///
/// El suelo no es decoración: desde `c = 0` —el caso «la búsqueda dijo que no hace falta nada y la
/// confirmación lo desmiente»— el 5 % vale cero, y un avance que no avanza gasta un sorteo de 2.500
/// caminos para volver a medir exactamente lo mismo. Con el suelo, cada avance mueve al menos 10 €.
fn advance_five_percent(c: Decimal) -> Decimal {
    let stepped = round_up_to_tens(c * ONE_PLUS_FIVE_PERCENT);
    let floored = c + CONTRIBUTION_ROUNDING_STEP;
    if stepped > floored {
        stepped
    } else {
        floored
    }
}

// =================================================================================================
// Los tres escenarios (una mutación cada uno, en un solo sitio)
// =================================================================================================

/// **El escenario que aporta `extra` €/mes de más hasta jubilarse.**
///
/// El extra se suma a `planning_monthly_cash_adjustment` —los «Próximos»— y es **PLANO EN
/// NOMINAL** (supuesto S2, #139: en este motor los ingresos no se indexan y los gastos sí; una
/// aportación que creciera con la inflación sería el único flujo del bucle que lo hace, y lo haría
/// en silencio).
///
/// # La rejilla, dicha explícitamente
///
/// `planning_monthly_cash_adjustment` es **0-based**: el índice `i` es el mes `i+1` del bucle
/// (`sim_core.rs:1496`, `planning_adj = input.planning_monthly_cash_adjustment[(k − 1) as usize]`).
/// La aportación cubre los meses del BUCLE `1..=r−1` —se aporta mientras se trabaja y se deja de
/// aportar al jubilarse— o sea los **índices `0..=r−2`**, que es lo que escribe el `..upto` de
/// abajo con `upto = r − 1`.
///
/// # Por qué «Próximos» y no `income_regular_monthly`
///
/// Porque el extra tiene que entrar en la CAJA del mes sin tocar ninguna otra magnitud. Subir el
/// ingreso regular cambiaría también `ordinary_need` (`gasto − ingreso`) y con ella la puerta de
/// tasa inicial, que es justo el criterio que el solve está midiendo: el hogar «necesitaría menos»
/// por aportar más, y la respuesta saldría demasiado optimista. Los «Próximos» están fuera de
/// `ordinary_need` a propósito (`sim_core.rs:1610-1619`).
pub fn contributing_extra(input: &ProjectionInput, extra: Decimal, r: u32) -> ProjectionInput {
    let mut scenario = input.clone();
    let upto = (r.saturating_sub(1) as usize).min(scenario.planning_monthly_cash_adjustment.len());
    for slot in &mut scenario.planning_monthly_cash_adjustment[..upto] {
        *slot += extra;
    }
    scenario
}

/// **El escenario que deja de aportar desde el mes `stop`.**
///
/// Una sola mutación, y es exactamente la de `futurefin_engine::run_stopping_at` — la plantilla
/// que E4 hizo pública para esto. Aquí se escribe aparte porque el camino estocástico necesita la
/// ENTRADA mutada (para sortearla), no la salida de una proyección; que las dos no puedan divergir
/// lo ata `the_stochastic_stop_scenario_is_the_same_mutation_as_the_engine_template`.
pub fn stopping_at(input: &ProjectionInput, stop: u32) -> ProjectionInput {
    let mut scenario = input.clone();
    scenario.phase_plan.contributions_stop_month = Some(stop);
    scenario
}

/// **El escenario cuya media jornada empieza en `start_month`.** Sin fase parcial declarada no hay
/// nada que mover y la entrada vuelve intacta.
pub fn partial_starting_at(input: &ProjectionInput, start_month: u32) -> ProjectionInput {
    let mut scenario = input.clone();
    if let Some(p) = scenario.phase_plan.partial.as_mut() {
        p.start_month = start_month;
    }
    scenario
}

// =================================================================================================
// 1 · Aportación mínima
// =================================================================================================

/// **El resultado de [`minimum_extra_contribution`].**
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContributionSolve {
    /// El mes de jubilación (1-based) para el que se resolvió. Es el `r` que entró.
    pub month: u32,
    /// **Aportación extra mensual, plana y nominal, redondeada HACIA ARRIBA a decenas de euro.**
    ///
    /// `Some(0)` = «no te falta nada» (el plan ya cumple el umbral tal cual) y es una respuesta,
    /// no una ausencia. **`None` ⟺ [`Self::underfunded`]**: ni el techo de búsqueda cumple, y
    /// publicar ahí un cero diría lo contrario de lo que pasa.
    pub extra_monthly: Option<Decimal>,
    /// `true` ⟺ el techo de búsqueda —[`Self::search_ceiling`]— NO cumple el umbral. La lectura
    /// honesta es «con esta fecha y este plan no llegas ni aportando esto», y el importe del techo
    /// viaja al lado para poder decirlo con una cifra.
    pub underfunded: bool,
    /// **El techo que la búsqueda estuvo dispuesta a explorar**: el mayor extremo alto sondeado
    /// (`max(100, sobrante del mes 1)` doblado tantas veces como hizo falta, y sin doblar si la
    /// primera sonda ya cumplió). No es «lo que el hogar puede aportar»: es la cota de la
    /// bisección, y es lo que da sentido a [`Self::underfunded`].
    pub search_ceiling: Decimal,
    /// **La medición de CONFIRMACIÓN de lo que se publica** (2.500 caminos).
    ///
    /// Con solución es el sorteo de [`Self::extra_monthly`]; y si **no cumple el umbral**, la
    /// confirmación no cerró y lo publicado es lo más cerca que se llegó — la misma honestidad que
    /// `date_is_approximate` en la fecha, dicha sin un segundo booleano porque aquí la medición
    /// viaja al lado y se puede mirar.
    ///
    /// Sin solución (`underfunded`) es la medición del TECHO, con el presupuesto de BÚSQUEDA: lo
    /// más lejos que se llegó. `SuccessAt::paths` dice cuál de las dos es.
    pub success_at_solution: Option<SuccessAt>,
    /// Sorteos de BÚSQUEDA ejecutados (500 caminos cada uno).
    pub draws_search: u32,
    /// Sorteos de CONFIRMACIÓN ejecutados (2.500 caminos cada uno).
    pub draws_confirm: u32,
}

impl ContributionSolve {
    /// El aviso que este resultado implica, en UN sitio: sin él, cada llamante volvería a derivar
    /// «underfunded ⇒ `retire_at_age_underfunded`» por su cuenta.
    pub fn warning(&self) -> Option<StrategySolveWarning> {
        self.underfunded
            .then_some(StrategySolveWarning::RetireAtAgeUnderfunded)
    }
}

/// **La aportación mínima que hace válida una fecha** (M12; la pregunta que E4 se llevó de
/// `solve.rs::required_contribution_monthly` cuando su criterio determinista murió con el objetivo).
///
/// El menor extra mensual **plano y nominal** `c` tal que, aportándolo todos los meses hasta
/// jubilarse en `r`, al menos el umbral de los caminos no vuelve a fallar nunca:
/// `success_at_month(input + c, …, r).meets(threshold_pct)`.
///
/// # Las cuatro fases
///
/// | fase | qué hace | presupuesto |
/// |---|---|---|
/// | **0** | sonda de `c = 0`: si el plan ya cumple, la respuesta es cero y no se busca nada | 1 sorteo de `search` |
/// | **A** | extremo alto desde `max(100, sobrante del mes 1)`, DOBLANDO hasta que cumpla | ≤ 1 + [`MAX_CONTRIBUTION_DOUBLINGS`] |
/// | **B** | bisección sobre el importe, invariante «`lo` falla, `hi` cumple, se devuelve `hi`» | ≤ [`MAX_CONTRIBUTION_BISECTION_DRAWS`] |
/// | **C** | redondeo a decenas hacia arriba y CONFIRMACIÓN con el presupuesto grande; si no cumple, +5 % hasta [`MAX_STRATEGY_CONFIRMATION_ADVANCES`] veces | 1 + ≤ 6 |
///
/// El orden de la fase C importa: **se redondea ANTES de confirmar**. Redondear después mediría
/// una cifra y publicaría otra, que es exactamente la clase de desliz que convierte un solve
/// verificado en un solve decorativo.
///
/// # El extremo alto, y por qué se DOBLA en vez de leer un techo
///
/// `solve.rs::search_ceiling` —el máximo sobrante mensual del horizonte— es la cota correcta para
/// un solve que pone TECHO a lo que la cascada invierte: ahí la respuesta no puede pasarse de la
/// caja que el hogar produce. Aquí la incógnita es **dinero nuevo** que el hogar tendría que
/// encontrar, y su caja actual no la acota: un hogar sin sobrante ninguno puede necesitar
/// 3.000 €/mes, y con esa cota se le contestaría «no llegas» sin haber probado. Por eso el
/// sobrante del mes 1 (`first_month_allocation().recurring_net`, la magnitud que R5 llama
/// «sobrante») entra solo como **escala de arranque**, junto al suelo de
/// [`CONTRIBUTION_SEED_FLOOR`], y el techo se descubre doblando.
///
/// # `r` fuera del horizonte
///
/// No se recorta ni se rechaza: `contributing_extra` cubre los meses que existen y
/// `success_at_month(r)` con `r > H` simplemente no jubila a nadie dentro del horizonte. Lo que
/// salga de ahí es la respuesta de ese plan, no un caso especial de este módulo.
pub fn minimum_extra_contribution(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    search: &McConfig,
    confirm: &McConfig,
    threshold_pct: u32,
    r: u32,
) -> Result<ContributionSolve, McError> {
    let mut draws_search = 0u32;
    let mut draws_confirm = 0u32;

    // ------------------------------------------------------------------------------------------
    // (0) ¿hace falta algo?
    // ------------------------------------------------------------------------------------------
    let zero = probe_contribution(
        input,
        volatilities,
        search,
        r,
        Decimal::ZERO,
        &mut draws_search,
    )?;
    let mut last = zero;
    let mut ceiling = CONTRIBUTION_SEED_FLOOR.max(month_one_headroom(input)?);

    // ------------------------------------------------------------------------------------------
    // (A) Extremo alto, doblando
    // ------------------------------------------------------------------------------------------
    let mut lo = Decimal::ZERO;
    let mut hi_ok = zero.meets(threshold_pct).then_some(Decimal::ZERO);
    if hi_ok.is_none() {
        let mut hi = ceiling;
        for step in 0..=MAX_CONTRIBUTION_DOUBLINGS {
            let s = probe_contribution(input, volatilities, search, r, hi, &mut draws_search)?;
            last = s;
            ceiling = hi;
            if s.meets(threshold_pct) {
                hi_ok = Some(hi);
                break;
            }
            if step == MAX_CONTRIBUTION_DOUBLINGS {
                break;
            }
            // Este extremo está comprobado MALO: es el nuevo suelo de la bisección.
            lo = hi;
            hi = hi * TWO;
        }
    }

    let Some(hi) = hi_ok else {
        return Ok(ContributionSolve {
            month: r,
            extra_monthly: None,
            underfunded: true,
            search_ceiling: ceiling,
            success_at_solution: Some(last),
            draws_search,
            draws_confirm,
        });
    };

    // ------------------------------------------------------------------------------------------
    // (B) Bisección sobre el importe. Con `hi == lo == 0` no hay nada que estrechar.
    // ------------------------------------------------------------------------------------------
    let refined = if hi > lo {
        bisect(
            lo,
            hi,
            MAX_CONTRIBUTION_BISECTION_DRAWS,
            amount_mid,
            |c| -> Result<bool, McError> {
                Ok(
                    probe_contribution(input, volatilities, search, r, c, &mut draws_search)?
                        .meets(threshold_pct),
                )
            },
        )?
    } else {
        hi
    };

    // ------------------------------------------------------------------------------------------
    // (C) Redondear y CONFIRMAR (en ese orden)
    // ------------------------------------------------------------------------------------------
    let mut published = round_up_to_tens(refined);
    let mut stats = probe_contribution(
        input,
        volatilities,
        confirm,
        r,
        published,
        &mut draws_confirm,
    )?;
    for _ in 0..MAX_STRATEGY_CONFIRMATION_ADVANCES {
        if stats.meets(threshold_pct) {
            break;
        }
        published = advance_five_percent(published);
        stats = probe_contribution(
            input,
            volatilities,
            confirm,
            r,
            published,
            &mut draws_confirm,
        )?;
    }

    Ok(ContributionSolve {
        month: r,
        extra_monthly: Some(published),
        underfunded: false,
        search_ceiling: ceiling,
        success_at_solution: Some(stats),
        draws_search,
        draws_confirm,
    })
}

/// Un sorteo del escenario «aporto `extra` €/mes de más y me jubilo en `r`».
fn probe_contribution(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    r: u32,
    extra: Decimal,
    draws: &mut u32,
) -> Result<SuccessAt, McError> {
    let out = success_at_month(&contributing_extra(input, extra, r), volatilities, mc, r)?;
    *draws += 1;
    Ok(out)
}

/// El «sobrante» del mes 1 (R5): el neto recurrente `ingreso − gasto − servicio de deuda`,
/// clampado a ≥ 0. Aquí es solo la ESCALA de arranque del doblaje, no una cota.
fn month_one_headroom(input: &ProjectionInput) -> Result<Decimal, McError> {
    Ok(first_month_allocation(input)?
        .recurring_net
        .max(Decimal::ZERO))
}

// =================================================================================================
// 2 · Coast: el PRIMER mes en que puedes dejar de aportar
// =================================================================================================

/// **El resultado de [`coast_stop_month`].**
#[derive(Debug, Clone, PartialEq)]
pub struct CoastSolve {
    /// El mes de jubilación (1-based) para el que se resolvió.
    pub retirement_month: u32,
    /// **El PRIMER mes desde el que se puede dejar de aportar** y el plan sigue cumpliendo el
    /// umbral (C8: el primero, no el último). `Some(1)` = «puedes dejar de aportar ya».
    ///
    /// **`None` ⟺ [`StrategySolveWarning::CoastNotReachable`]**: ni aportando durante toda la
    /// acumulación se cumple el umbral, y ahí un cero o un `r` se leerían como una fecha.
    pub stop_month: Option<u32>,
    /// **El ahorro que se libera**: la caja del primer mes sin aportación, leída de una ejecución
    /// **determinista** (`Decimal`) del plan CON el corte.
    ///
    /// **Supuesto S4 — el ahorro liberado es caja DISPONIBLE, no se reinvierte.** Con
    /// `contributions_stop_month = Some(C)` el techo efectivo del mes `C` es 0
    /// (`PhasePlanG::contribution_cap_at`), el pool que llega a la cascada es 0 y el sobrante
    /// entero cae en `ProjectionOutput::disposable_cash[C]`: no se invierte, no compone y no entra
    /// en `net_worth`. Regresión: `the_freed_saving_of_coast_is_disposable_and_is_not_reinvested`.
    ///
    /// **El mes que se lee es `C`, no `C+1`.** `contributions_stop_month` es INCLUSIVO —el motor
    /// pone el techo a cero desde `k ≥ C` (`phases.rs:321-324`)—, así que el primer mes sin
    /// aportación es `C` mismo. Con `C = r` no se libera nada (el mes `r` ya está jubilado y no
    /// tiene sobrante), y el importe publicado es 0: cero euros, no «no aplica».
    pub freed_saving_monthly: Option<Decimal>,
    /// La medición de CONFIRMACIÓN del mes publicado (2.500 caminos). Si no cumple el umbral, la
    /// confirmación no cerró y lo publicado es lo más cerca que se llegó. Sin mes, la medición de
    /// BÚSQUEDA de `C = r` — la que dice por qué no lo hay (`failures_by_kind`).
    pub success_at_solution: Option<SuccessAt>,
    /// Avisos de este solve. Hoy solo [`StrategySolveWarning::CoastNotReachable`].
    pub warnings: Vec<StrategySolveWarning>,
    /// Sorteos de BÚSQUEDA ejecutados.
    pub draws_search: u32,
    /// Sorteos de CONFIRMACIÓN ejecutados.
    pub draws_confirm: u32,
}

/// **El primer mes en que puedes dejar de aportar** (M10 modo A, con la corrección **C8**).
///
/// El menor `C` tal que, cortando las aportaciones desde ese mes y jubilándose igualmente en `r`,
/// el plan sigue cumpliendo el umbral. **El PRIMER `C`, no el último**: la pregunta del usuario es
/// «¿desde cuándo puedo dejar de ahorrar?», y responderla con el último mes en que todavía podría
/// hacerlo sería contestar la contraria.
///
/// # Las tres fases
///
/// | fase | qué hace | presupuesto |
/// |---|---|---|
/// | **alta** | `C = r`: aportar durante toda la acumulación. Si ni así cumple ⇒ `CoastNotReachable` y no se sortea nada más | 1 sorteo de `search` |
/// | **baja** | `C = 1`: no aportar nunca. Si cumple ⇒ `stop_month = Some(1)` | 1 |
/// | **bisección** | sobre el mes, invariante «`lo` falla, `hi` cumple, se devuelve `hi`» | ≤ [`MAX_MONTH_BISECTION_DRAWS`] |
///
/// y luego la CONFIRMACIÓN con el presupuesto grande, avanzando `C` de mes en mes hacia `r` —el
/// extremo que la búsqueda verificó— hasta [`MAX_STRATEGY_CONFIRMATION_ADVANCES`] veces.
///
/// # `C = r` es «aportar siempre», no «parar al jubilarse»
///
/// El corte es inclusivo, así que `contributions_stop_month = Some(r)` deja las aportaciones vivas
/// en `1..=r−1` — toda la acumulación— y las apaga desde el mes en que el hogar se jubila, que es
/// cuando dejan de existir de todos modos. Por eso es el extremo BUENO por construcción: no hay
/// un plan de coast mejor que no hacer coast.
///
/// # Lo que NO se garantiza
///
/// La minimalidad, igual que en `valid_retirement_month`: se devuelve un mes **verificado**. Con
/// esta palanca la monotonía es más plausible que en la fecha (aportar más nunca puede quitar
/// líquido) pero tampoco se supone — la trayectoria de la base de coste cambia con el mes de
/// corte, y con impuestos activos dos ejecuciones con el mismo valor por activo pagan distinto.
pub fn coast_stop_month(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    search: &McConfig,
    confirm: &McConfig,
    threshold_pct: u32,
    r: u32,
) -> Result<CoastSolve, McError> {
    let r = r.max(1);
    let mut draws_search = 0u32;
    let mut draws_confirm = 0u32;

    // ------------------------------------------------------------------------------------------
    // Sonda ALTA: aportar siempre. Es el mejor plan de coast que existe; si falla, no hay ninguno.
    // ------------------------------------------------------------------------------------------
    let high = probe_stop(input, volatilities, search, r, r, &mut draws_search)?;
    if !high.meets(threshold_pct) {
        return Ok(CoastSolve {
            retirement_month: r,
            stop_month: None,
            freed_saving_monthly: None,
            success_at_solution: Some(high),
            warnings: vec![StrategySolveWarning::CoastNotReachable],
            draws_search,
            draws_confirm,
        });
    }

    // ------------------------------------------------------------------------------------------
    // Sonda BAJA + bisección.
    // ------------------------------------------------------------------------------------------
    let mut c = if r == 1 {
        1
    } else {
        let low = probe_stop(input, volatilities, search, r, 1, &mut draws_search)?;
        if low.meets(threshold_pct) {
            1
        } else {
            bisect(
                1,
                r,
                MAX_MONTH_BISECTION_DRAWS,
                month_mid,
                |m| -> Result<bool, McError> {
                    Ok(
                        probe_stop(input, volatilities, search, r, m, &mut draws_search)?
                            .meets(threshold_pct),
                    )
                },
            )?
        }
    };

    // ------------------------------------------------------------------------------------------
    // Confirmación, avanzando hacia `r` (el extremo verificado bueno).
    // ------------------------------------------------------------------------------------------
    let mut stats = probe_stop(input, volatilities, confirm, r, c, &mut draws_confirm)?;
    for _ in 0..MAX_STRATEGY_CONFIRMATION_ADVANCES {
        if stats.meets(threshold_pct) || c >= r {
            break;
        }
        c += 1;
        stats = probe_stop(input, volatilities, confirm, r, c, &mut draws_confirm)?;
    }

    Ok(CoastSolve {
        retirement_month: r,
        stop_month: Some(c),
        freed_saving_monthly: Some(freed_saving_monthly(input, r, c)?),
        success_at_solution: Some(stats),
        warnings: Vec::new(),
        draws_search,
        draws_confirm,
    })
}

/// Un sorteo del escenario «dejo de aportar en `stop` y me jubilo en `r`».
fn probe_stop(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    r: u32,
    stop: u32,
    draws: &mut u32,
) -> Result<SuccessAt, McError> {
    let out = success_at_month(&stopping_at(input, stop), volatilities, mc, r)?;
    *draws += 1;
    Ok(out)
}

/// **El ahorro liberado, medido en el camino DETERMINISTA** (`Decimal`, sin un solo `f64` de por
/// medio): la caja del mes `stop` que el corte dejó fuera de la cascada.
///
/// Se pasa por `futurefin_engine::run_stopping_at` —la plantilla pública— sobre el escenario ya
/// jubilado en `r` (`retiring_at`), para que la fase del mes `stop` sea la misma que el sorteo vio.
fn freed_saving_monthly(
    input: &ProjectionInput,
    r: u32,
    stop: u32,
) -> Result<Decimal, McError> {
    let out = run_stopping_at(&retiring_at(input, r), stop)?;
    Ok(out
        .disposable_cash
        .get(stop as usize)
        .copied()
        .unwrap_or(Decimal::ZERO)
        .max(Decimal::ZERO))
}

// =================================================================================================
// 3 · Media jornada: el PRIMER mes en que puedes empezar
// =================================================================================================

/// **El resultado de [`earliest_partial_start`].**
#[derive(Debug, Clone, PartialEq)]
pub struct PartialSolve {
    /// **El primer mes (1-based) en que la media jornada puede empezar** sin que la fase falle.
    ///
    /// **`None`** con [`StrategySolveWarning::PartialNeverStarts`] (la fase falla incluso empezando
    /// en el último mes del horizonte) **o sin fase parcial declarada** — y esos dos casos se
    /// distinguen por [`Self::warnings`], que en el segundo está vacío: una fase que no existe no
    /// es una fase que fracasa.
    pub start_month: Option<u32>,
    /// La medición de la FASE en el mes publicado (confirmación, 2.500 caminos). Sin mes, la de la
    /// última sonda de búsqueda — la que dice por qué no lo hay. `None` sin fase declarada.
    pub phase_success: Option<SuccessAt>,
    /// **La jubilación TOTAL con la fase dentro**, resuelta con `valid_retirement_month` desde
    /// `k_min = start_month + 1`. `None` si no hay `start_month` que probar.
    ///
    /// Su `month: None` es lo que emite [`StrategySolveWarning::PartialNeverFullyRetires`], y su
    /// `best_effort` es lo único que hay que enseñar en ese caso.
    pub full_retirement: Option<RetirementDateSolve>,
    /// Avisos de este solve.
    pub warnings: Vec<StrategySolveWarning>,
    /// Sorteos de BÚSQUEDA **de este solve**, sin los de la fecha anidada.
    ///
    /// No se suman a propósito: un total que esconde qué capa gastó qué no sirve para presupuestar
    /// nada. Los de la fecha están en `full_retirement.draws_search` / `.draws_confirm`, y el coste
    /// entero es la suma de los cuatro contadores.
    pub draws_search: u32,
    /// Sorteos de CONFIRMACIÓN **de este solve**, sin los de la fecha anidada.
    pub draws_confirm: u32,
}

/// **El primer mes en que puedes bajar a media jornada** (M11 modo «en cuanto pueda»).
///
/// El menor `S` tal que **la FASE no falla**, y después —una sola vez— la fecha de jubilación
/// total con esa fase dentro.
///
/// # El criterio de la fase: `AtMonth(H+1)`
///
/// «La fase no falla» se mide simulando un plan que **NUNCA se jubila del todo**:
/// `retirement_trigger = AtMonth(H+1)` con `H = horizon_months`. Es la única forma de aislar la
/// fase — con una jubilación total dentro del horizonte, un `S` se juzgaría por lo que pase
/// DESPUÉS de la media jornada, que es otra pregunta (y la que responde la fecha anidada).
///
/// Durante `Phase::Partial` el motor solo puede fallar por **F1** (cartera agotada): F3 está
/// restringida a `Phase::Retired` y la puerta de tasa inicial (F2) se evalúa en el primer mes
/// jubilado, que con `AtMonth(H+1)` no llega nunca (`sim_core.rs:1785-1812`, supuesto S1). Así que
/// el criterio dice exactamente lo que parece: **la media jornada no se come la cartera**.
///
/// # Las fases
///
/// | fase | qué hace | presupuesto |
/// |---|---|---|
/// | **baja** | `S = 1`: empezar ya. Si cumple, no hay nada que buscar | 1 sorteo de `search` |
/// | **alta** | `S = H`: empezar en el último mes. Si falla ⇒ `PartialNeverStarts` | 1 |
/// | **bisección** | sobre el mes, extremo verificado | ≤ [`MAX_MONTH_BISECTION_DRAWS`] |
/// | **confirmación** | con el presupuesto grande, avanzando `S` hacia `H` | 1 + ≤ 6 |
/// | **fecha** | **UNA** llamada a `valid_retirement_month` con la fase desde `S*` y `k_min = S*+1` | ≤ 25 + 14 |
///
/// # UNA capa anidada, no un producto
///
/// Es la propiedad que hace viable este solve, y está pineada
/// (`the_partial_phase_solve_is_one_nested_layer_not_a_product`): el coste es
/// `bisección_de_S + UNA fecha`, no `bisección_de_S × fecha`. Resolver la fecha dentro del
/// criterio de cada candidato `S` costaría ~14 × ~39 ≈ 550 sorteos — diez minutos de CPU por
/// request. Aquí son ~22 en el caso del arnés y ≤ 60 en la cota teórica.
///
/// # Sin fase parcial declarada
///
/// `start_month: None`, sin avisos y sin sorteos: **«no hay pregunta que responder»**, la
/// convención de `solve.rs`. Nunca un cero, que se leería como «puedes empezar ya». Las dos
/// configuraciones de Monte Carlo se validan igual antes de volver — un `McConfig` inválido es un
/// error aunque no haya nada que sortear.
pub fn earliest_partial_start(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    search: &McConfig,
    confirm: &McConfig,
    threshold_pct: u32,
) -> Result<PartialSolve, McError> {
    if input.phase_plan.partial.is_none() {
        let probe = retiring_at(input, 1);
        PathEngine::new(&probe, volatilities, search)?;
        PathEngine::new(&probe, volatilities, confirm)?;
        return Ok(PartialSolve {
            start_month: None,
            phase_success: None,
            full_retirement: None,
            warnings: Vec::new(),
            draws_search: 0,
            draws_confirm: 0,
        });
    }

    let horizon = input.horizon_months;
    // El mes en el que la jubilación total NO llega: un mes más allá del horizonte.
    let never = horizon.saturating_add(1);
    let mut draws_search = 0u32;
    let mut draws_confirm = 0u32;

    // ------------------------------------------------------------------------------------------
    // Sonda BAJA (empezar ya) y sonda ALTA (empezar en el último mes).
    // ------------------------------------------------------------------------------------------
    let low = probe_phase(input, volatilities, search, 1, never, &mut draws_search)?;
    let mut s = if low.meets(threshold_pct) {
        1
    } else {
        // Con `horizon <= 1` la sonda alta ES la baja: no se vuelve a sortear lo mismo.
        let high = if horizon <= 1 {
            low
        } else {
            probe_phase(
                input,
                volatilities,
                search,
                horizon,
                never,
                &mut draws_search,
            )?
        };
        if !high.meets(threshold_pct) {
            return Ok(PartialSolve {
                start_month: None,
                phase_success: Some(high),
                full_retirement: None,
                warnings: vec![StrategySolveWarning::PartialNeverStarts],
                draws_search,
                draws_confirm,
            });
        }
        bisect(
            1,
            horizon,
            MAX_MONTH_BISECTION_DRAWS,
            month_mid,
            |m| -> Result<bool, McError> {
                Ok(
                    probe_phase(input, volatilities, search, m, never, &mut draws_search)?
                        .meets(threshold_pct),
                )
            },
        )?
    };

    // ------------------------------------------------------------------------------------------
    // Confirmación, avanzando hacia `horizon`.
    // ------------------------------------------------------------------------------------------
    let mut stats = probe_phase(input, volatilities, confirm, s, never, &mut draws_confirm)?;
    for _ in 0..MAX_STRATEGY_CONFIRMATION_ADVANCES {
        if stats.meets(threshold_pct) || s >= horizon {
            break;
        }
        s += 1;
        stats = probe_phase(input, volatilities, confirm, s, never, &mut draws_confirm)?;
    }

    // ------------------------------------------------------------------------------------------
    // **UNA** fecha, con la fase desde `s`.
    // ------------------------------------------------------------------------------------------
    let date = valid_retirement_month(
        &partial_starting_at(input, s),
        volatilities,
        search,
        confirm,
        threshold_pct,
        s.saturating_add(1),
    )?;
    let warnings = if date.month.is_none() {
        vec![StrategySolveWarning::PartialNeverFullyRetires]
    } else {
        Vec::new()
    };

    Ok(PartialSolve {
        start_month: Some(s),
        phase_success: Some(stats),
        full_retirement: Some(date),
        warnings,
        draws_search,
        draws_confirm,
    })
}

/// Un sorteo del escenario «la media jornada empieza en `start` y la jubilación total no llega
/// nunca (`AtMonth(never)`)».
fn probe_phase(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    start: u32,
    never: u32,
    draws: &mut u32,
) -> Result<SuccessAt, McError> {
    let out = success_at_month(
        &partial_starting_at(input, start),
        volatilities,
        mc,
        never,
    )?;
    *draws += 1;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_codes_are_the_wire_contract() {
        assert_eq!(
            StrategySolveWarning::CoastNotReachable.code(),
            "coast_not_reachable"
        );
        assert_eq!(
            StrategySolveWarning::PartialNeverStarts.code(),
            "partial_never_starts"
        );
        assert_eq!(
            StrategySolveWarning::PartialNeverFullyRetires.code(),
            "partial_never_fully_retires"
        );
        assert_eq!(
            StrategySolveWarning::RetireAtAgeUnderfunded.code(),
            "retire_at_age_underfunded"
        );
    }

    /// La bisección genérica devuelve SIEMPRE un extremo comprobado bueno, en los dos ejes, y
    /// agotar el presupuesto no lo cambia.
    #[test]
    fn the_shared_bisection_returns_a_verified_high_end_on_both_axes() {
        // Eje MES: función escalón, cumple a partir de 137.
        let mut calls = 0u32;
        let hi = bisect(100u32, 160, 12, month_mid, |m| {
            calls += 1;
            Ok(m >= 137)
        })
        .unwrap();
        assert_eq!(hi, 137);
        assert!(calls <= 12, "{calls} sorteos");
        // Presupuesto de 1: no llega a 137, pero lo que devuelve cumple igual.
        let hi = bisect(100u32, 160, 1, month_mid, |m| Ok(m >= 137)).unwrap();
        assert!((137..=160).contains(&hi), "{hi}");

        // Eje IMPORTE: cumple a partir de 750.
        let target = Decimal::from(750);
        let hi = bisect(Decimal::ZERO, Decimal::from(8_000), 12, amount_mid, |c| {
            Ok(c >= target)
        })
        .unwrap();
        assert!(hi >= target, "{hi} debe cumplir");
        assert!(hi - target < Decimal::from(3), "resolución: {}", hi - target);
    }

    /// `month_mid` para cuando ya no hay candidato interior; `amount_mid`, cuando la media
    /// coincide con un extremo. Sin las dos guardas la bisección gastaría el presupuesto
    /// remidiendo `lo`.
    #[test]
    fn the_midpoints_stop_when_there_is_nothing_left_between() {
        assert_eq!(month_mid(10, 12), Some(11));
        assert_eq!(month_mid(10, 11), None);
        assert_eq!(month_mid(10, 10), None);
        assert_eq!(
            amount_mid(Decimal::ZERO, Decimal::from(10)),
            Some(Decimal::from(5))
        );
        assert_eq!(amount_mid(Decimal::from(5), Decimal::from(5)), None);
    }

    /// El redondeo es HACIA ARRIBA y a decenas — nunca al más cercano.
    #[test]
    fn the_published_contribution_is_rounded_up_to_tens() {
        let up = |v: &str| round_up_to_tens(v.parse::<Decimal>().unwrap());
        assert_eq!(up("0"), Decimal::ZERO);
        assert_eq!(up("-5"), Decimal::ZERO);
        assert_eq!(up("0.01"), Decimal::from(10));
        assert_eq!(up("10"), Decimal::from(10), "un múltiplo exacto no sube");
        assert_eq!(up("11"), Decimal::from(20));
        assert_eq!(up("7833.34"), Decimal::from(7_840));
    }

    /// El avance de la confirmación **siempre avanza**, y desde cero también: sin el suelo de una
    /// decena, `0 · 1,05 = 0` gastaría un sorteo de 2.500 caminos por nada.
    #[test]
    fn the_confirmation_advance_never_stands_still() {
        assert_eq!(advance_five_percent(Decimal::ZERO), Decimal::from(10));
        assert_eq!(advance_five_percent(Decimal::from(10)), Decimal::from(20));
        // 1.000 · 1,05 = 1.050 > 1.010 ⇒ manda el 5 %.
        assert_eq!(advance_five_percent(Decimal::from(1_000)), Decimal::from(1_050));
        for c in [0u32, 10, 100, 1_000, 7_840] {
            let c = Decimal::from(c);
            assert!(advance_five_percent(c) >= c + CONTRIBUTION_ROUNDING_STEP);
        }
    }
}
