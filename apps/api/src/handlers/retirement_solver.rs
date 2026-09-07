//! **El ÚNICO sitio de la API donde el sorteo elige un mes** (WP A3 del modelo v2 de jubilación,
//! 5.0.0).
//!
//! Desde 5.0.0 la pregunta «¿cuándo me puedo jubilar?» no la contesta un cruce determinista contra
//! un objetivo: la contesta el **umbral de éxito sobre miles de caminos**
//! (`crates/engine-stochastic`). Este módulo es la frontera entre esa maquinaria y el ensamblado
//! HTTP, y es deliberadamente **la única**: si `projection.rs`, `projection_bands.rs` y
//! `simulate_projection` decidieran cada uno su presupuesto de sorteos, la misma instalación
//! publicaría tres fechas distintas en tres pantallas — que es exactamente el fallo que
//! `compute_strategy_solves` existía para evitar en 4.15.x, con solves deterministas.
//!
//! # La doctrina, escrita una sola vez
//!
//! **Se BUSCA con [`SOLVE_SEARCH_PATHS`] caminos y se CONFIRMA con [`SOLVE_CONFIRM_PATHS`].** La
//! partición no es un ajuste: sale de los ~0,2 ms por camino que mide
//! `crates/engine-stochastic/tests/timing_mc.rs`, y de que los caminos de la búsqueda son un
//! **prefijo exacto** de los de la confirmación (números aleatorios comunes, `path_rng(seed, p)`
//! no depende de `k`). La confirmación no es una segunda muestra: es la misma, ampliada. Por eso
//! puede desmentir a la búsqueda sin contradecirla, y por eso lo que se publica se **ejecutó**.
//!
//! El presupuesto es un **número conocido de sorteos**, no un umbral de convergencia. Los dos
//! contadores viajan en [`PlanLevel1::draws_search`] / [`PlanLevel1::draws_confirm`], así que el
//! coste de una respuesta es siempre `draws_search · t(500) + draws_confirm · t(2.500)` y se puede
//! comprobar sin cronómetro.
//!
//! **La cota inferior del presupuesto no es estética.** Con cero fallos el intervalo de Wilson
//! colapsa a `n/(n + z²)`, así que un umbral `u < 100` es INALCANZABLE con menos de `z²·u/(1−u)`
//! caminos: 73 para el 95 %, **381 para el 99 %**. El perfil admite umbrales de 80 a 100, luego
//! los dos presupuestos tienen que estar por encima de 381 o habría umbrales que ninguna muestra
//! limpia podría satisfacer. Lo pinea [`tests::the_budgets_can_reach_the_highest_profile_threshold`].
//!
//! # Coste medido (release, caso P9 de 840 meses)
//!
//! De `crates/engine-stochastic/tests/timing_mc.rs` — **mide, no afirma**: ≈ 105 ms por sorteo de
//! 500 caminos y ≈ 455 ms por sorteo de 2.500.
//!
//! | solve | nivel | coste típico |
//! |---|---|---|
//! | fecha válida (`valid_retirement_month`) | 1 | **1,3–1,9 s** |
//! | capital necesario hoy (`needed_capital_today`) | 1 | **≈ 2,8 s** |
//! | aportación mínima (`minimum_extra_contribution`) | 1 | **≈ 1,9 s** |
//! | coast (`coast_stop_month`) | 1 | **≈ 1,2 s** |
//! | media jornada (`earliest_partial_start`, con UNA fecha anidada) | 1 | **≈ 2,4 s** |
//! | curva de capital por edad (`needed_capital_curve`) | 2 | **≈ 16 s** |
//!
//! De ahí la partición en **dos niveles**, que es la decisión de producto de este módulo:
//!
//! - **Nivel 1** ([`solve_plan_level1`]) es lo que la serie NO puede publicar sin: la fecha, el
//!   éxito del plan y el capital necesario hoy. Corre **síncrono, dentro del miss de proyección**
//!   y bajo el semáforo de simulaciones (`heavy::run_projection_sim`) — el llamante espera.
//! - **Nivel 2** ([`spawn_plan_extras`]) es todo lo que ilustra pero no decide: las fechas al
//!   100 % y al 90 %, la curva de capital por edad, el éxito por año de jubilación y el fallo
//!   acumulado por edad. Corre en `tokio::spawn` bajo **UN** permiso del semáforo, deduplicado por
//!   [`AppState::plan_inflight`], y hasta que termina los lectores publican `computing`.
//!
//! Meter el nivel 2 en el nivel 1 sumaría ~25 s a un GET. Publicar el nivel 1 en segundo plano
//! dejaría la pantalla de Jubilación sin fecha hasta un segundo sondeo.
//!
//! # Rejillas y presupuestos del nivel 2
//!
//! - **Curva de capital**: la rejilla es `{1, 1+`[`CURVE_GRID_MONTHS`]`, …} ∪ {mes del plan}`
//!   acotada al horizonte, con [`CURVE_PATHS`] caminos y **sin confirmación** (la cifra que se
//!   confirma es la de HOY, que es de nivel 1). Un nodo sin cifra —`absent_reason`— **no se
//!   publica como 0 €**: sale del vector de importes y su razón viaja en
//!   [`PlanExtras::needed_capital_curve_absent`], para que quien dibuja pueda **partir la línea**
//!   ahí en vez de unir dos puntos por encima de un hueco.
//!
//!   **Qué mide cada nodo (decisión C9 del owner, 2026-09-07): «lo que hay que TENER (líquido) a
//!   esa edad para jubilarse entonces al umbral».** El crate fija la acumulación hasta `k−1` en la
//!   línea DETERMINISTA y sortea solo desde `k`, así que todos los caminos llegan al nodo con el
//!   mismo líquido y el nodo publica ese líquido. Antes de C9 sorteaba también la acumulación y
//!   publicaba la MEDIANA del hogar escalado, que arrastraba la dispersión de treinta años y el
//!   sobrecoste de Wilson sobre ella: medido sobre P9 en release, la curva salía **monótona
//!   creciente de 2,9 M€ a 93,5 M€ (euros de hoy) entre el mes 1 y el 840** — el ahorro que el
//!   hogar acumula multiplicado por su propia incertidumbre, no un capital necesario. Condicionada
//!   se queda en 2,4–2,9 M€ y baja a 737 k€ en el horizonte. **El coste no cambia** (15,62 s antes,
//!   15,68 s después: la simulación recorre el horizonte entero igual).
//!
//! # Las dos bases de los euros, dichas aquí porque no son la misma
//!
//! | cifra | base | de dónde sale |
//! |---|---|---|
//! | [`PlanLevel1::needed_capital_today`] | **euros de HOY** | `NeededCapital::amount_today` del nodo `k = 1` |
//! | [`PlanExtras::needed_capital_curve`] | **euros NOMINALES de cada mes** | `NeededCapital::amount_nominal` de cada nodo — «lo que hay que TENER a esa edad» (C9) |
//!
//! No es una inconsistencia, es lo que cada una tiene que ser. El capital de hoy es una cifra que
//! el usuario compara con su cartera de hoy: en euros de hoy. La curva se dibuja **contra la
//! trayectoria del patrimonio**, que es nominal, así que un nodo deflactado cruzaría la línea en
//! el mes equivocado; quien quiera verla «en dinero de hoy» deflacta los dos a la vez con el
//! MISMO factor, que es lo que hace la SPA. En `k = 1` las dos coinciden exactamente —el factor
//! de inflación en el índice 0 es 1— y por eso el primer nodo de la curva y el capital de hoy son
//! el mismo número.
//! - **Éxito por año**: `success_by_retirement_month` sobre los aniversarios del suelo del plan,
//!   con [`CURVE_PATHS`] caminos y a lo sumo [`YEARLY_GRID_MAX_NODES`] nodos. Con el horizonte
//!   máximo de la app (840 meses) son 70 sorteos ⇒ **≈ 7,4 s**.
//! - **Fallo acumulado por edad**: sale de `project_percentile_bands` con
//!   [`SOLVE_CONFIRM_PATHS`] caminos y la semilla del plan, es decir **de la misma muestra que
//!   publica `GET /v1/projection/bands`** — no de un `success_at_month` aparte. Es deliberado:
//!   `success_at_month` cuenta cuántos caminos fallan, no CUÁNDO, así que no puede producir una
//!   curva por edad; y dos ejecuciones con presupuestos distintos darían dos probabilidades del
//!   mismo plan en dos pantallas. Por eso [`SOLVE_CONFIRM_PATHS`] **es** `DEFAULT_BANDS_PATHS` y
//!   no un número escrito aparte.
//!
//! # La clave es el CONTENIDO
//!
//! [`PlanKey`] es un hash de la entrada entera (`format!("{input:?}")`), las volatilidades, el
//! umbral, los caminos y la semilla. **Direccionada por contenido**: dos peticiones que describen
//! el mismo hogar comparten entrada, y una mutación mueve la clave. De ahí la propiedad que
//! gobierna la cache del plan y que se dice en `state.rs` y en `.claude/api-routes.md`:
//!
//! > el `plan_cache` **no entra en `invalidate_projection_by_*`**, porque una entrada obsoleta es
//! > **inalcanzable**, nunca peligrosa: nadie puede volver a pedirla sin reconstruir exactamente
//! > el hogar que la produjo. Lo único que hace falta es que no crezca sin límite, y de eso se
//! > ocupan el TTL (el de la proyección) y el tope LRU de [`PLAN_CACHE_MAX_ENTRIES`].
//!
//! El hash es de **proceso**: `DefaultHasher` (SipHash) no garantiza estabilidad entre versiones
//! de Rust, y da igual — la cache vive en memoria y muere con el proceso. Es la diferencia con
//! `seed_for`, que sí necesita ser estable entre builds (una semilla que cambia al actualizar el
//! toolchain es una semilla que no existe) y por eso escribe su propio FNV-1a.
//!
//! # El escenario que se hashea es el que se simula
//!
//! [`plan_scenario`] construye, en UN sitio, la entrada que el plan describe: las mutaciones que
//! el nivel 1 decidió (mes forzado, mes de corte de aportaciones, inicio de la media jornada)
//! aplicadas sobre la entrada del ensamblado. **Lo que se pasa a [`plan_fingerprint`] y lo que se
//! pasa a [`spawn_plan_extras`] tienen que ser ese mismo valor**: si divergieran, la clave dejaría
//! de describir lo que hay dentro y la cache serviría los extras de otro plan. `spawn_plan_extras`
//! lo comprueba con un `debug_assert`.
//!
//! # Lo que este módulo NO promete
//!
//! **La minimalidad de ninguna de sus respuestas.** Se devuelve un mes VERIFICADO que cumple, no
//! el mínimo demostrable: la monotonía del éxito en `k` se rompe con «Próximos» fechados, con
//! fases parciales caras y con la inflación por encima del crecimiento neto, y encima la medición
//! es muestral. Es la misma frase que gobierna `crates/engine/src/solve.rs` y `solve_mc`, y aquí
//! no se suaviza. Quien quiera saber si hay algo antes tiene `predecessor_success` del crate.
//!
//! **Un plan sin fecha no es un plan con fecha 0.** [`PlanLevel1::retirement_date_basis`] vale
//! `not_reachable`, [`PlanLevel1::forced_month`] es `None` y lo único que hay que enseñar es
//! [`PlanLevel1::best_effort`] («lo más cerca que llegas es el 78 % a los 67»). El llamante
//! simula entonces la línea determinista **sin jubilación** — en la práctica,
//! `RetirementTrigger::AtMonth(horizonte + 1)` con el cruce en solo-lectura, que es lo que
//! [`plan_scenario`] construye para que el ancla del nivel 2 esté definida.
//!
//! # De aquí salen euros, y se dice de dónde
//!
//! Dos importes cruzan esta frontera: [`PlanLevel1::needed_capital_today`] y
//! [`PlanLevel1::contribution_required_monthly`] (con su techo de búsqueda). Los dos vienen ya
//! **redondeados hacia arriba** por el crate —a cientos el capital, a decenas la aportación— y
//! **no se recalculan aquí**: se pasan tal cual. El crate declara por qué puede publicarlos
//! (D4 enmendado: su error es de MUESTREO, no del tipo numérico) y por qué el redondeo es hacia
//! arriba y no al más cercano. Todo lo demás que sale de aquí son meses, contadores y
//! proporciones.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use futurefin_engine::ProjectionInput;
use futurefin_engine_stochastic::{
    coast_stop_month, earliest_partial_start, minimum_extra_contribution, needed_capital_curve,
    needed_capital_today, partial_starting_at, project_percentile_bands, retiring_at,
    stopping_at, success_at_month, success_by_retirement_month, valid_retirement_month, McConfig,
    McError, NeededCapital, RetirementDateSolve, SuccessAt,
};
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;

use crate::error::ApiError;
use crate::handlers::projection_bands::{map_mc_err, DEFAULT_BANDS_PATHS};
use crate::state::{AppState, PlanCacheSlot, PROJECTION_CACHE_TTL};

// =================================================================================================
// Presupuestos y rejillas — todos los números del módulo, en un solo sitio
// =================================================================================================

/// Caminos de la fase de **BÚSQUEDA** de todos los solves. ≈ 105 ms por sorteo sobre P9.
///
/// Está por encima de los 381 que exige el umbral máximo del perfil por debajo de 100 (ver el doc
/// del módulo), así que ningún umbral configurable es inalcanzable por falta de muestra.
pub(crate) const SOLVE_SEARCH_PATHS: u32 = 500;

/// Caminos de la fase de **CONFIRMACIÓN**, y del sorteo del que sale el fallo acumulado por edad.
///
/// **Es literalmente `DEFAULT_BANDS_PATHS`, no un 2.500 escrito aparte.** Esa identidad es
/// load-bearing: hace que la probabilidad de éxito que confirma la fecha y la que dibuja el fan
/// chart salgan de **la misma muestra**, y no de dos ejecuciones que darían dos números del mismo
/// plan en dos pantallas. Un literal aquí sería la trampa de los defaults duplicados que CLAUDE.md
/// nombra: dos números que hay que acordarse de mover a la vez.
///
/// **Hoy vale lo que valga `DEFAULT_BANDS_PATHS`**; el WP A6 lo sube de 500 a 2.500. Los costes de
/// la tabla del doc del módulo están calculados con 2.500, que es el destino declarado.
pub(crate) const SOLVE_CONFIRM_PATHS: u32 = DEFAULT_BANDS_PATHS;

/// **El presupuesto de sorteos de un nivel 1**, como un valor y no como dos constantes leídas
/// desde dentro.
///
/// Existe por `simulate_projection` (WP A8): un what-if simula DOS planes —baseline y escenario— y
/// pagar dos veces la confirmación de 2.500 caminos convierte una pregunta conversacional en diez
/// segundos de espera. El eje no es «cuánta precisión quiero» sino «cuánto estoy dispuesto a
/// pagar», así que viaja como parámetro y **se PUBLICA** ([`PlanLevel1::paths_used`], que la API
/// sirve como `date_solved_with_paths`): una probabilidad sin su tamaño de muestra no se compara
/// con nada, y dos lados medidos con presupuestos distintos no se restan.
///
/// **Los dos lados de una misma llamada usan SIEMPRE el mismo presupuesto y la misma semilla.** Lo
/// contrario haría que un delta mezclara el cambio del plan con el ruido de dos muestras de
/// tamaños distintos — exactamente el fallo que la doctrina del módulo existe para evitar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanBudget {
    /// Caminos de la fase de búsqueda.
    pub search_paths: u32,
    /// Caminos de la fase de confirmación. **Nunca menor que [`Self::search_paths`]**: confirmar
    /// con menos muestra que la búsqueda desmentiría una medición con otra peor.
    pub confirm_paths: u32,
}

impl PlanBudget {
    /// El presupuesto **completo**: busca con [`SOLVE_SEARCH_PATHS`] y confirma con
    /// [`SOLVE_CONFIRM_PATHS`]. Es el de `GET /v1/projection/series`, el de `/v1/projection/bands`
    /// y el que produce la fecha que el resto de la app publica — un plan resuelto con él es, bit a
    /// bit, el mismo plan.
    pub(crate) const FULL: Self = Self {
        search_paths: SOLVE_SEARCH_PATHS,
        confirm_paths: SOLVE_CONFIRM_PATHS,
    };

    /// Solo **búsqueda**: la fase de confirmación se ejecuta igual, pero con el mismo tamaño de
    /// muestra que la búsqueda. Ni se salta un paso ni se publica una fecha sin verificar; lo que
    /// se reduce es la MUESTRA, y la respuesta lo dice publicando `date_solved_with_paths`.
    ///
    /// Es el presupuesto por defecto del what-if: cuesta ~1/5 y contesta la pregunta que un
    /// what-if hace de verdad —«¿en qué dirección y cuánto se mueve mi fecha?»—, que es un DELTA
    /// entre dos lados medidos igual, no una fecha para grabar en piedra.
    pub(crate) const SEARCH_ONLY: Self = Self {
        search_paths: SOLVE_SEARCH_PATHS,
        confirm_paths: SOLVE_SEARCH_PATHS,
    };
}

/// Paso de la rejilla de la **curva de capital necesario por edad**: cinco años.
///
/// Mismo paso que `FAILURE_STEP_MONTHS` del crate y que el bracket de `valid_retirement_month`, y
/// por la misma razón: catorce nodos cubren el horizonte máximo (840 meses) y cada nodo cuesta un
/// bracket + una bisección con warm start.
pub(crate) const CURVE_GRID_MONTHS: u32 = 60;

/// Caminos por nodo de la curva y de la tira anual: los de BÚSQUEDA. La curva **no confirma nada**
/// (lo dice su doc en el crate): dibuja la forma, no publica un compromiso.
pub(crate) const CURVE_PATHS: u32 = SOLVE_SEARCH_PATHS;

/// Paso de la tira **éxito por año de jubilación**.
pub(crate) const YEARLY_GRID_STEP_MONTHS: u32 = 12;

/// Tope de nodos de esa tira. Con el horizonte máximo de la app (840 meses) la rejilla anual tiene
/// exactamente 70 nodos y el tope no recorta nada; existe para que un horizonte mayor no convierta
/// una tira informativa en un bucle sin cota — 70 sorteos de 500 caminos ya son ≈ 7,4 s.
pub(crate) const YEARLY_GRID_MAX_NODES: usize = 70;

/// Entradas máximas de la cache de plan (nivel 2), desalojadas por LRU sobre `last_used`.
///
/// 256 es holgado para una instalación doméstica —una entrada por hogar × perfil × umbral vivo— y
/// acota la memoria: lo que se guarda son meses, contadores y una curva de catorce importes, del
/// orden de un kilobyte por entrada.
pub(crate) const PLAN_CACHE_MAX_ENTRIES: usize = 256;

/// TTL de la cache de plan: **el mismo de la proyección**, deliberadamente.
///
/// No hay ninguna razón para que los extras de un plan sobrevivan a la serie que los enseña, y dos
/// TTL distintos serían dos números que hay que acordarse de mover a la vez.
pub(crate) const PLAN_CACHE_TTL: Duration = PROJECTION_CACHE_TTL;

/// Decimales del redondeo de PUBLICACIÓN de la barra de error, en puntos porcentuales. Cuatro son
/// dos órdenes de magnitud por debajo de la resolución real (con 2.500 caminos un camino vale
/// 0,04 pp) y bastan para que «0,1534 pp» no se lea como «0,15».
const SAMPLING_ERROR_DP: u32 = 4;

// =================================================================================================
// Códigos de ausencia y de fallo — literales, nunca compuestos
// =================================================================================================

/// El nivel 2 terminó bien. Estado de [`PlanExtras::state`].
pub const PLAN_EXTRAS_READY: &str = "ready";
/// El sorteo del nivel 2 no pudo completarse: el motor rechazó la entrada. El llamante publica
/// `unavailable`; el detalle vive en el log, no en el wire.
pub const PLAN_EXTRAS_FAILED_ENGINE: &str = "engine_failed";
/// La configuración de Monte Carlo del nivel 2 era inválida. Inalcanzable con las constantes de
/// este módulo; se nombra en vez de suponerse.
pub const PLAN_EXTRAS_FAILED_CONFIG: &str = "invalid_mc_config";
/// La tarea del nivel 2 se cayó (pánico) o el semáforo estaba cerrado.
pub const PLAN_EXTRAS_FAILED_TASK: &str = "task_failed";

/// Un nodo de la curva sin importe **y sin razón**: inalcanzable hoy (`NeededCapital` garantiza
/// que las dos ausencias viajan juntas) y nombrado igualmente, porque un `None` sin motivo se
/// leería como un fallo del dibujo y no como una medición que no se pudo hacer.
pub const CURVE_NODE_ABSENT_UNKNOWN: &str = "unknown";

/// `retirement_date_basis`: la fecha la decidió el **umbral de éxito** (estrategias `asap`,
/// `coast` modo B y `partial`).
pub const DATE_BASIS_SUCCESS_THRESHOLD: &str = "success_threshold";
/// `retirement_date_basis`: la fecha es un DATO del usuario — la edad objetivo (`retire_at_age`,
/// `coast` modo A). El umbral no la mueve; lo que dice es si se llega.
pub const DATE_BASIS_TARGET_AGE: &str = "target_age";
/// `retirement_date_basis`: **no hay ninguna fecha en el horizonte** que cumpla el umbral. Nunca
/// un mes 0, que se leería como «ya puedes».
pub const DATE_BASIS_NOT_REACHABLE: &str = "not_reachable";

// =================================================================================================
// El perfil, visto por el SOLVER
// =================================================================================================

/// **Qué solve corresponde a esta estrategia.** Cuatro variantes, sin brazo comodín.
///
/// Es un enum PROPIO y no `handlers::retirement_profile::RetirementStrategy` a propósito, y la
/// razón no es el desacoplamiento por el desacoplamiento: el enum del perfil carga cosas de
/// producto que aquí no significan nada —el alias `pension_bridge` que migra, la distinción entre
/// lo almacenado y lo resuelto, la validación de qué campos son obligatorios— y este solo tiene
/// que contestar «¿qué bisección hay que correr?». Los literales son los MISMOS que serializa el
/// perfil ([`PlanStrategy::from_wire`]), así que la traducción vive en un sitio y se puede probar
/// sin construir un perfil entero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlanStrategy {
    /// «En cuanto pueda»: la fecha la decide el umbral.
    Asap,
    /// «A los N años»: la fecha es un dato y lo que se resuelve es cuánto falta aportar.
    RetireAtAge,
    /// Coast: dejar de aportar antes de jubilarse. Dos modos, ver [`CoastSolveMode`].
    Coast,
    /// Media jornada antes de la jubilación total. Dos modos, ver [`PartialSolveMode`].
    Partial,
}

impl PlanStrategy {
    // **No hay `as_wire`** (retirado en 5.0.0, WP A12). Existía como gemelo de `from_wire` y no lo
    // llamaba nadie salvo un test que lo comparaba consigo mismo: el literal que de verdad se
    // publica no sale de aquí, sale de `serde` sobre `RetirementStrategy`
    // (`projection::strategy_label`), así que este método era una SEGUNDA tabla de los mismos
    // cuatro literales — la clase de duplicado que diverge en el primer renombrado y no se entera
    // nadie. Lo que hay que pinear es el cruce real, y lo pinea
    // `the_profile_literals_are_the_ones_the_solver_accepts` contra `strategy_label`.

    /// Traducción desde el literal del perfil. `None` = literal desconocido — el llamante decide
    /// qué hacer con él (hoy: degradar a `asap` con aviso, nunca reventar una LECTURA).
    ///
    /// El alias `pension_bridge` de 4.15.x **no se acepta aquí**: el perfil lo resuelve a `asap`
    /// con el puente encendido antes de llegar a este módulo (A1), así que aceptarlo también aquí
    /// sería un segundo sitio donde la migración vive.
    pub(crate) fn from_wire(s: &str) -> Option<Self> {
        match s {
            "asap" => Some(PlanStrategy::Asap),
            "retire_at_age" => Some(PlanStrategy::RetireAtAge),
            "coast" => Some(PlanStrategy::Coast),
            "partial" => Some(PlanStrategy::Partial),
            _ => None,
        }
    }
}

/// Los dos modos de `coast`, en términos de **qué se resuelve y qué es dato**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CoastSolveMode {
    /// **Modo A** (`fixed_retirement_age`): la edad de jubilación es dato (`target_month = R`) y
    /// se resuelve el PRIMER mes en que se puede dejar de aportar.
    FixedRetirementAge,
    /// **Modo B** (`fixed_stop_age`): el mes de corte es dato (`coast_stop_month = C`) y se
    /// resuelve la fecha de jubilación con las aportaciones cortadas.
    FixedStopAge,
}

/// Los dos modos de la media jornada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PartialSolveMode {
    /// **Modo A** (`at_age`): el inicio es dato (`partial_start_month = S`) y se resuelve la
    /// jubilación total desde `S + 1`.
    AtAge,
    /// **Modo B** (`asap`): se resuelve el primer mes en que la fase puede empezar y, con esa fase
    /// dentro, la jubilación total.
    Asap,
}

/// **Lo que el solver necesita saber del perfil, y nada más.**
///
/// Lo construye el ensamblado (WP A4) a partir del `RetirementProfile` ya resuelto y de la fecha
/// de nacimiento del usuario cuya jubilación se simula. Todos los meses son **meses del BUCLE**
/// (1-based, la rejilla de `RetirementTrigger::AtMonth`), nunca meses de la rejilla publicada:
/// convertir es cosa del llamante, con `engine_month_to_grid`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PlanSolveProfile {
    pub strategy: PlanStrategy,
    /// Umbral de éxito del perfil, 80..=100. Se aplica tal cual: **nunca se recorta en silencio**
    /// (un umbral recortado es una promesa que el usuario no hizo). La cota la valida el perfil.
    pub threshold_pct: u32,
    /// **`R`** — el mes del bucle en que manda la EDAD. Obligatorio en `retire_at_age` y en
    /// `coast` modo A; ignorado en el resto. `None` con una estrategia que lo necesita degrada a
    /// `asap` (§A del plan: «nunca un 500»).
    pub target_month: Option<u32>,
    /// Qué modo de coast. Ignorado fuera de [`PlanStrategy::Coast`].
    pub coast_mode: CoastSolveMode,
    /// **`C`** — el mes desde el que NO se aporta, cuando es dato (coast modo B). En modo A lo
    /// resuelve el solve y este campo se ignora.
    pub coast_stop_month: Option<u32>,
    /// Qué modo de media jornada. Ignorado fuera de [`PlanStrategy::Partial`].
    pub partial_mode: PartialSolveMode,
    /// **`S`** — el mes en que empieza la media jornada, cuando es dato (partial modo A). En modo
    /// B lo resuelve el solve.
    pub partial_start_month: Option<u32>,
    /// **El puente a la pensión**, `(P, max_years)`, cuando está activado.
    ///
    /// **`P` es un mes del BUCLE (1-based)**, no el `start_index` 0-based de
    /// `PensionSchedule` — el ensamblado convierte con `+ 1`. La asimetría entre las dos rejillas
    /// está declarada en `phases.rs` y es exactamente la clase de off-by-one que mueve una fecha
    /// un mes sin que nada falle.
    ///
    /// Aquí el puente hace **una sola cosa**: baja el suelo de la búsqueda de fechas a
    /// `max(1, P − 12·max_years)`. El tope de tasa inicial que el puente levanta es cosa del
    /// motor (`InitialRateGate::bridge`), y confundir los dos era el error que el panel
    /// adversarial midió.
    pub bridge: Option<(u32, u32)>,
}

impl PlanSolveProfile {
    /// **El suelo de la búsqueda de fechas**, `k_min`, en meses del bucle.
    ///
    /// Sin puente vale 1. Con puente, `max(1, P − 12·max_years)`: no tiene sentido buscar una
    /// fecha de jubilación anterior a lo que el puente puede cubrir, porque el motor la tumbaría
    /// por tasa inicial y cada sondeo de esa zona es un sorteo tirado.
    pub(crate) fn bridge_k_min(&self) -> u32 {
        match self.bridge {
            Some((pension_month, max_years)) => {
                pension_month.saturating_sub(max_years.saturating_mul(12)).max(1)
            }
            None => 1,
        }
    }
}

// =================================================================================================
// La clave del plan
// =================================================================================================

/// **La clave de la cache de plan: un hash del CONTENIDO.**
///
/// Es `pub` —y no `pub(crate)` como el resto del módulo— porque aparece en la superficie pública
/// de `AppState` (`plan_cache`, `ProjectionCacheEntry::plan_key`), igual que `ProjectionCacheKey`.
/// Lo que `pub(crate)` protegía se protege igual: **el campo es privado y el único constructor es
/// [`plan_fingerprint`], que sí es `pub(crate)`**, así que nadie fuera del crate puede acuñar una
/// clave — solo nombrarla.
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub struct PlanKey(u64);

impl PlanKey {
    /// El hash desnudo, para el log. No para reconstruir nada: de un `u64` no vuelve una entrada.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// **La clave de la cache del NIVEL 1**, y por qué NO puede ser [`PlanKey`] (5.0.0, WP A12).
///
/// [`PlanKey`] se calcula sobre el escenario **de después del solve** —el que ya lleva el mes
/// forzado dentro— y por eso sirve para el nivel 2, que cuelga de ese escenario. Para reutilizar
/// el nivel 1 hace falta una clave que se pueda calcular **antes de resolver**, o el hueco y el
/// huevo: no se puede consultar una cache cuya clave necesita el resultado que se busca.
///
/// Es un **tipo propio y no un alias** a propósito. Las dos claves son `u64` y viven en el mismo
/// `AppState`; si compartieran tipo, nada impediría preguntarle al mapa de una con la clave de la
/// otra, y una colisión entre los dos espacios devolvería el plan de otro hogar sin que ningún
/// test lo notara. Con dos tipos distintos eso no compila. Además la huella empieza sembrando un
/// **dominio distinto** ([`LEVEL1_DOMAIN`]), así que las dos funciones no producen el mismo `u64`
/// ni para la misma entrada.
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub struct Level1Key(u64);

impl Level1Key {
    /// El hash desnudo, para el log. Ver [`PlanKey::as_u64`].
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Separador de dominio de [`level1_fingerprint`] frente a [`plan_fingerprint`]. Cualquier valor
/// sirve mientras sea distinto entre los dos; se escribe como constante para que quede claro que
/// el número no significa nada y que lo único que importa es que no se repita.
const LEVEL1_DOMAIN: u64 = 0x4c56_3120; // "LV1 "

/// **La huella de una llamada a [`solve_plan_level1_with_budget`]: sus CINCO argumentos.**
///
/// El nivel 1 es una función pura de `(input, vols, seed, profile, budget)` —no lee reloj, no lee
/// base de datos y su RNG se siembra con `seed`—, así que hashear exactamente esos cinco es
/// necesario y suficiente: dos llamadas con la misma huella devuelven el MISMO `PlanLevel1`, bit a
/// bit, y dos llamadas distintas no pueden compartir huella salvo colisión de SipHash.
///
/// Qué cambia respecto de [`plan_fingerprint`], y por qué:
///
/// - el `input` es el de **antes** del solve (ese es el punto entero de esta clave);
/// - entra el **perfil** (`PlanSolveProfile`, que ya deriva `Hash`), y tiene que entrar: `asap` y
///   `retire_at_age` sobre el mismo hogar comparten `ProjectionInput` y resuelven planes
///   distintos. `plan_fingerprint` puede permitirse omitirlo porque el escenario post-solve ya
///   lleva la decisión dentro; aquí ese escenario todavía no existe;
/// - entra el **presupuesto** entero (los dos tamaños de muestra) y no solo el de confirmación: el
///   what-if resuelve con [`PlanBudget::SEARCH_ONLY`] y publica `date_solved_with_paths` porque la
///   cifra depende de él. Servirle a la serie un nivel 1 medido con 500 caminos sería publicar una
///   fecha «confirmada con 2.500» que nadie confirmó con 2.500.
///
/// Como [`plan_fingerprint`], es una huella de CONTENIDO: una entrada obsoleta es **inalcanzable**
/// (cambiar cualquier dato produce otra clave), así que este mapa tampoco se invalida — lo acotan
/// el TTL y el LRU.
pub(crate) fn level1_fingerprint(
    input: &ProjectionInput,
    vols: &[Option<f64>],
    seed: u64,
    profile: &PlanSolveProfile,
    budget: PlanBudget,
) -> Level1Key {
    let mut h = DefaultHasher::new();
    LEVEL1_DOMAIN.hash(&mut h);
    format!("{input:?}").hash(&mut h);
    // Longitud explícita y bits de cada `f64`, por las mismas dos razones que en
    // `plan_fingerprint`: sin la longitud, `[Some(x)]` y `[Some(x), None]` colisionarían por
    // prefijo; por bits, porque `f64` no es `Hash` y comparar bits es MÁS estricto que `==`.
    vols.len().hash(&mut h);
    for v in vols {
        match v {
            Some(x) => {
                1u8.hash(&mut h);
                x.to_bits().hash(&mut h);
            }
            None => 0u8.hash(&mut h),
        }
    }
    seed.hash(&mut h);
    profile.hash(&mut h);
    // `PlanBudget` no deriva `Hash` (es un par de cotas, no una clave), así que se hashean sus dos
    // campos a mano. Si algún día gana un tercero, este es el sitio que hay que tocar — y por eso
    // se desestructura en vez de leerse por campos: un campo nuevo rompe la compilación aquí.
    let PlanBudget {
        search_paths,
        confirm_paths,
    } = budget;
    search_paths.hash(&mut h);
    confirm_paths.hash(&mut h);
    Level1Key(h.finish())
}

/// **La huella del plan**: entrada + volatilidades + umbral + caminos + semilla.
///
/// # Qué entra y por qué
///
/// - `input`, por su `Debug`. No es elegante y es lo correcto: `ProjectionInput` no implementa
///   `Hash` (lleva `Decimal`, `f64` de volatilidad no, pero sí `Option<Decimal>` y `NaiveDate`), y
///   derivarlo obligaría a tocar `crates/engine` para que la API pueda cachear — la cola moviendo
///   al perro. `format!("{input:?}")` recorre **todos** los campos, así que un campo nuevo entra
///   en la huella solo, que es justo lo que hay que garantizar: una entrada que se olvida de
///   hashear un campo es una cache que sirve el plan de otro hogar.
/// - `vols`, por sus **bits** (`f64::to_bits`): `f64` no es `Hash`, y comparar por bits distingue
///   `0.0` de `-0.0`, que es más estricto que la igualdad y por tanto seguro (a lo sumo falla un
///   hit, nunca acierta uno que no toca).
/// - `threshold_pct`, `paths` y `seed`, porque **los tres cambian la respuesta** sin cambiar la
///   entrada: el umbral mueve la fecha, los caminos mueven la barra de error y la semilla mueve el
///   mercado entero.
///
/// # El hash es de PROCESO, y da igual
///
/// `DefaultHasher` es SipHash y Rust **no garantiza** su algoritmo entre versiones. Aquí no
/// importa: la cache vive en memoria y muere con el proceso, así que nunca se compara una huella
/// de un build con la de otro. Es la diferencia con
/// `futurefin_engine_stochastic::seed_for`, que sí necesita estabilidad entre builds y por eso
/// escribe su propio FNV-1a en vez de usar este.
///
/// # Una entrada obsoleta es INALCANZABLE, no peligrosa
///
/// La clave ES el contenido. Cambiar un activo, un umbral o la semilla produce otra clave, así que
/// la entrada vieja deja de tener quien la pida: no hay forma de servirla por error. Por eso el
/// `plan_cache` **no** entra en `invalidate_projection_by_installation` / `..._by_user` — lo que
/// haría falta ahí es borrar por instalación, y la clave no la lleva ni podría llevarla sin dejar
/// de ser una huella del contenido. Lo único que hay que evitar es que crezca: de eso se ocupan el
/// TTL y el LRU.
///
/// # El escenario que se hashea es el que se simula
///
/// El `input` que se pasa aquí tiene que ser el que produjo [`plan_scenario`] — el que lleva ya el
/// mes forzado, el corte de aportaciones y el inicio de la media jornada que el nivel 1 decidió.
/// Hashear la entrada de ANTES del solve mezclaría en una sola clave dos planes distintos del
/// mismo hogar (por ejemplo `asap` y `retire_at_age`, que solo se diferencian en el trigger).
pub(crate) fn plan_fingerprint(
    input: &ProjectionInput,
    vols: &[Option<f64>],
    threshold_pct: u32,
    paths: u32,
    seed: u64,
) -> PlanKey {
    let mut h = DefaultHasher::new();
    format!("{input:?}").hash(&mut h);
    // Longitud explícita: sin ella `[Some(x)]` y `[Some(x), None]` podrían colisionar por prefijo.
    vols.len().hash(&mut h);
    for v in vols {
        match v {
            Some(x) => {
                1u8.hash(&mut h);
                x.to_bits().hash(&mut h);
            }
            None => 0u8.hash(&mut h),
        }
    }
    threshold_pct.hash(&mut h);
    paths.hash(&mut h);
    seed.hash(&mut h);
    PlanKey(h.finish())
}

// =================================================================================================
// Nivel 1
// =================================================================================================

/// **El resultado del nivel 1**: lo que la serie no puede publicar sin.
///
/// Todos los meses son **meses del BUCLE** (1-based). El llamante los convierte a la rejilla
/// publicada con `engine_month_to_grid` — este módulo no publica JSON y no conoce la rejilla.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanLevel1 {
    /// Quién decidió la fecha: [`DATE_BASIS_SUCCESS_THRESHOLD`], [`DATE_BASIS_TARGET_AGE`] o
    /// [`DATE_BASIS_NOT_REACHABLE`]. Es un literal cerrado, no una frase.
    pub retirement_date_basis: &'static str,
    /// **El mes del bucle que hay que forzar en el `PhasePlan`.**
    ///
    /// `None` ⟺ [`DATE_BASIS_NOT_REACHABLE`], y entonces el llamante simula la línea determinista
    /// **sin jubilación** — `RetirementTrigger::AtMonth(horizonte + 1)` con el cruce en
    /// solo-lectura, que es lo que [`plan_scenario`] construye. Nunca un `Some(0)`: el mes 0 no
    /// existe en el bucle y se leería como «ya puedes».
    pub forced_month: Option<u32>,
    /// Éxito del plan publicado, estimador puntual en `[0, 1]`.
    pub success_of_plan: f64,
    /// Cota inferior del intervalo de Wilson al 95 % del mismo. **Es el número contra el que se
    /// compara el umbral** por debajo de 100 (decisión C3): estable frente a semilla y a `N`, que
    /// es justo lo que el estimador puntual no era.
    pub success_wilson_low: f64,
    /// Distancia del estimador puntual a la cota de Wilson, en **puntos porcentuales**. Es la
    /// barra que se dibuja hacia abajo, que es el lado que decide el umbral. **Nunca 0**, ni con
    /// cero fallos: con 0 de 2.500 vale 0,1534 pp.
    pub success_sampling_error_pp: Decimal,
    /// Caminos con los que se midió lo anterior. Una probabilidad sin su `N` no se puede comparar
    /// con otra, así que viaja al lado.
    pub paths_used: u32,
    /// La semilla del sorteo, ecoada: sin ella el resultado no es reproducible y por tanto no es
    /// un resultado.
    pub seed: u64,
    /// **Capital necesario HOY**, en **euros de HOY**, ya redondeado a cientos **hacia arriba** por
    /// el crate (`NeededCapital::amount_today` del nodo `k = 1`).
    ///
    /// Es la base que el usuario compara con su cartera de hoy, y es **otra** que la de
    /// [`PlanExtras::needed_capital_curve`], que va en euros nominales de cada mes. En `k = 1` las
    /// dos coinciden exactamente: el factor de inflación en el índice 0 es 1.
    ///
    /// `None` ⟺ hay [`Self::needed_capital_absent_reason`] — **nunca un 0 €**, que se leería como
    /// «no necesitas nada».
    pub needed_capital_today: Option<Decimal>,
    /// Por qué no hay capital necesario: `no_liquid_assets` | `threshold_unreachable` |
    /// `month_beyond_horizon` | `already_covered` (literales del crate, no copiados aquí).
    ///
    /// **`already_covered` no es un fallo de medición, es una RESPUESTA**: con la cartera dividida
    /// por 256 el plan sigue cumpliendo el umbral, así que no hace falta capital adicional hoy y la
    /// necesidad queda por debajo de lo que el método sabe medir. Tampoco ahí se publica un 0 €:
    /// la necesidad es «menor que `λ_min × tu líquido», no cero.
    pub needed_capital_absent_reason: Option<&'static str>,
    /// **Aportación extra mensual mínima** para que la fecha sea válida, plana y nominal, ya
    /// redondeada a decenas hacia arriba. Solo existe en las estrategias donde la fecha es un
    /// dato (`retire_at_age`, `coast` modo A). `Some(0)` = «no te falta nada», que es una
    /// respuesta; `None` con [`Self::contribution_underfunded`] = ni el techo llega.
    pub contribution_required_monthly: Option<Decimal>,
    /// El techo que la búsqueda estuvo dispuesta a explorar. No es «lo que el hogar puede
    /// aportar»: es la cota de la bisección, y es lo que da sentido al infra-financiado.
    pub contribution_required_search_ceiling: Option<Decimal>,
    /// `true` ⟺ ni el techo cumple el umbral. `None` donde la pregunta no se hizo.
    pub contribution_underfunded: Option<bool>,
    /// **`C`** — el primer mes en que se puede dejar de aportar (coast modo A, resuelto) o el que
    /// el usuario fijó (modo B, ecoado). `None` fuera de coast, o si no hay ninguno
    /// (`coast_not_reachable`, que viaja en [`Self::warnings`]).
    pub coast_stop_month_index: Option<u32>,
    /// **`S`** — el primer mes en que puede empezar la media jornada (modo B, resuelto) o el que
    /// el usuario fijó (modo A, ecoado).
    pub partial_start_month_index: Option<u32>,
    /// **`true` ⟺ la confirmación no cerró**: ni el mes que la búsqueda verificó ni los doce
    /// siguientes cumplieron el umbral con el presupuesto grande, y lo que se publica es el que
    /// más cerca quedó. La fecha se publica igual, porque «no lo sé» no es lo mismo que «no
    /// existe» — pero se publica DICIÉNDOLO. Tragarse este flag convertiría una fecha aproximada
    /// en una verificada.
    pub date_is_approximate: bool,
    /// Códigos de aviso, literales de contrato de cable
    /// (`futurefin_engine_stochastic::StrategySolveWarning::code`): `coast_not_reachable`,
    /// `partial_never_starts`, `partial_never_fully_retires`, `retire_at_age_underfunded`. **No se
    /// traducen aquí**: un `match` duplicado en `apps/api` se queda atrás en cuanto el enum crece.
    pub warnings: Vec<&'static str>,
    /// **El mejor par `(mes, éxito)` OBSERVADO** durante el solve. Con
    /// [`DATE_BASIS_NOT_REACHABLE`] es lo único que hay que enseñar; con fecha se publica igual y
    /// **no tiene por qué ser el mes devuelto** (la búsqueda pudo ver uno posterior con más
    /// margen).
    pub best_effort: Option<(u32, f64)>,
    /// Sorteos de búsqueda ejecutados, sumando todos los solves de este nivel.
    pub draws_search: u32,
    /// Sorteos de confirmación ejecutados, sumando todos los solves de este nivel.
    pub draws_confirm: u32,
}

/// **El nivel 1: la fecha, el éxito y el capital necesario hoy.**
///
/// Es **síncrono y CPU puro** a propósito: el llamante lo mete dentro del `heavy::run_projection_sim`
/// que ya envuelve el miss de proyección, de modo que un plan y su serie comparten un solo permiso
/// del semáforo en vez de pelearse por dos.
///
/// # El reparto por estrategia, en una tabla
///
/// | estrategia | fecha | además |
/// |---|---|---|
/// | `asap` | `valid_retirement_month(k_min)` | — |
/// | `retire_at_age` | **es dato** (`R`) | `success_at_month(R)` con presupuesto de confirmación + `minimum_extra_contribution(R)` |
/// | `coast` modo A | **es dato** (`R`) | `coast_stop_month(R)` + `success_at_month(R)` + `minimum_extra_contribution(R)` |
/// | `coast` modo B | `valid_retirement_month` sobre la entrada con `contributions_stop_month = C` | — |
/// | `partial` modo A | `valid_retirement_month` sobre la entrada con la fase desde `S`, con `k_min = max(k_min, S+1)` | — |
/// | `partial` modo B | la fecha ANIDADA de `earliest_partial_start` | el `S` resuelto |
/// | **todas** | | `needed_capital_today(umbral)` |
///
/// # Por qué `retire_at_age` **no bisecciona**
///
/// Porque no hay nada que buscar: la fecha la puso el usuario. Lo que se mide es si se llega
/// (`success_at_month(R)` con el presupuesto grande, un solo sorteo) y cuánto falta aportar si no.
/// Biseccionar ahí gastaría veinte sorteos para devolver el número que ya se tenía.
///
/// # Sin fecha
///
/// [`PlanLevel1::forced_month`] es `None`, la base es [`DATE_BASIS_NOT_REACHABLE`] y las cifras de
/// éxito describen la **mejor observación** del solve, no una fecha. El llamante simula entonces la
/// línea sin jubilación.
pub(crate) fn solve_plan_level1(
    input: &ProjectionInput,
    vols: &[Option<f64>],
    seed: u64,
    profile: &PlanSolveProfile,
) -> Result<PlanLevel1, ApiError> {
    solve_plan_level1_with_budget(input, vols, seed, profile, PlanBudget::FULL)
}

/// [`solve_plan_level1`] con el **presupuesto elegido por el llamante**.
///
/// Único usuario hoy: `simulate_projection` (WP A8), que resuelve DOS planes por llamada y por
/// defecto los mide a los dos con [`PlanBudget::SEARCH_ONLY`]. El presupuesto viaja de vuelta en
/// [`PlanLevel1::paths_used`]: lo que se publica dice con cuántos caminos se midió.
pub(crate) fn solve_plan_level1_with_budget(
    input: &ProjectionInput,
    vols: &[Option<f64>],
    seed: u64,
    profile: &PlanSolveProfile,
    budget: PlanBudget,
) -> Result<PlanLevel1, ApiError> {
    solve_plan_level1_inner(input, vols, seed, profile, budget).map_err(map_mc_err)
}

fn solve_plan_level1_inner(
    input: &ProjectionInput,
    vols: &[Option<f64>],
    seed: u64,
    profile: &PlanSolveProfile,
    budget: PlanBudget,
) -> Result<PlanLevel1, McError> {
    debug_assert!(
        budget.confirm_paths >= budget.search_paths,
        "confirmar con menos muestra que la búsqueda desmiente una medición con otra peor"
    );
    let search = mc(seed, budget.search_paths);
    let confirm = mc(seed, budget.confirm_paths);
    let threshold = profile.threshold_pct;
    let k_min = profile.bridge_k_min();

    let mut out = PlanLevel1 {
        retirement_date_basis: DATE_BASIS_NOT_REACHABLE,
        forced_month: None,
        success_of_plan: 0.0,
        success_wilson_low: 0.0,
        success_sampling_error_pp: Decimal::ZERO,
        paths_used: budget.confirm_paths,
        seed,
        needed_capital_today: None,
        needed_capital_absent_reason: None,
        contribution_required_monthly: None,
        contribution_required_search_ceiling: None,
        contribution_underfunded: None,
        coast_stop_month_index: None,
        partial_start_month_index: None,
        date_is_approximate: false,
        warnings: Vec::new(),
        best_effort: None,
        draws_search: 0,
        draws_confirm: 0,
    };

    // La estrategia por EDAD que se quedó sin `R` (sin fecha de nacimiento, o sin edad objetivo)
    // degrada a `asap`: el ensamblado ya emitió su aviso (`birth_date_missing` /
    // `target_retirement_age_missing`) y aquí lo que toca es contestar, no reventar una lectura.
    let by_age = matches!(profile.strategy, PlanStrategy::RetireAtAge)
        || (matches!(profile.strategy, PlanStrategy::Coast)
            && profile.coast_mode == CoastSolveMode::FixedRetirementAge);
    let target = profile.target_month.filter(|_| by_age);

    match (profile.strategy, target) {
        // -----------------------------------------------------------------------------------
        // La fecha es un DATO: `retire_at_age` y `coast` modo A.
        // -----------------------------------------------------------------------------------
        (PlanStrategy::RetireAtAge, Some(r)) | (PlanStrategy::Coast, Some(r)) => {
            out.retirement_date_basis = DATE_BASIS_TARGET_AGE;
            out.forced_month = Some(r);

            if matches!(profile.strategy, PlanStrategy::Coast) {
                let coast = coast_stop_month(input, vols, &search, &confirm, threshold, r)?;
                out.draws_search += coast.draws_search;
                out.draws_confirm += coast.draws_confirm;
                out.coast_stop_month_index = coast.stop_month;
                out.warnings.extend(coast.warnings.iter().map(|w| w.code()));
            }

            // Un solo sorteo, con el presupuesto grande: no hay nada que buscar.
            let s = success_at_month(input, vols, &confirm, r)?;
            out.draws_confirm += 1;
            apply_success(&mut out, &s);
            out.best_effort = Some((r, s.success));

            let c = minimum_extra_contribution(input, vols, &search, &confirm, threshold, r)?;
            out.draws_search += c.draws_search;
            out.draws_confirm += c.draws_confirm;
            out.contribution_required_monthly = c.extra_monthly;
            out.contribution_required_search_ceiling = Some(c.search_ceiling);
            out.contribution_underfunded = Some(c.underfunded);
            out.warnings.extend(c.warning().map(|w| w.code()));
        }

        // -----------------------------------------------------------------------------------
        // `partial` modo B: se resuelve S y, con la fase dentro, la fecha total.
        // -----------------------------------------------------------------------------------
        (PlanStrategy::Partial, _) if profile.partial_mode == PartialSolveMode::Asap => {
            let p = earliest_partial_start(input, vols, &search, &confirm, threshold)?;
            out.draws_search += p.draws_search;
            out.draws_confirm += p.draws_confirm;
            out.partial_start_month_index = p.start_month;
            out.warnings.extend(p.warnings.iter().map(|w| w.code()));
            match &p.full_retirement {
                Some(date) => {
                    out.draws_search += date.draws_search;
                    out.draws_confirm += date.draws_confirm;
                    apply_date(&mut out, date, budget);
                }
                // Sin fase declarada no hay `S` y no hay fecha anidada: la única lectura honesta
                // es la de la fase, y `not_reachable` con su `best_effort` vacío.
                None => {
                    if let Some(s) = p.phase_success {
                        apply_success(&mut out, &s);
                        out.paths_used = s.paths;
                        out.best_effort = Some((s.month, s.success));
                    }
                }
            }
        }

        // -----------------------------------------------------------------------------------
        // El resto resuelve la FECHA sobre un escenario: `asap`, `coast` modo B, `partial` modo A
        // (y las estrategias por edad que se quedaron sin `R`, degradadas a `asap`).
        // -----------------------------------------------------------------------------------
        _ => {
            let (scenario, floor) = date_scenario(input, profile, k_min);
            let date = valid_retirement_month(&scenario, vols, &search, &confirm, threshold, floor)?;
            out.draws_search += date.draws_search;
            out.draws_confirm += date.draws_confirm;
            apply_date(&mut out, &date, budget);
            // Los dos ejes que en estas ramas son DATO se ecoan para que la respuesta no obligue
            // a nadie a mirar el perfil para saber qué se simuló.
            if matches!(profile.strategy, PlanStrategy::Coast) {
                out.coast_stop_month_index = profile.coast_stop_month;
            }
            if matches!(profile.strategy, PlanStrategy::Partial) {
                out.partial_start_month_index = profile.partial_start_month;
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // El capital necesario HOY: SIEMPRE, en las seis ramas. Es la única cifra del plan que no
    // depende de la estrategia — «cuánto necesitarías para jubilarte ya» se contesta igual.
    // ---------------------------------------------------------------------------------------
    let needed = needed_capital_today(input, vols, &search, &confirm, threshold)?;
    out.draws_search += needed.draws_search;
    out.draws_confirm += needed.draws_confirm;
    apply_needed_capital(&mut out, &needed);

    Ok(out)
}

/// El escenario sobre el que se resuelve la FECHA, y el suelo de la búsqueda.
///
/// Dos mutaciones posibles y ninguna más — las dos con la plantilla PÚBLICA del crate, nunca
/// reescritas aquí:
///
/// - `coast` modo B: `contributions_stop_month = C` (`stopping_at`).
/// - `partial` modo A: la fase empieza en `S` (`partial_starting_at`) y el suelo sube a `S + 1`,
///   porque jubilarse del todo antes de empezar la media jornada no es el plan que se pidió.
fn date_scenario(
    input: &ProjectionInput,
    profile: &PlanSolveProfile,
    k_min: u32,
) -> (ProjectionInput, u32) {
    match profile.strategy {
        PlanStrategy::Coast if profile.coast_mode == CoastSolveMode::FixedStopAge => {
            match profile.coast_stop_month {
                Some(c) => (stopping_at(input, c), k_min),
                None => (input.clone(), k_min),
            }
        }
        PlanStrategy::Partial => match profile.partial_start_month {
            Some(s) => (
                partial_starting_at(input, s),
                k_min.max(s.saturating_add(1)),
            ),
            None => (input.clone(), k_min),
        },
        _ => (input.clone(), k_min),
    }
}

/// Copia una medición de éxito a la salida. Un solo sitio para que las cuatro cifras
/// (`success`, `wilson_low`, la barra y el `N`) no puedan salir de mediciones distintas.
fn apply_success(out: &mut PlanLevel1, s: &SuccessAt) {
    out.success_of_plan = s.success;
    out.success_wilson_low = s.wilson_low;
    out.success_sampling_error_pp = pp_out(s.half_width_pp);
    out.paths_used = s.paths;
}

/// Copia un `RetirementDateSolve` a la salida, con su base y su honestidad.
fn apply_date(out: &mut PlanLevel1, date: &RetirementDateSolve, budget: PlanBudget) {
    out.forced_month = date.month;
    out.retirement_date_basis = if date.month.is_some() {
        DATE_BASIS_SUCCESS_THRESHOLD
    } else {
        DATE_BASIS_NOT_REACHABLE
    };
    out.success_of_plan = date.success;
    out.success_wilson_low = date.wilson_low;
    out.success_sampling_error_pp = pp_out(date.half_width_pp);
    // El mes devuelto se midió con el presupuesto de CONFIRMACIÓN cuando hubo confirmación; sin
    // fecha, las cifras describen la mejor observación de la BÚSQUEDA. Publicar 2.500 ahí sería
    // atribuirle a una medición un tamaño de muestra que no tuvo.
    out.paths_used = if date.draws_confirm > 0 {
        budget.confirm_paths
    } else {
        budget.search_paths
    };
    out.date_is_approximate = date.date_is_approximate;
    out.best_effort = date.best_effort;
}

/// Copia el capital necesario. **Nunca fabrica un 0 €**: sin importe viaja la razón.
fn apply_needed_capital(out: &mut PlanLevel1, needed: &NeededCapital) {
    out.needed_capital_today = needed.amount_today;
    out.needed_capital_absent_reason = needed.absent_reason;
}

/// La configuración de un presupuesto. Un solo percentil porque **este módulo no dibuja bandas**:
/// los percentiles no tocan el conteo de fallos y pedir tres reservaría tres vectores del
/// horizonte para tirarlos.
fn mc(seed: u64, paths: u32) -> McConfig {
    McConfig {
        seed,
        paths,
        percentiles: vec![50],
    }
}

/// La barra de error en puntos porcentuales, llevada al `Decimal` que la API publica como string.
///
/// `from_f64_retain` no puede fallar aquí: `half_width_pp` sale de Wilson sobre una muestra finita
/// y vive en `[0, 100]`. La rama imposible publica `Decimal::ZERO` y **deja rastro en el log**,
/// porque un 0 ahí se leería como «medición sin error», que es exactamente lo contrario de lo que
/// este número existe para decir.
fn pp_out(half_width_pp: f64) -> Decimal {
    match Decimal::from_f64(half_width_pp) {
        Some(d) => d.round_dp(SAMPLING_ERROR_DP),
        None => {
            debug_assert!(false, "half_width_pp no representable: {half_width_pp}");
            tracing::error!(
                half_width_pp,
                "la barra de error de Wilson no cabe en Decimal — bug del crate estocástico"
            );
            Decimal::ZERO
        }
    }
}

// =================================================================================================
// El escenario del plan
// =================================================================================================

/// **La entrada que el plan describe, construida en UN sitio.**
///
/// Aplica sobre la entrada del ensamblado las tres decisiones del nivel 1 —el mes de corte de
/// aportaciones, el inicio de la media jornada y el mes forzado de jubilación— usando las
/// plantillas públicas del crate (`stopping_at`, `partial_starting_at`, `retiring_at`), nunca
/// mutaciones reescritas aquí.
///
/// **Sin fecha** (`forced_month: None`) el escenario se jubila en `horizonte + 1`: «no se jubila
/// dentro del horizonte», que es la misma convención con la que `earliest_partial_start` aísla su
/// fase. Es lo que hace que el ancla de `cumulative_failure_by_age` esté definida también ahí; el
/// llamante publica esa misma línea como «sin jubilación».
///
/// Es la entrada que hay que pasar a [`plan_fingerprint`] **y** a [`spawn_plan_extras`]: si las dos
/// no fueran el mismo valor, la clave dejaría de describir lo que hay dentro.
pub(crate) fn plan_scenario(input: &ProjectionInput, level1: &PlanLevel1) -> ProjectionInput {
    let mut scenario = input.clone();
    if let Some(c) = level1.coast_stop_month_index {
        scenario = stopping_at(&scenario, c);
    }
    if let Some(s) = level1.partial_start_month_index {
        scenario = partial_starting_at(&scenario, s);
    }
    let month = level1
        .forced_month
        .unwrap_or_else(|| scenario.horizon_months.saturating_add(1));
    retiring_at(&scenario, month)
}

// =================================================================================================
// Nivel 2
// =================================================================================================

/// **Lo que ilustra el plan pero no lo decide.** Se calcula en segundo plano y se publica entero o
/// no se publica: no hay resultados parciales, porque una curva a medias dibujada junto a una
/// completa es peor que un hueco declarado.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanExtras {
    /// [`PLAN_EXTRAS_READY`] o una de las razones de fallo. El llamante traduce cualquier fallo a
    /// `unavailable`; la razón es para el log y para poder responder «¿por qué falta?».
    pub state: &'static str,
    /// Mes del bucle en que el plan alcanzaría el **100 %** de éxito, medido con presupuesto de
    /// BÚSQUEDA. `None` = no lo alcanza dentro del horizonte.
    pub safe_date_at_100: Option<u32>,
    /// Ídem al **90 %**. Por construcción del umbral (`meets` es monótono en él) esta fecha no
    /// puede ser posterior a la del 100 % salvo por los avances de confirmación.
    pub safe_date_at_90: Option<u32>,
    /// **La curva de capital necesario por edad**: `(mes del bucle, importe en euros NOMINALES de
    /// ese mes)`, ya redondeado a cientos hacia arriba por el crate
    /// (`NeededCapital::amount_nominal`).
    ///
    /// **Qué mide un nodo (C9, 2026-09-07): «lo que necesitas TENER (líquido) a esa edad para
    /// jubilarte entonces al umbral».** La acumulación hasta el mes anterior no se sortea —es la
    /// línea determinista—, así que todos los caminos llegan al nodo con esta misma cifra y lo que
    /// se sortea es solo lo que viene DESPUÉS de jubilarse. No es «el percentil de tu cartera de
    /// hoy proyectada hasta esa edad», que es lo que la curva publicaba antes y lo que la hacía
    /// crecer sin techo con el horizonte.
    ///
    /// **Nominal, no deflactado, y es deliberado**: la curva se dibuja contra la trayectoria del
    /// patrimonio, que es nominal, así que un nodo en euros de hoy se compararía con la línea en el
    /// mes equivocado. Quien la quiera «en dinero de hoy» deflacta la curva y la línea **a la vez y
    /// con el mismo factor**, que es lo que hace la SPA. Ojo con la asimetría respecto a
    /// [`PlanLevel1::needed_capital_today`], que va en euros de HOY: en `k = 1` las dos bases
    /// coinciden exactamente (el factor en el índice 0 es 1) y a partir de ahí divergen.
    ///
    /// **No tiene por qué CRUZAR la línea del patrimonio en la fecha del plan**: la fecha la decide
    /// el éxito sobre miles de caminos, cada uno con su propia acumulación (definición A), y esta
    /// curva contesta la otra pregunta. Son dos magnitudes, no dos vistas de una.
    ///
    /// Los nodos sin cifra **no aparecen aquí** —un 0 € diría lo contrario de lo que pasa— y su
    /// razón viaja en [`Self::needed_capital_curve_absent`].
    pub needed_capital_curve: Vec<(u32, Decimal)>,
    /// **Los nodos que no tienen importe, con su razón**: `(mes del bucle, `no_liquid_assets` |
    /// `threshold_unreachable` | `month_beyond_horizon` | `already_covered`)`, literales del crate.
    ///
    /// **`already_covered` es el motivo esperado a partir de la fecha del plan**: cuando la pensión
    /// ya cubre el gasto, `λ` deja de morder y no hay frontera que biseccionar. Los nodos que hasta
    /// la corrección de este bug publicaban ahí una curva CRECIENTE («a los 86 necesitas 2,9 M€»)
    /// no medían una necesidad: publicaban el líquido de un hogar escalado a casi cero, es decir lo
    /// que el hogar acumula de su nómina hasta esa edad.
    ///
    /// Existe para que quien dibuja pueda **partir la línea** en ese mes en vez de unir los dos
    /// nodos vecinos por encima del hueco: una curva continua sobre un tramo que no se pudo medir
    /// es una interpolación disfrazada de medición. Y para poder contestar «¿por qué falta este
    /// punto?» con la razón en vez de con un encogimiento de hombros.
    ///
    /// Es **paralelo y disjunto** de [`Self::needed_capital_curve`]: la unión de los dos, ordenada
    /// por mes, es la rejilla que se evaluó.
    pub needed_capital_curve_absent: Vec<(u32, &'static str)>,
    /// **La tira bajo el eje**: `(mes del bucle, éxito)` jubilándose en cada aniversario.
    pub success_by_retirement_year: Vec<(u32, f64)>,
    /// **El fallo acumulado por edad**: `(mes del bucle, fracción de caminos con ALGÚN fallo hasta
    /// ese mes, fallos por motivo)`.
    ///
    /// El tercer elemento es el reparto `[F1 cartera agotada, F2 tasa inicial excedida, F3 la
    /// regla no llega a la necesidad]` **de la ejecución entera**, no del tramo: `McOutcome`
    /// clasifica el PRIMER fallo de cada camino sobre todo el horizonte y no lo desglosa por mes.
    /// Es exacto en la ÚLTIMA fila —que cierra en `1 − éxito`, el mismo conjunto de caminos— y en
    /// las anteriores es la composición del horizonte completo, no la del tramo. Inventarse aquí
    /// un reparto por tramo sería una segunda implementación de una clasificación que el motor
    /// posee: si hace falta, se pide en `crates/engine-stochastic`.
    pub failure_probability_by_age: Vec<(u32, f64, [u32; 3])>,
}

impl PlanExtras {
    /// El resultado vacío con su razón. **Nunca se publica un `ready` con vectores vacíos**: un
    /// hueco declarado y una curva de cero puntos no significan lo mismo.
    fn failed(state: &'static str) -> Self {
        PlanExtras {
            state,
            safe_date_at_100: None,
            safe_date_at_90: None,
            needed_capital_curve: Vec::new(),
            needed_capital_curve_absent: Vec::new(),
            success_by_retirement_year: Vec::new(),
            failure_probability_by_age: Vec::new(),
        }
    }
}

/// **Lanza el nivel 2, una sola vez por clave.**
///
/// # Por qué es `async` cuando solo lanza una tarea
///
/// Porque el `Pending` tiene que estar en la cache **antes de que el llamante responda**. Si el
/// marcado ocurriera dentro de la tarea lanzada, un lector que llegara en medio no vería nada y
/// publicaría `unavailable` («no se puede») donde la verdad es `computing` («se está calculando»).
/// Es exactamente el razonamiento por el que `refresh_projection_after_mutation` dejó de ser un
/// `tokio::spawn` y pasó a esperarse: el efecto observable tiene que ser final cuando la respuesta
/// sale. Lo que sí va en la tarea es el trabajo, que dura decenas de segundos.
///
/// # Deduplicación
///
/// Dos peticiones concurrentes del mismo hogar producen la MISMA [`PlanKey`]. La segunda ve la
/// clave en [`AppState::plan_inflight`] y vuelve sin lanzar nada: el nivel 2 —y con él la fecha al
/// 100 %, la del 90 % y la curva— se resuelve **una vez**. Si ya hay extras terminados para esa
/// clave, tampoco se relanza: se refresca su TTL y se vuelve.
///
/// # Un solo permiso
///
/// Los cinco cómputos van en **una** llamada a `heavy::run_projection_sim`. Pedir cinco permisos
/// dejaría al nivel 2 compitiendo consigo mismo por el semáforo que existe para que `/v1/ready`
/// siga respondiendo.
pub(crate) async fn spawn_plan_extras(
    state: Arc<AppState>,
    key: PlanKey,
    input: ProjectionInput,
    vols: Vec<Option<f64>>,
    seed: u64,
    profile: PlanSolveProfile,
    forced_month: Option<u32>,
) {
    debug_assert_eq!(
        plan_fingerprint(&input, &vols, profile.threshold_pct, SOLVE_CONFIRM_PATHS, seed),
        key,
        "la clave y el escenario tienen que salir del MISMO `plan_scenario`"
    );

    // Ya está resuelto: refrescar el TTL y volver. `touch` no reconstruye nada.
    if state.plan_cache_touch(&key).await {
        return;
    }
    {
        let mut inflight = state.plan_inflight.lock().await;
        if !inflight.insert(key) {
            return;
        }
    }
    state.plan_cache_insert(key, PlanCacheSlot::Pending).await;

    tokio::spawn(async move {
        let t0 = std::time::Instant::now();
        tracing::info!(plan_key = key.as_u64(), "plan extras start");
        let threshold = profile.threshold_pct;
        let k_min = profile.bridge_k_min();
        let extras = crate::heavy::run_projection_sim("plan extras", move || {
            compute_plan_extras(&input, &vols, seed, threshold, k_min, forced_month)
        })
        .await;

        let extras = match extras {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => {
                tracing::warn!(plan_key = key.as_u64(), error = %e, "plan extras failed");
                PlanExtras::failed(match e {
                    McError::Engine(_) => PLAN_EXTRAS_FAILED_ENGINE,
                    _ => PLAN_EXTRAS_FAILED_CONFIG,
                })
            }
            Err(e) => {
                tracing::warn!(plan_key = key.as_u64(), error = ?e, "plan extras task failed");
                PlanExtras::failed(PLAN_EXTRAS_FAILED_TASK)
            }
        };
        tracing::info!(
            plan_key = key.as_u64(),
            state = extras.state,
            ms = t0.elapsed().as_millis() as u64,
            "plan extras done"
        );
        state
            .plan_cache_insert(key, PlanCacheSlot::Done(Arc::new(extras)))
            .await;
        state.plan_inflight.lock().await.remove(&key);
    });
}

/// Los cinco cómputos del nivel 2, en orden de coste creciente. Síncrono y CPU puro: corre entero
/// bajo un permiso de `heavy::run_projection_sim`.
fn compute_plan_extras(
    input: &ProjectionInput,
    vols: &[Option<f64>],
    seed: u64,
    threshold_pct: u32,
    k_min: u32,
    forced_month: Option<u32>,
) -> Result<PlanExtras, McError> {
    let search = mc(seed, SOLVE_SEARCH_PATHS);
    let curve_cfg = mc(seed, CURVE_PATHS);
    let confirm = mc(seed, SOLVE_CONFIRM_PATHS);

    // (1)(2) Las dos fechas de referencia, con presupuesto de BÚSQUEDA en las dos fases.
    //
    // Pasar `search` también como `confirm` no es un descuido: son la MISMA muestra, así que la
    // fase de confirmación vuelve a medir lo que ya midió y cierra en el acto. Estas dos fechas
    // son ORIENTACIÓN («¿y si quisiera dormir del todo tranquilo?»); la única confirmada con el
    // presupuesto grande es la del plan, que es la que se publica como fecha.
    let at_100 = valid_retirement_month(input, vols, &search, &search, 100, k_min)?.month;
    let at_90 = valid_retirement_month(input, vols, &search, &search, 90, k_min)?.month;

    // (3) La curva de capital: cada cinco años y, además, el mes del plan — el nodo que la SPA
    // necesita para tener una MEDICIÓN en la fecha. No para forzar un cruce ahí: la fecha la
    // decide el éxito sobre miles de caminos, no un cruce de esta curva con el patrimonio (el doc
    // de `PlanExtras::needed_capital_curve` y `.claude/api-routes.md` §serie lo dicen igual).
    //
    // Los importes son **NOMINALES** (`amount_nominal`), no deflactados: la curva se dibuja contra
    // la trayectoria del patrimonio, que es nominal. Ver el doc de `PlanExtras`. Y desde C9 cada
    // nodo es «lo que hay que TENER a esa edad», condicionado a llegar por la línea determinista.
    let nodes = needed_capital_curve(
        input,
        vols,
        &curve_cfg,
        threshold_pct,
        &curve_grid(input.horizon_months, forced_month),
    )?;
    let mut curve = Vec::with_capacity(nodes.len());
    let mut curve_absent = Vec::new();
    for n in nodes {
        // Un nodo sin cifra NO se publica como `(mes, 0 €)` —eso diría «no necesitas nada» donde
        // la verdad es «este método no puede medirlo»— y tampoco se calla: su razón va al vector
        // hermano para que la línea se pueda partir ahí.
        match n.amount_nominal {
            Some(a) => curve.push((n.month, a)),
            None => curve_absent.push((
                n.month,
                // `NeededCapital` garantiza `amount_nominal.is_none() ⟺ absent_reason.is_some()`;
                // el `unwrap_or` es la red por si esa invariante se rompiera algún día — y dice
                // «no se sabe», no un motivo inventado.
                n.absent_reason.unwrap_or(CURVE_NODE_ABSENT_UNKNOWN),
            )),
        }
    }

    // (4) La tira anual.
    let yearly = success_by_retirement_month(
        input,
        vols,
        &curve_cfg,
        &yearly_grid(input.horizon_months, k_min),
    )?
    .into_iter()
    .map(|s| (s.month, s.success))
    .collect();

    // (5) El fallo acumulado por edad, de la MISMA muestra que publica el fan chart (misma
    // semilla, mismos caminos: `SOLVE_CONFIRM_PATHS` ES `DEFAULT_BANDS_PATHS`). La lista de
    // percentiles no toca el conteo de fallos, así que pedir uno solo no cambia estas cifras.
    let bands = project_percentile_bands(input, vols, &confirm)?;
    let by_kind = bands.failures_by_kind;
    let failure = bands
        .cumulative_failure_by_age
        .into_iter()
        .map(|(m, p)| (m, p, by_kind))
        .collect();

    Ok(PlanExtras {
        state: PLAN_EXTRAS_READY,
        safe_date_at_100: at_100,
        safe_date_at_90: at_90,
        needed_capital_curve: curve,
        needed_capital_curve_absent: curve_absent,
        success_by_retirement_year: yearly,
        failure_probability_by_age: failure,
    })
}

/// La rejilla de la curva: `{1, 1+`[`CURVE_GRID_MONTHS`]`, …} ∪ {mes del plan}`, dentro del
/// horizonte, ordenada y sin repetidos.
///
/// El nodo del mes del plan no es decorativo: sin él la curva se evaluaría en los múltiplos de
/// cinco años y **nunca en la fecha**, así que el punto donde la curva cruza la trayectoria —el
/// único que responde «¿por qué esa fecha?»— sería una interpolación entre dos nodos, no una
/// medición.
fn curve_grid(horizon_months: u32, forced_month: Option<u32>) -> Vec<u32> {
    let mut grid: Vec<u32> = (0..)
        .map(|i| 1 + i * CURVE_GRID_MONTHS)
        .take_while(|m| *m <= horizon_months)
        .collect();
    if let Some(m) = forced_month.filter(|m| (1..=horizon_months).contains(m)) {
        grid.push(m);
    }
    grid.sort_unstable();
    grid.dedup();
    grid
}

/// La rejilla anual: aniversarios de `k_min`, dentro del horizonte y con tope
/// [`YEARLY_GRID_MAX_NODES`].
///
/// Ancla en `k_min` y no en el mes 1 porque los meses por debajo del suelo del plan no son fechas
/// de jubilación candidatas —con puente, el motor las tumbaría por tasa inicial— y sondearlos sería
/// gastar sorteos en preguntas ya contestadas.
fn yearly_grid(horizon_months: u32, k_min: u32) -> Vec<u32> {
    let start = k_min.max(1);
    (0..)
        .map(|i| start + i * YEARLY_GRID_STEP_MONTHS)
        .take_while(|m| *m <= horizon_months)
        .take(YEARLY_GRID_MAX_NODES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use futurefin_engine::{PhasePlan, SimAsset};
    use uuid::Uuid;

    /// Un hogar mínimo, suficiente para que el motor corra y para que las huellas sean
    /// distinguibles. Doce meses: los sorteos de estos tests cuestan milisegundos, no segundos.
    fn tiny_input(liquid: i64) -> ProjectionInput {
        ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 1).expect("fecha válida"),
            horizon_months: 12,
            annual_inflation_percent: Decimal::ZERO,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ZERO,
            income_regular_monthly: Decimal::from(2_000),
            expense_regular_monthly: Decimal::from(1_500),
            assets: vec![SimAsset {
                id: Uuid::nil(),
                value: Decimal::from(liquid),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            allocation_rules: Vec::new(),
            liabilities: Vec::new(),
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 12],
            phase_plan: PhasePlan::classic(Decimal::ZERO, Decimal::from(1_500)),
            fire_target: None,
        }
    }

    fn tiny_profile(strategy: PlanStrategy) -> PlanSolveProfile {
        PlanSolveProfile {
            strategy,
            threshold_pct: 95,
            target_month: Some(6),
            coast_mode: CoastSolveMode::FixedRetirementAge,
            coast_stop_month: None,
            partial_mode: PartialSolveMode::AtAge,
            partial_start_month: None,
            bridge: None,
        }
    }

    // ---------------------------------------------------------------------------------------
    // Presupuestos
    // ---------------------------------------------------------------------------------------

    /// **La cota de muestra de Wilson, aplicada a los presupuestos de este módulo.**
    ///
    /// Con cero fallos la cota inferior colapsa a `n/(n + z²)`, así que un umbral `u < 100` es
    /// inalcanzable con `n < z²·u/(1−u)`: **381 para el 99 %**, que es el mayor umbral por debajo
    /// de 100 que el perfil admite. Los dos presupuestos tienen que estar por encima, o habría
    /// umbrales configurables que ninguna muestra limpia podría satisfacer.
    #[test]
    fn the_budgets_can_reach_the_highest_profile_threshold() {
        const N_FOR_99: u32 = 381;
        assert!(
            SOLVE_SEARCH_PATHS >= N_FOR_99,
            "buscar con {SOLVE_SEARCH_PATHS} caminos deja el 99 % inalcanzable"
        );
        assert!(
            SOLVE_CONFIRM_PATHS >= N_FOR_99,
            "confirmar con {SOLVE_CONFIRM_PATHS} caminos deja el 99 % inalcanzable"
        );
        assert!(
            SOLVE_CONFIRM_PATHS >= SOLVE_SEARCH_PATHS,
            "la confirmación tiene que AMPLIAR la muestra de la búsqueda, no reducirla"
        );
    }

    /// El presupuesto de confirmación **es** el de las bandas: la fecha y el fan chart citan la
    /// misma ejecución de Monte Carlo, no dos. Si alguien escribe un literal aquí, este test
    /// deja de ser trivial y empieza a fallar.
    #[test]
    fn the_confirmation_budget_is_the_bands_budget() {
        assert_eq!(SOLVE_CONFIRM_PATHS, DEFAULT_BANDS_PATHS);
    }

    // ---------------------------------------------------------------------------------------
    // La huella
    // ---------------------------------------------------------------------------------------

    /// Mismo contenido ⇒ misma clave. Es la propiedad que hace que dos peticiones del mismo hogar
    /// compartan el nivel 2.
    #[test]
    fn the_same_content_gives_the_same_key() {
        let a = tiny_input(50_000);
        let b = tiny_input(50_000);
        let vols = vec![Some(15.0)];
        assert_eq!(
            plan_fingerprint(&a, &vols, 95, SOLVE_CONFIRM_PATHS, 7),
            plan_fingerprint(&b, &vols, 95, SOLVE_CONFIRM_PATHS, 7)
        );
    }

    /// **Cada eje mueve la clave.** Umbral, caminos, semilla, volatilidades y entrada: si alguno
    /// no entrara en la huella, la cache serviría el plan de otra pregunta.
    #[test]
    fn every_axis_of_the_question_moves_the_key() {
        let input = tiny_input(50_000);
        let vols = vec![Some(15.0)];
        let base = plan_fingerprint(&input, &vols, 95, SOLVE_CONFIRM_PATHS, 7);

        assert_ne!(base, plan_fingerprint(&input, &vols, 96, SOLVE_CONFIRM_PATHS, 7), "umbral");
        assert_ne!(base, plan_fingerprint(&input, &vols, 95, SOLVE_SEARCH_PATHS, 7), "caminos");
        assert_ne!(base, plan_fingerprint(&input, &vols, 95, SOLVE_CONFIRM_PATHS, 8), "semilla");
        assert_ne!(
            base,
            plan_fingerprint(&input, &[Some(16.0)], 95, SOLVE_CONFIRM_PATHS, 7),
            "volatilidad"
        );
        assert_ne!(
            base,
            plan_fingerprint(&input, &[None], 95, SOLVE_CONFIRM_PATHS, 7),
            "sin volatilidad no es lo mismo que con volatilidad"
        );
        assert_ne!(
            base,
            plan_fingerprint(&tiny_input(50_001), &vols, 95, SOLVE_CONFIRM_PATHS, 7),
            "un euro más en la cartera es otro hogar"
        );
    }

    /// La longitud del vector de volatilidades entra en la huella: sin ella, dos hogares con
    /// distinto número de activos podrían colisionar por prefijo.
    #[test]
    fn the_volatility_vector_length_is_part_of_the_key() {
        let input = tiny_input(50_000);
        assert_ne!(
            plan_fingerprint(&input, &[Some(15.0)], 95, 500, 7),
            plan_fingerprint(&input, &[Some(15.0), None], 95, 500, 7)
        );
    }

    // ---------------------------------------------------------------------------------------
    // El reparto por estrategia
    // ---------------------------------------------------------------------------------------

    /// **`asap` bisecciona la fecha; `retire_at_age` no.** El eco que lo demuestra son los
    /// contadores de sorteos: la fecha por umbral gasta varios sorteos de búsqueda, la fecha por
    /// edad no gasta ninguno para decidir el mes (los que gasta son de la aportación mínima y del
    /// capital necesario, que son otras preguntas).
    #[test]
    fn retire_at_age_does_not_bisect_the_date_it_only_measures_it() {
        let input = tiny_input(400_000);
        let vols = vec![None];
        let p = tiny_profile(PlanStrategy::RetireAtAge);
        let out = solve_plan_level1(&input, &vols, 7, &p).expect("el solve corre");
        assert_eq!(out.retirement_date_basis, DATE_BASIS_TARGET_AGE);
        assert_eq!(out.forced_month, Some(6), "la fecha es el dato del perfil");
        assert!(
            out.contribution_underfunded.is_some(),
            "con fecha dada se contesta SIEMPRE cuánto falta aportar"
        );
    }

    /// `asap` decide la fecha por el umbral, y su base lo dice.
    #[test]
    fn asap_lets_the_threshold_decide_the_date() {
        let input = tiny_input(400_000);
        let vols = vec![None];
        let p = tiny_profile(PlanStrategy::Asap);
        let out = solve_plan_level1(&input, &vols, 7, &p).expect("el solve corre");
        assert!(
            out.retirement_date_basis == DATE_BASIS_SUCCESS_THRESHOLD
                || out.retirement_date_basis == DATE_BASIS_NOT_REACHABLE,
            "base inesperada: {}",
            out.retirement_date_basis
        );
        assert!(out.draws_search > 0, "la fecha por umbral se BUSCA");
        assert!(
            out.contribution_required_monthly.is_none(),
            "sin fecha dada no hay «cuánto me falta aportar para esa fecha»"
        );
    }

    /// **Sin fecha, `forced_month` es `None` y no un 0.** Un hogar sin un euro no puede jubilarse
    /// en ningún mes del horizonte, y la respuesta honesta es la ausencia con su `best_effort`.
    #[test]
    fn a_plan_with_no_reachable_date_is_none_not_zero() {
        let input = tiny_input(0);
        let vols = vec![None];
        let p = tiny_profile(PlanStrategy::Asap);
        let out = solve_plan_level1(&input, &vols, 7, &p).expect("el solve corre");
        assert_eq!(out.retirement_date_basis, DATE_BASIS_NOT_REACHABLE);
        assert_eq!(out.forced_month, None);
        assert_ne!(out.forced_month, Some(0), "un 0 se leería como «ya puedes»");
    }

    /// **Sin activos líquidos no se publica «0 €», se publica por qué.**
    #[test]
    fn a_household_with_no_liquid_assets_says_why_instead_of_publishing_zero() {
        let input = tiny_input(0);
        let vols = vec![None];
        let p = tiny_profile(PlanStrategy::Asap);
        let out = solve_plan_level1(&input, &vols, 7, &p).expect("el solve corre");
        assert_eq!(out.needed_capital_today, None);
        assert_eq!(
            out.needed_capital_absent_reason,
            Some(futurefin_engine_stochastic::ABSENT_NO_LIQUID_ASSETS)
        );
    }

    /// **La barra de error nunca es 0**, tampoco con cero fallos: es el sentido entero del
    /// intervalo de Wilson frente a la aproximación normal.
    #[test]
    fn the_sampling_error_is_never_zero() {
        let input = tiny_input(1_000_000);
        let vols = vec![None];
        let p = tiny_profile(PlanStrategy::Asap);
        let out = solve_plan_level1(&input, &vols, 7, &p).expect("el solve corre");
        assert!(
            out.success_sampling_error_pp > Decimal::ZERO,
            "barra = {}",
            out.success_sampling_error_pp
        );
        assert!(out.paths_used > 0, "una probabilidad sin su N no se puede leer");
    }

    // ---------------------------------------------------------------------------------------
    // El escenario y las rejillas
    // ---------------------------------------------------------------------------------------

    /// El escenario del plan lleva el mes forzado; sin fecha, se jubila **más allá** del
    /// horizonte, que es «no se jubila» dicho en la rejilla del motor.
    #[test]
    fn the_plan_scenario_carries_the_forced_month_or_never_retires() {
        use futurefin_engine::RetirementTrigger;
        let input = tiny_input(50_000);
        let mut level1 = PlanLevel1 {
            retirement_date_basis: DATE_BASIS_SUCCESS_THRESHOLD,
            forced_month: Some(5),
            success_of_plan: 1.0,
            success_wilson_low: 0.99,
            success_sampling_error_pp: Decimal::ONE,
            paths_used: 500,
            seed: 7,
            needed_capital_today: None,
            needed_capital_absent_reason: None,
            contribution_required_monthly: None,
            contribution_required_search_ceiling: None,
            contribution_underfunded: None,
            coast_stop_month_index: None,
            partial_start_month_index: None,
            date_is_approximate: false,
            warnings: Vec::new(),
            best_effort: None,
            draws_search: 0,
            draws_confirm: 0,
        };
        let s = plan_scenario(&input, &level1);
        assert_eq!(s.phase_plan.retirement_trigger, RetirementTrigger::AtMonth(5));
        assert!(s.phase_plan.crossing_is_reading_only, "el cruce no puede adelantar la fecha");

        level1.forced_month = None;
        let s = plan_scenario(&input, &level1);
        assert_eq!(
            s.phase_plan.retirement_trigger,
            RetirementTrigger::AtMonth(input.horizon_months + 1),
            "sin fecha, el escenario no se jubila dentro del horizonte"
        );
    }

    /// El corte de aportaciones del coast entra en el escenario, así que entra en la clave.
    #[test]
    fn the_coast_cut_is_part_of_the_scenario_and_therefore_of_the_key() {
        let input = tiny_input(50_000);
        let base = PlanLevel1 {
            retirement_date_basis: DATE_BASIS_TARGET_AGE,
            forced_month: Some(10),
            success_of_plan: 1.0,
            success_wilson_low: 0.99,
            success_sampling_error_pp: Decimal::ONE,
            paths_used: 500,
            seed: 7,
            needed_capital_today: None,
            needed_capital_absent_reason: None,
            contribution_required_monthly: None,
            contribution_required_search_ceiling: None,
            contribution_underfunded: None,
            coast_stop_month_index: None,
            partial_start_month_index: None,
            date_is_approximate: false,
            warnings: Vec::new(),
            best_effort: None,
            draws_search: 0,
            draws_confirm: 0,
        };
        let with_cut = PlanLevel1 {
            coast_stop_month_index: Some(4),
            ..base.clone()
        };
        let vols = vec![None];
        assert_ne!(
            plan_fingerprint(&plan_scenario(&input, &base), &vols, 95, 500, 7),
            plan_fingerprint(&plan_scenario(&input, &with_cut), &vols, 95, 500, 7)
        );
    }

    /// La rejilla de la curva empieza en el mes 1, avanza de cinco en cinco años, **incluye el mes
    /// del plan** y no se sale del horizonte.
    #[test]
    fn the_curve_grid_includes_the_plan_month_and_stays_inside_the_horizon() {
        let g = curve_grid(840, Some(317));
        assert_eq!(g.first(), Some(&1));
        assert!(g.contains(&317), "sin el nodo de la fecha la curva no cruza en la fecha");
        assert!(g.windows(2).all(|w| w[0] < w[1]), "ordenada y sin repetidos: {g:?}");
        assert!(g.iter().all(|m| (1..=840).contains(m)));
        // Un mes del plan que ya está en la rejilla no se duplica.
        let g = curve_grid(840, Some(61));
        assert_eq!(g.iter().filter(|m| **m == 61).count(), 1);
        // Fuera del horizonte no entra.
        assert!(!curve_grid(120, Some(500)).contains(&500));
    }

    /// La rejilla anual ancla en el suelo del plan, avanza de doce en doce y tiene tope.
    #[test]
    fn the_yearly_grid_anchors_on_the_floor_and_has_a_ceiling() {
        let g = yearly_grid(840, 1);
        assert_eq!(g.first(), Some(&1));
        assert!(g.len() <= YEARLY_GRID_MAX_NODES, "{} nodos", g.len());
        assert!(g.iter().all(|m| *m <= 840));
        assert!(g.windows(2).all(|w| w[1] - w[0] == YEARLY_GRID_STEP_MONTHS));
        // Con puente el suelo sube y la tira empieza ahí.
        assert_eq!(yearly_grid(840, 200).first(), Some(&200));
        // Un suelo por encima del horizonte no produce ningún nodo (nunca uno inventado).
        assert!(yearly_grid(120, 500).is_empty());
    }

    /// **El suelo del puente**, la única cosa que el puente hace en este módulo.
    #[test]
    fn the_bridge_lowers_the_floor_by_its_maximum_years_and_never_below_one() {
        let mut p = tiny_profile(PlanStrategy::Asap);
        assert_eq!(p.bridge_k_min(), 1, "sin puente el suelo es el mes 1");
        p.bridge = Some((600, 7));
        assert_eq!(p.bridge_k_min(), 600 - 84);
        p.bridge = Some((24, 20));
        assert_eq!(p.bridge_k_min(), 1, "el suelo nunca baja del mes 1");
    }

    /// **El cruce que de verdad se puede romper**: el literal que `serde` produce para cada
    /// estrategia del PERFIL tiene que ser uno de los que [`PlanStrategy::from_wire`] acepta.
    ///
    /// Es exactamente lo que hace el ensamblado (`projection.rs`:
    /// `PlanStrategy::from_wire(&strategy_label(perfil.strategy)).unwrap_or(PlanStrategy::Asap)`),
    /// y ese `unwrap_or` es lo que convierte un renombrado en un **fallo silencioso**: un literal
    /// que dejara de casar no revienta, degrada la estrategia a `asap` y el usuario recibe el plan
    /// de otra persona sin que ningún campo lo diga. Hasta 5.0.0 aquí había un round-trip contra
    /// `PlanStrategy::as_wire`, un gemelo que solo usaba este test: comparaba esta tabla consigo
    /// misma y no miraba el lado que puede moverse.
    #[test]
    fn the_profile_literals_are_the_ones_the_solver_accepts() {
        use crate::handlers::projection::strategy_label;
        use crate::handlers::retirement_profile::RetirementStrategy;
        for (profile, expected) in [
            (RetirementStrategy::Asap, PlanStrategy::Asap),
            (RetirementStrategy::RetireAtAge, PlanStrategy::RetireAtAge),
            (RetirementStrategy::Coast, PlanStrategy::Coast),
            (RetirementStrategy::Partial, PlanStrategy::Partial),
        ] {
            let wire = strategy_label(profile);
            assert_eq!(
                PlanStrategy::from_wire(&wire),
                Some(expected),
                "el perfil serializa `{wire}` y el solver no lo reconoce: el ensamblado \
                 degradaría a `asap` en silencio"
            );
        }
        // El alias de 4.15.x lo resuelve el PERFIL antes de llegar aquí (A1), así que este lado
        // tiene que seguir rechazándolo: aceptarlo sería un segundo sitio donde vive la migración.
        assert_eq!(PlanStrategy::from_wire("pension_bridge"), None);
        assert_eq!(PlanStrategy::from_wire(""), None);
    }
}
