//! **Los solves ESTOCÁSTICOS** (WP E6 de 5.0.0; decisiones M1/M2 del owner y correcciones C2/C3
//! del panel adversarial de 2026-09-06).
//!
//! Aquí vive la pregunta que define el modelo v2: **«¿cuándo me puedo jubilar?»**. Y se responde
//! como todas las inversas de esta casa — **biseccionando sobre el motor entero**, no sobre una
//! fórmula que lo aproxime — con una diferencia que cambia el vocabulario entero: cada evaluación
//! de la función objetivo no es UNA proyección, son **N caminos sorteados**, y su respuesta no es
//! un booleano sino una **proporción con barra de error**.
//!
//! # La doctrina, la misma que `crates/engine/src/solve.rs`
//!
//! Tres piezas, idénticas a las del solve `Decimal` (ver su doc de módulo, `solve.rs:1-40`):
//!
//! 1. **Bisección sobre el motor entero.** El criterio de «puedo jubilarme en `k`» no se aproxima
//!    con un capital descontado a una tasa escalar (hallazgo M8: un número plausible que ninguna
//!    simulación produce). Se mide ejecutando el bucle con `retirement_trigger = AtMonth(k)` y
//!    contando cuántos caminos fallan — con la cascada, los topes de las reglas, el servicio de
//!    deuda, los «Próximos», la fiscalidad del drenaje, la pensión con fecha y la puerta de tasa
//!    inicial dentro.
//! 2. **El extremo VERIFICADO.** Toda bisección mantiene «un extremo comprobado BUENO y el otro
//!    comprobado MALO» y devuelve el bueno. Lo que se publica no es una extrapolación: es un mes
//!    en el que se ejecutó el sorteo entero y cumplió el umbral.
//! 3. **Presupuesto de iteraciones, no umbral de convergencia.** El coste de un solve es un número
//!    conocido de sorteos ([`MAX_BRACKET_DRAWS`] + [`MAX_ANNUAL_DRAWS`] +
//!    [`MAX_MONTHLY_BISECTION_DRAWS`] de búsqueda, más las confirmaciones), y el llamante lo paga
//!    una vez y lo guarda en cache.
//!
//! # Lo que este módulo NO promete: la MINIMALIDAD
//!
//! El solve `Decimal` ya declaraba que la monotonía es lo que aporta la minimalidad, no la
//! validez. **Aquí la monotonía directamente no se supone**, y no por prudencia retórica: se sabe
//! que se rompe.
//!
//! - Un «Próximo» (`planning_monthly_cash_adjustment`) es un flujo en un mes ABSOLUTO. Jubilarse
//!   más tarde puede colocar una salida de 20.000 € justo después de la fecha en vez de mucho
//!   antes, y empeorar un plan que a un mes vista funcionaba.
//! - Con una fase de media jornada y base de gasto REGULAR (D10), retrasar la jubilación total
//!   alarga la fase cara.
//! - Con la inflación por encima del crecimiento neto de la cartera, la necesidad anual del mes
//!   `R` —el numerador de la puerta de tasa inicial— crece más deprisa que `L(R−1)`: el éxito
//!   **decrece** con `k` en tramos enteros del horizonte.
//! - Y la propia medición es muestral: dos meses contiguos evaluados con los mismos caminos
//!   pueden cruzar el umbral en distinto orden por ruido de muestreo.
//!
//! Por eso el resultado de [`valid_retirement_month`] se lee así, y así lo dice su doc:
//! **«un mes VERIFICADO que cumple», no «el mínimo demostrable»**. La diferencia importa cuando
//! alguien quiera afirmar en la UI que no existe ninguna fecha anterior: no lo sabemos, y
//! [`RetirementDateSolve::predecessor_success`] publica el único dato honesto al respecto — cuánto
//! éxito tiene el mes inmediatamente anterior, medido con el mismo presupuesto de confirmación.
//!
//! # El criterio de fallo de UN camino, y el del PLAN
//!
//! Un camino **falla** ⟺ `SimOutput::failure_month_index.is_some()`. Los tres motivos (F1
//! cartera agotada, F2 tasa inicial excedida, F3 la regla por saldo no llega a la necesidad
//! ordinaria) los clasifica el bucle del motor, no este módulo: aquí solo se CUENTAN
//! ([`SuccessAt::by_kind`]). La puerta F2 solo existe si el `PhasePlan` trae
//! `initial_rate: Some(..)`; sin ella ningún camino puede fallar por tasa inicial, y eso es una
//! propiedad de la entrada que arma el handler, no un default de este crate.
//!
//! El **plan** se juzga con [`SuccessAt::meets`]: por debajo de 100 con el **límite inferior del
//! intervalo de Wilson al 95 %** (C3 — estable frente a semilla y a `N`, que es justo lo que el
//! estimador puntual no era: medido al 100 % la fecha bailaba ±10 años según semilla), y en 100
//! con **cero fallos de N** más la cota de la regla de tres publicada al lado.
//!
//! Con cero fallos, Wilson colapsa a `n/(n + z²)` — de donde sale la consecuencia que hay que
//! tener presente al elegir presupuestos: **un umbral `u < 100` es inalcanzable con menos de
//! `z²·u/(1−u)` caminos** (73 para el 95 %, 381 para el 99 %) por limpia que salga la muestra.
//! Los 500 de la búsqueda topan en 0,99237 y los 2.500 de la confirmación en 0,99847, así que el
//! rango 80–100 del perfil cabe entero.
//!
//! # Números aleatorios COMUNES (common random numbers)
//!
//! Todas las evaluaciones de un solve —de todos los meses `k`, y también las de confirmación—
//! usan la MISMA semilla y los mismos índices de camino. `path_rng(seed, p)` no depende de `k`,
//! así que el camino `p` vive exactamente la misma secuencia de shocks se jubile en el mes 100 o
//! en el 400: lo que cambia entre dos evaluaciones es **solo la decisión**, no el mercado. Es lo
//! que hace comparables `éxito(k)` y `éxito(k+1)` y lo que convierte la muestra de búsqueda (500
//! caminos) en un **prefijo exacto** de la de confirmación (2.500): los 500 primeros caminos de la
//! confirmación son, bit a bit, los de la búsqueda. Regresión:
//! `common_random_numbers_make_the_confirmation_a_superset`.
//!
//! # Coste
//!
//! Medido en release sobre P9 (840 meses, 5 activos, 2 pasivos, cascada con topes, impuestos por
//! tramos) — `apps/api/src/handlers/projection_bands.rs` §presupuesto y
//! `crates/engine-stochastic/tests/timing_mc.rs`: **≈ 0,2 ms por camino**, medido como
//! ≈ 100–110 ms por sorteo de 500 y ≈ 445–465 ms por sorteo de 2.500. De ahí la partición del
//! presupuesto: se **busca** con 500 y se **confirma** con 2.500. Un solve completo A→E son 12–14
//! sorteos de búsqueda más 2 de confirmación ⇒ **1,8–1,9 s**, y la cota del peor caso (25 + 14)
//! ≈ 8,7–9,2 s. Objetivo declarado del plan: **≤ 3,5 s** típico y **≤ 10 s** en el peor caso; lo
//! mide, sin afirmarlo, `the_date_solve_costs_what_the_plan_says`. El coste de una fecha es
//! exactamente `draws_search · t(500) + draws_confirm · t(2.500)`, y los dos contadores se
//! publican en [`RetirementDateSolve`].
//!
//! La maquinaria del sorteo (`PathEngine`, en `mc.rs`) se construye **una vez por presupuesto**
//! (una para buscar, otra para confirmar) y se sostiene durante todo el solve: entre dos
//! evaluaciones solo se reescribe `sim.phase_plan.retirement_trigger`. Reconstruirla por sorteo
//! pagaría la conversión de la entrada y el buffer de factores (`meses × activos`) unas veinte
//! veces por fecha.
//!
//! # De aquí no sale un euro
//!
//! Sigue siendo la regla del crate: [`SuccessAt`] y [`RetirementDateSolve`] son **meses,
//! contadores y proporciones**. Ni un importe.

use futurefin_engine::{PathFailure, ProjectionInput, RetirementTrigger};

use crate::mc::{for_each_path, PathEngine};
use crate::{McConfig, McError};

// =================================================================================================
// Presupuestos y constantes
// =================================================================================================

/// El cuantil 97,5 % de la normal estándar, redondeado a dos decimales — el `z` del intervalo de
/// Wilson **al 95 %** que fija la decisión C3.
///
/// Se escribe `1.96` y no `1.959963984540054` a propósito: es el número que la decisión nombra y
/// el que cualquiera puede reproducir con una tabla. La diferencia sobre la cota inferior con
/// 0 fallos de 2.500 es de 6e-6 pp — cuatro órdenes de magnitud por debajo de la resolución con
/// la que se publica.
pub const WILSON_Z_95: f64 = 1.96;

/// Paso del bracket de la fase (A): **cinco años**. Con el horizonte de la app (≤ 840 meses) los
/// [`MAX_BRACKET_DRAWS`] sorteos cubren el rango entero; si el horizonte fuera mayor, el paso se
/// ESTIRA (ver `bracket_grid`) en vez de dejar la cola del horizonte sin sondear.
pub const BRACKET_STEP_MONTHS: u32 = 60;

/// Sorteos máximos de la fase (A). 15 × 60 = 900 meses ≥ 840.
pub const MAX_BRACKET_DRAWS: u32 = 15;

/// Sorteos máximos del refinado ANUAL, fase (B). Un bracket de 60 meses tiene exactamente 4
/// candidatos anuales interiores (`lo+12, +24, +36, +48`).
pub const MAX_ANNUAL_DRAWS: u32 = 4;

/// Sorteos máximos de la bisección MENSUAL, fase (C). `log₂ 12 ≈ 3,6`, y 6 cubren hasta 64 meses
/// de bracket — el margen que deja el estiramiento del paso.
pub const MAX_MONTHLY_BISECTION_DRAWS: u32 = 6;

/// Avances mes a mes de la fase (D) cuando la confirmación no cierra: **un año**. Más allá, la
/// respuesta honesta no es «sigue buscando», es `date_is_approximate = true`.
pub const MAX_CONFIRMATION_ADVANCES: u32 = 12;

/// Posición de `PathFailure::PortfolioDepleted` (F1) en [`SuccessAt::by_kind`].
pub const KIND_PORTFOLIO_DEPLETED: usize = 0;
/// Posición de `PathFailure::InitialRateExceeded` (F2) en [`SuccessAt::by_kind`].
pub const KIND_INITIAL_RATE_EXCEEDED: usize = 1;
/// Posición de `PathFailure::RuleBelowNeed` (F3) en [`SuccessAt::by_kind`].
pub const KIND_RULE_BELOW_NEED: usize = 2;

/// El índice de un motivo de fallo en [`SuccessAt::by_kind`]. Vive aquí —y no como un `as usize`
/// suelto— para que el orden del vector de contadores esté escrito UNA vez.
fn kind_index(kind: PathFailure) -> usize {
    match kind {
        PathFailure::PortfolioDepleted => KIND_PORTFOLIO_DEPLETED,
        PathFailure::InitialRateExceeded => KIND_INITIAL_RATE_EXCEEDED,
        PathFailure::RuleBelowNeed => KIND_RULE_BELOW_NEED,
    }
}

// =================================================================================================
// La medición de un mes
// =================================================================================================

/// **El éxito de jubilarse en un mes concreto**, con su barra de error.
///
/// Todo lo de aquí es una CUENTA sobre `paths` caminos del mismo sorteo. No hay ningún euro.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuccessAt {
    /// El mes del BUCLE (1-based) en el que se forzó la jubilación.
    pub month: u32,
    /// Caminos efectivamente simulados. Es el denominador de todo lo demás y por eso viaja al
    /// lado: una probabilidad sin su `N` no se puede comparar con otra.
    pub paths: u32,
    /// Caminos con `failure_month_index.is_some()`.
    pub failures: u32,
    /// El estimador PUNTUAL, `1 − failures/paths`.
    pub success: f64,
    /// **Límite inferior del intervalo de Wilson al 95 %** — el número contra el que se compara
    /// el umbral por debajo de 100 (C3).
    ///
    /// Wilson y no la aproximación normal (`p̂ ± z·√(p̂q̂/n)`) por la razón por la que existe:
    /// con `p̂ = 1` la normal da una barra de error **exactamente cero** y declararía «100 %
    /// seguro» con 2.500 caminos, que es la clase de número que esta casa no publica.
    pub wilson_low: f64,
    /// La distancia del estimador puntual a [`Self::wilson_low`], **en puntos porcentuales**:
    /// `(success − wilson_low)·100`. Es la barra de error que la UI dibuja hacia abajo, que es el
    /// lado que decide el umbral.
    ///
    /// **No es media anchura del intervalo**: el de Wilson es asimétrico, y de sus dos lados el
    /// que importa aquí es el de abajo. Con 0 fallos de 2.500 vale **0,1534 pp** — nunca 0.
    pub half_width_pp: f64,
    /// La **cota de la regla de tres**, `3/N`, publicada solo cuando `failures == 0`: con cero
    /// fallos observados el riesgo real está por debajo de `3/N` con confianza ≈ 95 %
    /// (0/2.500 ⇒ ≤ 0,12 %). `None` con al menos un fallo, donde la regla no aplica y el número
    /// honesto es el intervalo entero.
    pub rule_of_three_upper: Option<f64>,
    /// Fallos por motivo, en el orden [`KIND_PORTFOLIO_DEPLETED`],
    /// [`KIND_INITIAL_RATE_EXCEEDED`], [`KIND_RULE_BELOW_NEED`]. Suma [`Self::failures`].
    pub by_kind: [u32; 3],
}

impl SuccessAt {
    /// Construye la medición a partir de los CONTADORES. Es el único sitio donde se escribe la
    /// aritmética de Wilson, y es público para que se pueda ejercitar sin sortear nada.
    ///
    /// `paths == 0` no es «todo falla»: es «no se midió». Como la firma no tiene sitio para un
    /// `None`, se devuelve todo a cero y [`Self::meets`] es `false` — y el camino que llega aquí
    /// desde un solve no existe, porque `PathEngine::new` rechaza `paths = 0` antes de sortear.
    pub fn new(month: u32, paths: u32, failures: u32, by_kind: [u32; 3]) -> Self {
        debug_assert!(failures <= paths);
        debug_assert_eq!(by_kind.iter().sum::<u32>(), failures);
        if paths == 0 {
            return SuccessAt {
                month,
                paths: 0,
                failures: 0,
                success: 0.0,
                wilson_low: 0.0,
                half_width_pp: 0.0,
                rule_of_three_upper: None,
                by_kind,
            };
        }
        let n = f64::from(paths);
        let successes = f64::from(paths - failures);
        let success = successes / n;
        let wilson_low = wilson_lower_bound(success, n);
        SuccessAt {
            month,
            paths,
            failures,
            success,
            wilson_low,
            half_width_pp: (success - wilson_low) * 100.0,
            rule_of_three_upper: (failures == 0).then(|| 3.0 / n),
            by_kind,
        }
    }

    /// **La regla del umbral** (C3), escrita una sola vez:
    ///
    /// ```text
    ///   umbral < 100  ⇒  wilson_low ≥ umbral/100
    ///   umbral = 100  ⇒  failures == 0
    /// ```
    ///
    /// El rango 80–100 lo valida el PERFIL (`success_threshold_out_of_range` en la API), no este
    /// módulo: la regla es total para cualquier `u32`, y un umbral por encima de 100 es
    /// insatisfacible por construcción. **Nunca se recorta en silencio** — un umbral recortado es
    /// una promesa que el usuario no hizo.
    ///
    /// # El umbral acota por abajo el TAMAÑO DE LA MUESTRA
    ///
    /// Consecuencia directa de la forma cerrada de `wilson_lower_bound` con `p̂ = 1`
    /// (`low = n/(n+z²)`): **el mejor resultado posible con `n` caminos es `n/(n+z²)`**, así que
    /// un umbral `u < 100` es INALCANZABLE —no falle ni un camino— si
    ///
    /// ```text
    ///   n < z²·u/(1−u)      ⇒  95 % pide n ≥ 73;  99 % pide n ≥ 381
    /// ```
    ///
    /// No es un fallo del solver: es lo que significa medir con pocas muestras, y por eso la
    /// respuesta correcta no es recortar el umbral sino sortear más. Los presupuestos del plan
    /// —500 buscando, 2.500 confirmando— cubren el rango 80–100 entero con holgura (500 topan en
    /// 0,99237). El caso `u = 100` es la excepción: «cero fallos de N» no depende de `N`, y por
    /// eso viaja siempre con [`Self::rule_of_three_upper`], que sí lo dice.
    pub fn meets(&self, threshold_pct: u32) -> bool {
        if self.paths == 0 {
            return false;
        }
        if threshold_pct >= 100 {
            threshold_pct == 100 && self.failures == 0
        } else {
            self.wilson_low >= f64::from(threshold_pct) / 100.0
        }
    }
}

/// **Límite inferior del intervalo de score de Wilson** al 95 %, para una proporción `p̂` sobre
/// `n` observaciones:
///
/// ```text
///   centro = (p̂ + z²/2n) / (1 + z²/n)
///   margen = (z / (1 + z²/n)) · √( p̂(1−p̂)/n + z²/4n² )
///   low    = centro − margen
/// ```
///
/// Es el intervalo que se obtiene INVIRTIENDO el test de score (se resuelve
/// `|p̂ − p| = z·√(p(1−p)/n)` en `p`), y por eso no degenera en los extremos: con `p̂ = 1` el
/// término `z²/4n²` mantiene el margen estrictamente positivo. La aproximación normal, que es la
/// que la mayoría de las herramientas usa, da cero ahí.
///
/// **Con `p̂ = 1` colapsa a una forma cerrada** que conviene tener escrita, porque es el caso que
/// más se publica (un plan que no falla en ningún camino):
///
/// ```text
///   centro − margen = (1 + z²/2n)/(1 + z²/n) − (z/(1+z²/n))·(z/2n)
///                   = [(1 + z²/2n) − z²/2n] / (1 + z²/n)
///                   = 1/(1 + z²/n)  =  n/(n + z²)
/// ```
///
/// De ahí salen los dos números que este módulo repite: `2500/2503,8416 = 0,9984657` (barra
/// 0,1534 pp) y `500/503,8416 = 0,9923764`. Y de ahí sale también la cota de tamaño de muestra
/// que documenta [`SuccessAt::meets`].
///
/// Se acota a `[0, 1]` al final por higiene de coma flotante: la fórmula no puede salirse, pero
/// una cota negativa de una probabilidad publicada sería peor que un redondeo.
fn wilson_lower_bound(p_hat: f64, n: f64) -> f64 {
    let z2 = WILSON_Z_95 * WILSON_Z_95;
    let denom = 1.0 + z2 / n;
    let centre = (p_hat + z2 / (2.0 * n)) / denom;
    let margin =
        (WILSON_Z_95 / denom) * (p_hat * (1.0 - p_hat) / n + z2 / (4.0 * n * n)).sqrt();
    (centre - margin).clamp(0.0, 1.0)
}

// =================================================================================================
// El escenario: «jubilarse en k»
// =================================================================================================

/// **La entrada que se jubila en `month`**, definida en UN solo sitio.
///
/// Dos mutaciones y ninguna más:
///
/// - `retirement_trigger = AtMonth(month)` — el mes forzado que ya existía en 4.15.0 y que el
///   modelo v2 reutiliza tal cual: el solver externo es lo único nuevo.
/// - `crossing_is_reading_only = true` — sin esto el motor conserva la UNIÓN
///   `cruce || k ≥ mes forzado` y un camino afortunado se jubilaría ANTES de `month`, con lo que
///   `éxito(k)` dejaría de medir «jubilarse en `k`» para medir «jubilarse en `min(cruce, k)`».
///   El cruce sigue anotándose como lectura (`liquid_crossing_month_index`).
///
/// El objetivo FIRE (`fire_target`) **no se toca**: si el llamante lo trae, sigue viajando y
/// sigue publicándose como lectura.
pub fn retiring_at(input: &ProjectionInput, month: u32) -> ProjectionInput {
    let mut scenario = input.clone();
    scenario.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(month);
    scenario.phase_plan.crossing_is_reading_only = true;
    scenario
}

/// La maquinaria de sorteo de UN presupuesto (búsqueda o confirmación), sostenida durante todo el
/// solve.
///
/// Entre dos evaluaciones solo se reescribe `sim.phase_plan.retirement_trigger`. Es EXACTAMENTE
/// equivalente a reconstruir el motor desde [`retiring_at`] —`SimInput::from` es una conversión
/// campo a campo sin nada derivado— y lo comprueba
/// `common_random_numbers_make_the_confirmation_a_superset`, que enfrenta esta vía con
/// `run_path(&retiring_at(..))` camino a camino.
struct Draws {
    engine: PathEngine,
    paths: u32,
    /// Hilos con los que repartir los caminos de CADA sorteo (E12). Se resuelve una vez, al
    /// construir el presupuesto, y no cambia entre evaluaciones: el reparto no es parte de la
    /// pregunta, solo de cómo se contesta. Ver `crate::parallel`.
    threads: usize,
    /// Sorteos completos ejecutados. Es el coste medido del solve, y se publica.
    draws: u32,
}

impl Draws {
    fn new(
        input: &ProjectionInput,
        volatilities: &[Option<f64>],
        config: &McConfig,
    ) -> Result<Self, McError> {
        // El mes concreto da igual: `at` lo reescribe antes de cada sorteo. Lo que sí se fija
        // aquí —y no cambia nunca— es `crossing_is_reading_only`.
        let scenario = retiring_at(input, 1);
        Ok(Draws {
            engine: PathEngine::new(&scenario, volatilities, config)?,
            paths: config.paths,
            threads: crate::parallel::resolve_threads(config),
            draws: 0,
        })
    }

    /// Un sorteo completo con la jubilación forzada en `month`.
    ///
    /// Los caminos se reparten entre hilos (E12) y **el conteo no se entera**: lo que cruza
    /// caminos son dos contadores ENTEROS, que se incrementan en el hilo llamante y en orden de
    /// índice desde el `sink` de `for_each_path`. Sumar enteros es asociativo y el orden no los
    /// mueve; aun así se pliegan en orden, porque la regla de la casa es que el pliegue no
    /// dependa de quién termine antes.
    fn at(&mut self, month: u32) -> Result<SuccessAt, McError> {
        self.engine.sim.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(month);
        let mut failures = 0u32;
        let mut by_kind = [0u32; 3];
        for_each_path(
            &self.engine,
            self.paths,
            self.threads,
            // **El fallo lo define `failure_month_index`**, no el motivo: es el campo que el
            // contrato nombra. `failure_kind` solo clasifica, y el motor garantiza que los dos
            // son `Some` a la vez.
            |_p, out| (out.failure_month_index.is_some(), out.failure_kind),
            |_p, (failed, kind)| {
                if failed {
                    failures += 1;
                    debug_assert!(
                        kind.is_some(),
                        "un camino fallido sin motivo rompería el reparto de `by_kind`"
                    );
                    if let Some(kind) = kind {
                        by_kind[kind_index(kind)] += 1;
                    }
                }
            },
        )?;
        self.draws += 1;
        Ok(SuccessAt::new(month, self.paths, failures, by_kind))
    }
}

// =================================================================================================
// éxito(k) y la tira anual
// =================================================================================================

/// **`éxito(k)`**: la proporción de caminos que, jubilándose en el mes `k`, no fallan nunca hasta
/// el horizonte.
///
/// Reconstruye la maquinaria del sorteo en cada llamada. Para evaluar una rejilla use
/// [`success_by_retirement_month`], y para resolver una fecha [`valid_retirement_month`]: los dos
/// la sostienen.
pub fn success_at_month(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    month: u32,
) -> Result<SuccessAt, McError> {
    Draws::new(input, volatilities, mc)?.at(month)
}

/// **`éxito(k)` CONDICIONADO a llegar al mes `k` por la línea determinista.**
///
/// Idéntica a [`success_at_month`] salvo en un eje: los meses `1..stochastic_from_month−1` crecen
/// con el multiplicador DETERMINISTA del motor y solo desde `stochastic_from_month` se sortea. Con
/// `stochastic_from_month = 1` (o `0`) es literalmente [`success_at_month`].
///
/// # Para qué existe
///
/// Es la pregunta de la curva de capital necesario desde la corrección de 2026-09-07: **«si llego a
/// esa edad con X, ¿aguanto?»**, no «¿qué percentil de mi hogar de hoy, treinta años después, aguanta?».
/// Fijando la acumulación en la línea determinista, todos los caminos llegan al cierre de
/// `k−1` con el MISMO líquido, así que:
///
/// - la puerta de tasa inicial (F2) se evalúa una sola vez sobre un `L(k−1)` común — pasa en todos
///   los caminos o falla en todos, que es lo que convierte `éxito(X, k)` en un ESCALÓN más el
///   margen que pida F1, en vez de en un percentil;
/// - `éxito` deja de arrastrar la dispersión de la acumulación (medida: ×3,19 a 30 años con σ 17 %)
///   y el sobrecoste que Wilson cobra sobre esa dispersión con 500 caminos (×1,24).
///
/// Con `stochastic_from_month = month` —el uso de `needed_capital`— el sorteo cubre exactamente el
/// tramo jubilado, que es el único donde la secuencia de retornos decide algo.
///
/// # Lo que NO cambia
///
/// La FECHA (`valid_retirement_month`) sigue siendo la definición A: cada camino con su propia
/// acumulación, sorteada desde el mes 1. Son dos preguntas distintas y ninguna sustituye a la otra.
pub fn success_at_month_from(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    month: u32,
    stochastic_from_month: u32,
) -> Result<SuccessAt, McError> {
    let mut draws = Draws::new(input, volatilities, mc)?;
    draws.engine.set_stochastic_from_month(stochastic_from_month);
    draws.at(month)
}

/// **La tira anual de la UI**: `éxito(k)` sobre la rejilla que el llamante pasa, en su MISMO orden
/// y con sus repeticiones.
///
/// La rejilla la decide quien dibuja (la SPA la quiere por años de edad, el MCP por décadas): este
/// crate no sabe de fechas de nacimiento y no se inventa un muestreo. Con una rejilla vacía
/// devuelve un vector vacío —**después** de validar la configuración, que un `McConfig` inválido
/// es un error aunque no haya nada que sortear.
pub fn success_by_retirement_month(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    grid: &[u32],
) -> Result<Vec<SuccessAt>, McError> {
    let mut draws = Draws::new(input, volatilities, mc)?;
    grid.iter().map(|&k| draws.at(k)).collect()
}

// =================================================================================================
// La fecha válida
// =================================================================================================

/// **El resultado de [`valid_retirement_month`]**: una fecha verificada, o la razón de que no la
/// haya.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetirementDateSolve {
    /// El mes del BUCLE (1-based) en el que el plan cumple el umbral. **`None` = no hay ninguno en
    /// `[k_min, horizonte]`**, y nunca un cero: un cero se leería como «ya puedes», que es la
    /// respuesta contraria.
    pub month: Option<u32>,
    /// Éxito del mes devuelto, medido con el presupuesto de CONFIRMACIÓN. Sin fecha, el de la
    /// mejor observación (la misma que [`Self::best_effort`]) — que no es una fecha, y `month` es
    /// lo que manda.
    pub success: f64,
    /// Cota inferior de Wilson del mes devuelto (misma medición que [`Self::success`]).
    pub wilson_low: f64,
    /// Barra de error hacia abajo, en puntos porcentuales (ver [`SuccessAt::half_width_pp`]).
    pub half_width_pp: f64,
    /// `3/N` cuando la confirmación no vio ni un fallo (ver [`SuccessAt::rule_of_three_upper`]).
    pub rule_of_three_upper: Option<f64>,
    /// **Éxito del mes ANTERIOR**, auditado con el presupuesto de confirmación. Es el único dato
    /// honesto sobre la minimalidad: dice cuánto le falta al mes de antes, no que no exista
    /// ninguno anterior que cumpla.
    ///
    /// `None` ⟺ no hay predecesor que auditar (el mes devuelto es `k_min`, o no hay fecha).
    /// **Nunca un 0**, que se leería como «el mes anterior fracasa siempre».
    pub predecessor_success: Option<f64>,
    /// `true` ⟺ la confirmación **no cerró**: ni el mes que la búsqueda verificó ni los
    /// [`MAX_CONFIRMATION_ADVANCES`] siguientes cumplieron el umbral con el presupuesto grande, y
    /// lo que se devuelve es el que más cerca quedó. La fecha se publica igual —con este flag y
    /// con su éxito real al lado—, porque «no lo sé» no es lo mismo que «no existe».
    pub date_is_approximate: bool,
    /// Sorteos de BÚSQUEDA ejecutados (fases A–C).
    pub draws_search: u32,
    /// Sorteos de CONFIRMACIÓN ejecutados (fases D–E).
    pub draws_confirm: u32,
    /// Fallos por motivo del mes devuelto, en el orden de [`SuccessAt::by_kind`]. Sin fecha, los
    /// de la mejor observación: es lo que dice POR QUÉ no hay fecha (todo F2 ⇒ la tasa inicial no
    /// da; todo F1 ⇒ la cartera no aguanta el horizonte).
    pub failures_by_kind: [u32; 3],
    /// **El mejor par `(mes, éxito)` OBSERVADO** durante el solve, elegido por `wilson_low` y con
    /// el mes más temprano deshaciendo empates.
    ///
    /// Su razón de ser es el caso sin fecha, donde es lo único que hay que enseñar («lo más cerca
    /// que llegas es el 78 % a los 67»). Con fecha se publica igual y **no tiene por qué ser el
    /// mes devuelto**: la búsqueda pudo ver un mes posterior con más margen.
    pub best_effort: Option<(u32, f64)>,
}

/// **La fecha válida (definición A del owner, M1)**: el primer mes `k ≥ k_min` que la búsqueda
/// encuentra y la confirmación verifica, tal que jubilándose en `k` **al menos el umbral de los
/// caminos no vuelve a fallar nunca** hasta el horizonte.
///
/// # Las cinco fases
///
/// | fase | qué hace | presupuesto |
/// |---|---|---|
/// | **A** | bracket de [`BRACKET_STEP_MONTHS`] meses sobre `[k_min, H]` con `search`: el primer mes de la rejilla que cumple | ≤ [`MAX_BRACKET_DRAWS`] |
/// | **B** | refinado ANUAL dentro del bracket, barrido de abajo arriba | ≤ [`MAX_ANNUAL_DRAWS`] |
/// | **C** | bisección MENSUAL dentro del año, invariante «`lo` falla, `hi` cumple, se devuelve `hi`» | ≤ [`MAX_MONTHLY_BISECTION_DRAWS`] |
/// | **D** | confirmación del mes devuelto con `confirm`; si no cumple, avanza mes a mes hasta [`MAX_CONFIRMATION_ADVANCES`] veces y, si aun así no cierra, marca `date_is_approximate` | 1 + ≤ 12 |
/// | **E** | auditoría de `k−1` con `confirm` → [`RetirementDateSolve::predecessor_success`] | ≤ 1 |
///
/// La fase D existe porque los 500 caminos de la búsqueda son un **prefijo** de los 2.500 de la
/// confirmación (números aleatorios comunes): la confirmación no es otra muestra, es la misma
/// ampliada, y por eso puede desmentir a la búsqueda sin contradecirla.
///
/// # `k_min` — el suelo, y quién lo pone
///
/// Lo pasa el LLAMANTE. Sin puente vale `1`; con puente, `max(1, P − 12·bridge_max_years)` con `P`
/// el mes en que la pensión entra en caja (C2). **Este es el único sitio donde
/// `bridge_max_years` acota la FECHA**: el tope de tasa inicial que el puente levanta es cosa del
/// motor (`InitialRateGate::bridge`), y confundir los dos era exactamente el error que el panel
/// midió. Un `k_min` mayor que el horizonte devuelve `month: None` sin sortear nada.
///
/// # Lo que se garantiza, dicho sin adornos
///
/// **Un mes verificado que cumple**, no el mínimo demostrable (ver el doc del módulo: la
/// monotonía se rompe con «Próximos», con fases parciales caras y con la inflación por encima del
/// crecimiento). Quien quiera saber si hay algo antes tiene [`RetirementDateSolve::predecessor_success`].
///
/// # Umbral
///
/// `threshold_pct` se aplica tal cual con [`SuccessAt::meets`]. La cota 80–100 la valida el
/// perfil; aquí un umbral fuera de rango no se recorta, simplemente no lo cumple nadie.
pub fn valid_retirement_month(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    search: &McConfig,
    confirm: &McConfig,
    threshold_pct: u32,
    k_min: u32,
) -> Result<RetirementDateSolve, McError> {
    // Las dos configuraciones se validan ANTES de sortear nada, en el orden en que se usan.
    let mut search_draws = Draws::new(input, volatilities, search)?;
    let mut confirm_draws = Draws::new(input, volatilities, confirm)?;

    let horizon = input.horizon_months;
    // El mes 0 no existe en el bucle: `AtMonth(0)` jubilaría desde el primer mes con la misma
    // condición `k ≥ 0`, pero el contrato de los índices es 1-based y aquí se respeta.
    let k_min = k_min.max(1);

    let mut best = Best::default();

    if k_min > horizon {
        return Ok(no_date(&best, 0, 0));
    }

    // ------------------------------------------------------------------------------------------
    // (A) Bracket de 60 meses
    // ------------------------------------------------------------------------------------------
    let mut lo_fails: Option<u32> = None;
    let mut hi_ok: Option<u32> = None;
    for month in bracket_grid(k_min, horizon) {
        let s = search_draws.at(month)?;
        best.offer(&s);
        if s.meets(threshold_pct) {
            hi_ok = Some(month);
            break;
        }
        lo_fails = Some(month);
    }
    let Some(mut hi) = hi_ok else {
        return Ok(no_date(&best, search_draws.draws, confirm_draws.draws));
    };

    // ------------------------------------------------------------------------------------------
    // (B) Refinado anual + (C) bisección mensual — solo si hay un extremo MALO por debajo.
    // Sin él, `hi == k_min` y no hay nada que estrechar: el suelo es el suelo.
    // ------------------------------------------------------------------------------------------
    if let Some(mut lo) = lo_fails {
        for month in annual_candidates(lo, hi) {
            let s = search_draws.at(month)?;
            best.offer(&s);
            if s.meets(threshold_pct) {
                hi = month;
                break;
            }
            lo = month;
        }
        hi = bisect_month(lo, hi, MAX_MONTHLY_BISECTION_DRAWS, |month| {
            let s = search_draws.at(month)?;
            best.offer(&s);
            Ok(s.meets(threshold_pct))
        })?;
    }

    // ------------------------------------------------------------------------------------------
    // (D) Confirmación
    // ------------------------------------------------------------------------------------------
    let mut stats = confirm_draws.at(hi)?;
    best.offer(&stats);
    let mut date_is_approximate = false;
    if !stats.meets(threshold_pct) {
        let mut probes = vec![stats];
        let mut closed = false;
        for step in 1..=MAX_CONFIRMATION_ADVANCES {
            let month = hi + step;
            if month > horizon {
                break;
            }
            let s = confirm_draws.at(month)?;
            best.offer(&s);
            probes.push(s);
            if s.meets(threshold_pct) {
                stats = s;
                closed = true;
                break;
            }
        }
        if !closed {
            // **El que más cerca quedó**, no el último probado: los trece meses se midieron con
            // el mismo presupuesto y el más cercano al umbral es la lectura útil. El flag y el
            // `success` publicado dicen la verdad — que ninguno cumplió.
            stats = probes
                .into_iter()
                .reduce(|a, b| if b.wilson_low > a.wilson_low { b } else { a })
                .expect("siempre está al menos la sonda de `hi`");
            date_is_approximate = true;
        }
    }
    let month = stats.month;

    // ------------------------------------------------------------------------------------------
    // (E) Auditoría del predecesor
    // ------------------------------------------------------------------------------------------
    let predecessor_success = if month > k_min {
        let s = confirm_draws.at(month - 1)?;
        best.offer(&s);
        Some(s.success)
    } else {
        None
    };

    Ok(RetirementDateSolve {
        month: Some(month),
        success: stats.success,
        wilson_low: stats.wilson_low,
        half_width_pp: stats.half_width_pp,
        rule_of_three_upper: stats.rule_of_three_upper,
        predecessor_success,
        date_is_approximate,
        draws_search: search_draws.draws,
        draws_confirm: confirm_draws.draws,
        failures_by_kind: stats.by_kind,
        best_effort: best.pair(),
    })
}

/// **La bisección por mes, escrita UNA vez.**
///
/// Invariante de entrada y de salida: `lo_fails` está comprobado MALO y `hi_ok` comprobado BUENO.
/// El bucle solo mueve `hi` a un mes que acaba de comprobar BUENO y `lo` a uno que acaba de
/// comprobar MALO, así que **lo que se devuelve siempre se ejecutó y cumplió**. Agotar el
/// presupuesto no invalida nada: solo deja el intervalo más ancho de lo que podría estar, y el
/// resultado sigue siendo un mes verificado (lo que se pierde es minimalidad, no validez — la
/// misma frase que gobierna `crates/engine/src/solve.rs`).
fn bisect_month<F>(lo_fails: u32, hi_ok: u32, max_draws: u32, mut meets: F) -> Result<u32, McError>
where
    F: FnMut(u32) -> Result<bool, McError>,
{
    debug_assert!(lo_fails < hi_ok);
    let (mut lo, mut hi) = (lo_fails, hi_ok);
    let mut budget = max_draws;
    while hi - lo > 1 && budget > 0 {
        let mid = lo + (hi - lo) / 2;
        if meets(mid)? {
            hi = mid;
        } else {
            lo = mid;
        }
        budget -= 1;
    }
    Ok(hi)
}

/// La rejilla de la fase (A): de `k_min` a `h` a pasos de [`BRACKET_STEP_MONTHS`], **cerrando
/// siempre en `h`** y sin pasar de [`MAX_BRACKET_DRAWS`] puntos.
///
/// Cerrar en `h` no es cosmético: con `k_min = 1` y `h = 840` la progresión de 60 en 60 termina en
/// 781, y una fecha que solo existiera en los últimos 59 meses del horizonte no se encontraría
/// nunca. El paso se ESTIRA si el horizonte no cabe en el presupuesto (horizontes por encima de
/// los 840 meses de la app): un bracket más ancho da una respuesta menos ajustada, pero seguir
/// siendo válida — y es preferible a no sondear la cola.
fn bracket_grid(k_min: u32, h: u32) -> Vec<u32> {
    debug_assert!(k_min <= h);
    let span = h - k_min;
    let step = BRACKET_STEP_MONTHS.max(span.div_ceil(MAX_BRACKET_DRAWS - 1).max(1));
    let mut grid = Vec::with_capacity(MAX_BRACKET_DRAWS as usize);
    let mut m = k_min;
    while m < h && (grid.len() as u32) < MAX_BRACKET_DRAWS - 1 {
        grid.push(m);
        m += step;
    }
    grid.push(h);
    grid
}

/// Los candidatos de la fase (B): los aniversarios estrictamente dentro de `(lo, hi)`.
///
/// Un bracket de 60 meses da exactamente cuatro (`lo+12, +24, +36, +48`). Si el bracket viniera
/// más ancho —paso estirado— se **submuestrean uniformemente** hasta [`MAX_ANNUAL_DRAWS`] en vez
/// de recorrer los cuatro primeros y dejar el resto del bracket sin mirar.
fn annual_candidates(lo: u32, hi: u32) -> Vec<u32> {
    let mut candidates: Vec<u32> = (1u32..)
        .map(|i| lo + 12 * i)
        .take_while(|m| *m < hi)
        .collect();
    let max = MAX_ANNUAL_DRAWS as usize;
    if candidates.len() > max {
        let stride = candidates.len().div_ceil(max);
        candidates = candidates.into_iter().step_by(stride).take(max).collect();
    }
    candidates
}

/// El acumulador de la mejor observación del solve.
#[derive(Debug, Default)]
struct Best(Option<SuccessAt>);

impl Best {
    /// Mejor = mayor `wilson_low`; en empate, el mes MÁS TEMPRANO. Se compara por la cota
    /// inferior y no por el estimador puntual porque es la magnitud con la que se decide el
    /// umbral: dos meses con el mismo `éxito` y distinto `N` no valen lo mismo, y el de más
    /// evidencia gana solo.
    fn offer(&mut self, s: &SuccessAt) {
        let better = match &self.0 {
            None => true,
            Some(b) => {
                s.wilson_low > b.wilson_low || (s.wilson_low == b.wilson_low && s.month < b.month)
            }
        };
        if better {
            self.0 = Some(*s);
        }
    }

    fn pair(&self) -> Option<(u32, f64)> {
        self.0.map(|s| (s.month, s.success))
    }
}

/// El resultado «no hay fecha»: `month: None` y, en su lugar, la mejor observación del solve. Las
/// cifras de éxito describen ESA observación, no una fecha — `month` es lo que manda.
fn no_date(best: &Best, draws_search: u32, draws_confirm: u32) -> RetirementDateSolve {
    let b = best.0;
    RetirementDateSolve {
        month: None,
        success: b.map_or(0.0, |s| s.success),
        wilson_low: b.map_or(0.0, |s| s.wilson_low),
        half_width_pp: b.map_or(0.0, |s| s.half_width_pp),
        rule_of_three_upper: b.and_then(|s| s.rule_of_three_upper),
        predecessor_success: None,
        date_is_approximate: false,
        draws_search,
        draws_confirm,
        failures_by_kind: b.map_or([0; 3], |s| s.by_kind),
        best_effort: best.pair(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **La aritmética de Wilson, con los números escritos antes de correr.**
    ///
    /// Con `p̂ = 1` y `n = 2.500`: `z² = 3,8416`, `z²/n = 0,00153664`,
    /// `centro = (1 + 0,00076832)/1,00153664 = 0,99923286`,
    /// `margen = (1,96/1,00153664)·√(3,8416/2,5e7) = 1,956993 · 3,92001e-4 = 7,67142e-4`,
    /// `low = 0,99846572` ⇒ barra hacia abajo **0,1534 pp**.
    #[test]
    fn wilson_with_zero_failures_is_derived_by_hand() {
        let s = SuccessAt::new(120, 2_500, 0, [0; 3]);
        assert_eq!(s.success, 1.0);
        assert!(
            (s.wilson_low - 0.998_465_7).abs() < 1e-6,
            "wilson_low = {}",
            s.wilson_low
        );
        assert!(
            (s.half_width_pp - 0.153_43).abs() < 1e-4,
            "half_width_pp = {}",
            s.half_width_pp
        );
        assert_eq!(s.rule_of_three_upper, Some(3.0 / 2_500.0));
    }

    /// **La forma cerrada de Wilson con `p̂ = 1`**, y la cota de muestra que se deduce de ella.
    #[test]
    fn with_zero_failures_wilson_is_n_over_n_plus_z_squared() {
        let z2 = WILSON_Z_95 * WILSON_Z_95;
        for n in [1u32, 60, 73, 120, 381, 500, 2_500, 5_000] {
            let s = SuccessAt::new(1, n, 0, [0; 3]);
            let closed = f64::from(n) / (f64::from(n) + z2);
            assert!(
                (s.wilson_low - closed).abs() < 1e-12,
                "n = {n}: {} vs {closed}",
                s.wilson_low
            );
            assert!(s.half_width_pp > 0.0, "n = {n}: la barra nunca es 0");
        }
        // `n ≥ z²·u/(1−u)`: 73 para el 95 %, 381 para el 99 %.
        assert!(!SuccessAt::new(1, 72, 0, [0; 3]).meets(95));
        assert!(SuccessAt::new(1, 73, 0, [0; 3]).meets(95));
        assert!(!SuccessAt::new(1, 380, 0, [0; 3]).meets(99));
        assert!(SuccessAt::new(1, 381, 0, [0; 3]).meets(99));
        // Y el 100 % no depende de `N` en absoluto: es «cero fallos».
        assert!(SuccessAt::new(1, 1, 0, [0; 3]).meets(100));
    }

    #[test]
    fn the_bracket_grid_always_closes_on_the_horizon() {
        let g = bracket_grid(1, 840);
        assert_eq!(g.first(), Some(&1));
        assert_eq!(g.last(), Some(&840));
        assert!(g.len() as u32 <= MAX_BRACKET_DRAWS, "{} puntos", g.len());
        assert!(g.windows(2).all(|w| w[0] < w[1]), "rejilla no creciente");
        // Un solo mes disponible: una sola sonda, y es el horizonte.
        assert_eq!(bracket_grid(840, 840), vec![840]);
        // Horizonte gigante: el paso se estira y el presupuesto no se rompe.
        let g = bracket_grid(1, 5_000);
        assert!(g.len() as u32 <= MAX_BRACKET_DRAWS);
        assert_eq!(g.last(), Some(&5_000));
    }

    #[test]
    fn the_annual_grid_of_a_sixty_month_bracket_is_exactly_four() {
        assert_eq!(annual_candidates(100, 160), vec![112, 124, 136, 148]);
        assert!(annual_candidates(100, 112).is_empty());
        assert!(annual_candidates(100, 101).is_empty());
        assert!(annual_candidates(100, 400).len() as u32 <= MAX_ANNUAL_DRAWS);
    }

    /// La bisección devuelve SIEMPRE un extremo bueno, y agotar el presupuesto no lo cambia.
    #[test]
    fn bisect_month_returns_a_verified_high_end() {
        // Función escalón: cumple a partir de 137.
        let mut calls = 0u32;
        let hi = bisect_month(100, 160, 6, |m| {
            calls += 1;
            Ok(m >= 137)
        })
        .unwrap();
        assert_eq!(hi, 137);
        assert!(calls <= 6, "{calls} sorteos");
        // Presupuesto de 1: no llega a 137, pero lo que devuelve cumple igual.
        let hi = bisect_month(100, 160, 1, |m| Ok(m >= 137)).unwrap();
        assert!(hi >= 137 && hi <= 160, "{hi}");
    }

    #[test]
    fn the_threshold_rule_is_wilson_below_a_hundred_and_zero_failures_at_a_hundred() {
        let clean = SuccessAt::new(1, 300, 0, [0; 3]);
        assert!(clean.meets(100));
        assert!(clean.meets(95));
        let one = SuccessAt::new(1, 300, 1, [1, 0, 0]);
        assert!(!one.meets(100), "un fallo de 300 no es el 100 %");
        assert!(one.success > 0.99);
        assert!(one.meets(95), "wilson_low = {}", one.wilson_low);
        assert!(!one.meets(99), "wilson_low = {}", one.wilson_low);
        // Un umbral imposible no se recorta: simplemente no lo cumple nadie.
        assert!(!clean.meets(101));
    }
}
