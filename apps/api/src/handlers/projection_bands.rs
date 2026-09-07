//! **Bandas de percentil de Monte Carlo** (5.0.0, modelo v2 de jubilación — WP A6; decisiones
//! C3, D11, D23, D25, D28 y E9).
//!
//! `GET /v1/projection/bands` es la superficie HTTP de
//! [`futurefin_engine_stochastic::project_percentile_bands`]: corre `paths` caminos del MISMO
//! bucle que produce la línea determinista, con los factores de crecimiento sorteados, y publica
//! bandas puntuales p10/p50/p90 más las cifras de riesgo del plan.
//!
//! # Las CINCO decisiones que definen este endpoint
//!
//! 1. **Se sortea EL PLAN, no el ensamblado en crudo** (modelo v2). Lo que se simula es el
//!    escenario que la serie publica: la entrada del ensamblado con el **mes de jubilación
//!    forzado** del nivel 1 del solver
//!    ([`crate::handlers::retirement_solver::plan_scenario`] sobre `BuiltProjection::plan_level1`
//!    si el ensamblado ya lo trae, o sobre el [`solve_plan_level1`] que este módulo corre con
//!    `built.plan_profile` si no). Hasta 4.15.x la jubilación la decidía un CRUCE dentro de
//!    cada camino, así que cada camino se jubilaba en un mes distinto y «éxito» mezclaba dos
//!    preguntas —¿ocurre? ¿aguanta?— en una cifra. En la v2 la fecha es **un dato del plan**, la
//!    misma en los `paths` caminos, y el éxito mide una sola cosa: **si ese plan se rompe**.
//! 2. **Solo `view=mine`.** `household` devuelve 400 `household_bands_unavailable`: los
//!    percentiles **no suman**. El p90 del hogar no es el p90 de Ana más el p90 de Bea — eso
//!    solo sería cierto si sus mercados fueran independientes, y con el shock común de D11 no lo
//!    son ni de lejos; sumar dos bandas produciría una banda demasiado ancha en el centro y
//!    demasiado estrecha en las colas, sin que ninguna cifra de la respuesta lo dijera. Es la
//!    misma razón por la que `simulate_projection` rechaza el hogar
//!    (`household_not_simulable`): un plan es de una persona.
//! 3. **Sin parámetro `density`** (arqueología §2.18, veto 22). Siempre `hybrid`. Servir la
//!    banda mensual completa multiplicaría por cinco un payload que ya lleva SEIS series
//!    donde la proyección lleva una, y no respondería a ninguna pregunta nueva.
//! 4. **Semilla estable por usuario** (D23): `seed_for(installation_id, user_id)`. Sin ella la
//!    probabilidad de éxito bailaría a cada refresco, que es exactamente lo que hace inservible
//!    a una herramienta de este tipo. El override existe para poder mirar OTRO mercado a
//!    propósito, y viaja ecoado en la respuesta: una banda sin su semilla no es reproducible y
//!    por tanto no es un resultado. **El `?seed=` solo mueve el SORTEO, jamás el solve**: la
//!    fecha del plan se resuelve siempre con la semilla estable, porque es una decisión de
//!    producto y no una vista — si cambiara con el parámetro, «mira otro mercado» se convertiría
//!    en «cámbiame el plan» sin que ningún campo lo dijera.
//! 5. **Cache propio** (`AppState::bands_cache`), con clave `(instalación, usuario, paths, seed,
//!    umbral)` y el TTL de la proyección. Se invalida en los MISMOS dos sitios que la serie
//!    (`invalidate_projection_by_installation` / `..._by_user`), porque sale del MISMO
//!    `ProjectionInput`: una banda vieja junto a una línea nueva son dos cifras que se
//!    contradicen en la misma pantalla. El **umbral** entró en la clave en 5.0.0 porque entró en
//!    la respuesta (ver [`ProjectionBandsResponse::success_verdict`]).
//!
//! # La IDENTIDAD con la serie, dicha con precisión
//!
//! Con el **sorteo por defecto** —`paths` = [`DEFAULT_BANDS_PATHS`] y la semilla estable del
//! usuario— este endpoint y el bloque «plan» de `GET /v1/projection/series` publican **el mismo
//! `success_of_plan`, bit a bit**. No es una coincidencia numérica, es una identidad de
//! construcción:
//!
//! - [`DEFAULT_BANDS_PATHS`] **es** el presupuesto de confirmación del solver
//!   ([`crate::handlers::retirement_solver::SOLVE_CONFIRM_PATHS`], que se define como este
//!   mismo símbolo y no como un 2.500 escrito aparte);
//! - los caminos dependen solo de `(semilla, índice de camino)` — `path_rng(seed, p)` no depende
//!   del mes forzado ni de la lista de percentiles;
//! - el escenario sorteado aquí es el que el solver confirmó (`plan_scenario` sobre el MISMO
//!   `solve_plan_level1`, con la MISMA semilla estable).
//!
//! **Fuera de ese sorteo la identidad no se promete y no debe leerse como una discrepancia.** Con
//! `?seed=` se mira OTRO mercado y con `?paths=` se mide con OTRO tamaño de muestra; lo que **no**
//! se mueve es la fecha del plan, porque el solve no ve ninguno de los dos parámetros. Lo dice
//! también [`BANDS_MODEL_NOTE`], porque quien lee el JSON no lee esto.
//!
//! **Y hay un caso en que los dos campos, llamándose igual, dicen cosas distintas a propósito**:
//! sin fecha alcanzable la SERIE publica su `success_of_plan` —la mejor observación del solve, el
//! mes que más cerca se quedó— y aquí va `null` con [`ProjectionBandsResponse::success_absent_reason`].
//! No es una divergencia del sorteo: son dos preguntas. La serie contesta «¿cuánto te faltó?»; esta
//! respuesta contesta «¿se rompe tu plan?», y sin plan con fecha esa pregunta no tiene respuesta.
//! La identidad bit a bit de arriba aplica a los planes CON fecha, que son todos los demás.
//!
//! # El caso sin fecha alcanzable: `null` con su motivo, jamás un verde
//!
//! Cuando el nivel 1 no encuentra ningún mes que cumpla el umbral
//! (`retirement_date_basis = not_reachable`), `plan_scenario` construye el escenario que la serie
//! publica: **no jubilarse dentro del horizonte** (`AtMonth(horizonte + 1)`). Y lo mismo pasa
//! cuando no hay plan que resolver (`plan_absent_reason`): se sortea la entrada del ensamblado
//! tal cual, que también se jubila en `horizonte + 1`.
//!
//! El motor **solo clasifica fallos estando jubilado o en media jornada**
//! (`sim_core.rs`: «un mes ACUMULANDO con déficit puede vaciar la cartera, pero eso no es un plan
//! de jubilación que falla»). Luego un escenario que nunca se jubila **no puede fallar**, y el
//! sorteo devuelve mecánicamente `success_probability = 1` con cero fallos de los tres tipos.
//!
//! Hasta 5.0.0 eso se publicaba tal cual —`success_of_plan: "1"`, `success_verdict: "green"`— con
//! una nota pidiendo que se leyera la serie al lado. **Era un verde falso**: la regla de oro de
//! esta API dice que un hueco se publica como `null` CON el campo que dice por qué falta, nunca
//! como una cifra tranquilizadora que hay que ir a desmentir a otro endpoint. Y la SPA, que pinta
//! el semáforo con este campo, pintaba de verde un plan que no existe.
//!
//! Desde 5.0.0 (WP A12) las **cuatro** cifras del éxito —[`ProjectionBandsResponse::success_of_plan`],
//! [`ProjectionBandsResponse::success_wilson_low`],
//! [`ProjectionBandsResponse::success_sampling_error_pp`] y
//! [`ProjectionBandsResponse::success_verdict`]— van a `null` y viaja
//! [`ProjectionBandsResponse::success_absent_reason`] con el motivo. **Se decide ANTES del
//! sorteo**, mirando `PlanLevel1::forced_month` / `BuiltProjection::plan_absent_reason`, y no
//! adivinando por el resultado: un plan real con éxito 1 y un plan que no ocurre son
//! indistinguibles mirando solo la salida del sorteo.
//!
//! Lo que **se sigue publicando** son las bandas de percentiles, `failures_by_kind`,
//! `failure_probability_by_age`, `months_below_need_p50` y `withdrawal_to_need_ratio_p50`: son la
//! trayectoria del patrimonio **sin jubilación**, que es una respuesta legítima a «¿qué pasa si no
//! me jubilo?». Sus ceros hay que leerlos con esa etiqueta puesta —«sin jubilación no hay fallo
//! que contar», no «riesgo cero»—, y por eso `success_absent_reason` viaja en la misma respuesta.
//! Pinea el comportamiento `a_plan_with_no_reachable_date_draws_the_line_that_never_retires` y
//! `a_plan_without_a_reachable_date_publishes_null_success_with_its_reason`.
//!
//! # El presupuesto de tiempo, dicho con números
//!
//! Medido en release sobre el caso P9 del motor (840 meses, 5 activos, 2 pasivos, cascada con
//! topes, impuestos por tramos) — `crates/engine-stochastic/tests/timing_mc.rs`:
//!
//! ```text
//!    100 caminos ·  20,5 ms  (medido)      500 caminos · 104,2 ms  (medido)
//!  1 000 caminos · 204,1 ms  (medido)    2 000 caminos · 391,4 ms  (medido)
//!  2 500 caminos · ≈ 500 ms  (DERIVADA)   5 000 caminos · ≈ 1,0 s  (DERIVADA)
//! ```
//!
//! Las dos últimas filas son **derivadas de los ~0,2 ms por camino** que sostienen las cuatro
//! medidas (el coste es lineal en `paths`: cada camino es una simulación completa e independiente,
//! y la ordenación por mes es `O(paths·log paths)` sobre un vector que ya está en caché). Se
//! marcan como derivadas y no como medidas a propósito: `timing_mc.rs::five_hundred_paths_of_p9`
//! recorre hoy `[100, 500, 1 000, 2 000]` y **no** incluye los dos techos nuevos, así que nadie ha
//! cronometrado todavía el default de este endpoint. Quien añada esas dos filas al arnés borra
//! esta advertencia y escribe el número medido.
//!
//! Memoria: `2 · paths · (horizonte+1) · 8` bytes de muestras — **33,6 MB** (32 MiB) con el
//! default de 2 500 caminos × 840 meses y **67,3 MB** (64 MiB) en el extremo de 5 000. El semáforo
//! de simulaciones (`heavy::run_projection_sim`, 2–8 permisos) es lo que acota el pico agregado.
//!
//! El presupuesto **se aplica a priori, acotando `paths`**, y no con un `timeout` alrededor de
//! la tarea: `spawn_blocking` no se puede cancelar, así que un timeout solo liberaría al
//! llamante mientras la CPU sigue ardiendo — y con un cliente que reintenta, empeoraría
//! exactamente el problema que pretende resolver. Como el horizonte ya está acotado a 840 meses,
//! `paths ≤ 5 000` acota el trabajo entero por construcción, y `computed_in_ms` publica lo que
//! costó de verdad para que el presupuesto sea auditable en vez de una promesa.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::person_view::LedgerView;
use crate::handlers::projection::{
    build_installation_projection_input, density_month_indices, engine_month_to_grid,
    jubilacion_civil, resolve_projection_context, serialize_decimal_as_f64, strategy_label,
    ProjectionContext,
};
use crate::handlers::retirement_solver::{
    level1_fingerprint, plan_scenario, solve_plan_level1, PlanBudget, DATE_BASIS_NOT_REACHABLE,
};
use crate::handlers::session::require_session_user;
use crate::state::{AppState, BandsCacheKey, Density};
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use futurefin_engine_stochastic::{
    project_percentile_bands, seed_for, McConfig, McError, McOutcome, MAX_PATHS,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Percentiles publicados. **Fijos**: la sección «Riesgo» de D28 dibuja un fan chart de tres
/// líneas, y un eje configurable convertiría la forma de la respuesta —y del cache— en algo que
/// depende del llamante sin que ninguna pregunta lo pida.
pub(crate) const BANDS_PERCENTILES: [u8; 3] = [10, 50, 90];

/// **Caminos por defecto de la superficie de bandas: 2.500.**
///
/// Desde 5.0.0 **NO es el `DEFAULT_PATHS` del crate** (que sigue valiendo 500 y sirve para lo que
/// solo dibuja: la curva de capital por edad, la tira anual, cualquier sondeo de búsqueda). Los
/// dos números se desacoplaron porque miden cosas distintas:
///
/// - 500 caminos bastan para **buscar** (comparar un mes con el siguiente); su cota de Wilson con
///   cero fallos topa en `500/503,84 = 0,99238`.
/// - 2.500 caminos son los que hacen falta para **publicar**: `2500/2503,84 = 0,99847`, o sea una
///   barra de error de **0,15 pp** hacia abajo con cero fallos, y un umbral del 99 % alcanzable
///   con holgura (la cota inferior de tamaño de muestra es 381 caminos; ver
///   `retirement_solver`).
///
/// Y sobre todo: **este es el presupuesto de CONFIRMACIÓN del solver**
/// ([`crate::handlers::retirement_solver::SOLVE_CONFIRM_PATHS`] se define como este símbolo). Esa
/// identidad es load-bearing — es lo que hace que la fecha que decide el plan y el fan chart que
/// lo dibuja salgan de **la misma muestra** en vez de dos ejecuciones que publicarían dos
/// probabilidades del mismo plan en dos pantallas.
pub(crate) const DEFAULT_BANDS_PATHS: u32 = 2_500;

/// Techo de caminos por HTTP: **5.000**, que es también el techo duro del crate
/// ([`MAX_PATHS`]) — así que `resolve_paths` es la única puerta y `McError::InvalidPaths` es
/// inalcanzable desde esta superficie. ≈ 1,0 s por sorteo de 840 meses (derivado; ver el doc del
/// módulo).
pub(crate) const HTTP_MAX_PATHS: u32 = 5_000;

/// Techo de caminos por MCP y por el eje `monte_carlo` de `simulate_projection`. La mitad del de
/// HTTP **a propósito**: un agente en bucle es el llamante que más fácil satura el semáforo, y
/// `simulate_projection` es cache-neutral por diseño (cada what-if paga sus caminos enteros).
/// 2.500 caminos son el default de la superficie —o sea, lo que ya está cacheado— y la diferencia
/// estadística con 5.000 (0,1534 pp de barra frente a 0,0768) es dos órdenes de magnitud menor que
/// el ancho de la propia banda.
pub(crate) const MCP_MAX_PATHS: u32 = 2_500;

/// **Los techos de esta capa no pueden pasarse del techo DURO del crate.** Se comprueba en tiempo
/// de COMPILACIÓN y no en un test: si `HTTP_MAX_PATHS` superara [`MAX_PATHS`], `resolve_paths`
/// dejaría pasar un valor que `PathEngine::new` rechaza, y el 400 saldría por `map_mc_err` con otro
/// mensaje —`paths_out_of_range` con el número del crate en vez del de la superficie— desde una
/// rama que este módulo documenta como inalcanzable.
const _: () = assert!(HTTP_MAX_PATHS <= MAX_PATHS && MCP_MAX_PATHS <= HTTP_MAX_PATHS);

/// El default tiene que caber en las dos superficies, o la tool MCP contestaría 400 a su propio
/// default. Compile-time por la misma razón que el de arriba.
const _: () = assert!(DEFAULT_BANDS_PATHS <= MCP_MAX_PATHS);

pub(crate) const VERDICT_GREEN: &str = "green";
pub(crate) const VERDICT_AMBER: &str = "amber";
pub(crate) const VERDICT_RED: &str = "red";

/// Decimales de los importes de las bandas. Dos, como las series de `/v1/history/series`: son
/// geometría de un fan chart salida de un `f64`, y publicar sus 17 dígitos significativos sería
/// precisión inventada sobre el percentil de una muestra.
pub(crate) const BANDS_VALUE_DP: u32 = 2;

/// Decimales de fracción de las probabilidades. **La misma política que `savings_rate`**
/// (`handlers/summary.rs::RATIO_DP`): 6 decimales de fracción = 4 de porcentaje. Con 5.000
/// caminos la resolución real del estimador es `1/5000 = 0,0002`, así que estos 6 decimales
/// sobran holgadamente — y son de PRESENTACIÓN: el redondeo se aplica al publicar, nunca a un
/// valor que alimente otra cuenta.
pub(crate) const BANDS_RATIO_DP: u32 = crate::handlers::summary::RATIO_DP;

/// Decimales de la **barra de error** publicada aquí, en puntos porcentuales: **uno**.
///
/// Es la barra que la UI dibuja hacia abajo junto al porcentaje («94,2 % −0,2 pp»), y un décimo de
/// punto porcentual es ya cuatro veces más fino que la resolución de un camino con el default
/// (2.500 caminos ⇒ 0,04 pp).
///
/// **OJO, divergencia deliberada**: el bloque «plan» de la serie publica ESTA MISMA medición con
/// cuatro decimales (`retirement_solver::SAMPLING_ERROR_DP`), porque allí es una cifra auditable
/// del solve y no una barra de un gráfico. Las dos salen del mismo `half_width_pp`; si algún día
/// se comparan campo a campo, lo que hay que comparar es la medición, no la cadena (0,1534 aquí se
/// publica «0.2»).
const SAMPLING_ERROR_DP: u32 = 1;

/// Una probabilidad de Monte Carlo (`f64` en `[0, 1]`) llevada al `Decimal` que la API publica
/// como string.
///
/// Es la ÚNICA frontera por la que sale un número del crate estocástico, y sale como
/// **probabilidad**, nunca como euros: es la regla del crate («de aquí no sale un euro») vista
/// desde este lado. Un `f64` no representable se publica como `null` en vez de como `0`
/// —inalcanzable hoy, porque el cociente de dos contadores siempre lo es—, para que un hueco
/// nunca se lea como «cero por ciento».
pub(crate) fn probability_out(p: f64) -> Option<Decimal> {
    Decimal::from_f64_retain(p).map(|d| d.round_dp(BANDS_RATIO_DP))
}

/// La barra de error de Wilson en puntos porcentuales → el `Decimal` que se publica como string.
///
/// `from_f64_retain` no puede fallar: `half_width_pp` sale de Wilson sobre una muestra finita y
/// vive en `[0, 100]`. La rama imposible publica `Decimal::ZERO` y **deja rastro en el log**,
/// porque un 0 ahí se leería como «medición sin error», que es exactamente lo contrario de lo que
/// este número existe para decir.
pub(crate) fn sampling_error_out(half_width_pp: f64) -> Decimal {
    match Decimal::from_f64_retain(half_width_pp) {
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

#[derive(Debug, Deserialize)]
pub struct ProjectionBandsQuery {
    /// `mine` (default) o `household` — este último es 400 `household_bands_unavailable`.
    #[serde(default)]
    pub view: Option<String>,
    /// Caminos a sortear (1..=5000; default 2500). Fuera de rango es 400, nunca un clamp.
    #[serde(default)]
    pub paths: Option<u32>,
    /// Semilla de 64 bits **como cadena de dígitos**. Ver [`ProjectionBandsResponse::seed`].
    #[serde(default)]
    pub seed: Option<String>,
}

/// Un punto de las bandas. Seis importes por punto, todos `f64` (excepción chart-only D4/I3, la
/// misma que `ProjectionPoint`): son geometría de un fan chart, no cifras que nadie sume.
///
/// **La banda p50 no es un camino.** Cada percentil se calcula mes a mes sobre los `paths`
/// valores de ESE mes, así que la curva p50 no corresponde a ninguna simulación real y no cumple
/// ninguna identidad contable: su `net_worth_p50` no tiene por qué ser la suma de nada, y
/// `net_worth_liquid_p50` puede venir de otro camino distinto que `net_worth_p50`. Es lo que la
/// ayuda de la UI tiene que decir, y por eso también lo dice `model_note`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionBandPoint {
    /// Número de MES desde `anchor_date_ymd`, **nunca** la posición en el array: la densidad es
    /// `hybrid` y los puntos no son equidistantes. Es la MISMA rejilla que
    /// `GET /v1/projection/series`, así que los dos se dibujan en el mismo eje sin traducir.
    pub month_index: u32,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth_p10: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth_p50: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth_p90: Decimal,
    /// Bandas del LÍQUIDO. **Por HTTP viajan siempre**; la tool MCP `get_projection_bands` las
    /// omite salvo con `include_liquid_bands: true` porque son la mitad exacta del payload y
    /// responden a una sola pregunta («cómo se vacía la hucha»). Ausente ≠ cero: la clave
    /// desaparece entera, no se sirve un `0` que se leería como «sin líquido».
    #[serde(
        serialize_with = "serialize_opt_decimal_as_f64",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<f64>)]
    pub net_worth_liquid_p10: Option<Decimal>,
    #[serde(
        serialize_with = "serialize_opt_decimal_as_f64",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<f64>)]
    pub net_worth_liquid_p50: Option<Decimal>,
    #[serde(
        serialize_with = "serialize_opt_decimal_as_f64",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<f64>)]
    pub net_worth_liquid_p90: Option<Decimal>,
}

/// Gemelo de `serialize_decimal_as_f64` para los tres campos opcionales de arriba. La rama
/// `None` solo se alcanza si alguien retira el `skip_serializing_if`; se emite `null` y no `0`
/// por la misma razón de siempre.
fn serialize_opt_decimal_as_f64<S: serde::Serializer>(
    d: &Option<Decimal>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match d {
        Some(v) => serialize_decimal_as_f64(v, s),
        None => s.serialize_none(),
    }
}

/// Probabilidad ACUMULADA de que el plan haya **fallado** en un mes o antes, cada cinco años desde
/// la jubilación del plan (D28 en su versión v2: «probabilidad de que esto se te rompa a los
/// 70/75/80/85/90»).
///
/// **Sustituye a `depletion_probability_by_age`**, que solo contaba el agotamiento de la cartera.
/// Desde el modelo v2 el bucle clasifica TRES motivos de fallo y el agotamiento es solo uno de
/// ellos, así que una tabla que mirara únicamente ese motivo publicaría ceros tranquilizadores
/// sobre planes que se rompen por otro lado — que es la razón exacta por la que se cambió.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FailureProbabilityPoint {
    /// Mes en la MISMA rejilla que `points[].month_index`.
    pub month_index: u32,
    /// Años cumplidos en ese mes. `null` ⟺ el usuario no tiene fecha de nacimiento: la tabla se
    /// sigue publicando por meses, porque la cifra existe aunque no se pueda rotular con una edad.
    pub age: Option<u32>,
    /// Fracción de caminos con **algún fallo** (F1, F2 o F3) en ese mes o antes — es acumulada,
    /// así que crece monótonamente. `0.12` = 12 de cada 100 escenarios.
    ///
    /// **La última fila es siempre el HORIZONTE** y cierra, al bit, en `1 − success_of_plan`: es
    /// el mismo conjunto de caminos contado de otra manera. La rejilla avanza de 60 en 60 desde la
    /// jubilación del plan y se fuerza ese cierre, así que **el último paso puede ser de menos de
    /// cinco años**: con jubilación en el mes 655 y horizonte 840, la penúltima fila es el mes 835.
    ///
    /// **Ojo con el número de esa última fila**: el horizonte del BUCLE es el mes `months`, y en
    /// la rejilla publicada eso es `months − 1` (`engine_month_to_grid`: un hecho del mes `k` del
    /// bucle se anuncia en la casilla `k − 1`, la frontera en la que ya es cierto). O sea que la
    /// tabla **no** termina en el mismo índice que el último punto de `points[]`, que llega hasta
    /// `months`. Es la misma traducción que usaban `depletion_probability_by_age` y
    /// `jubilacion_month_index`, y cambiarla aquí sola descolocaría esta tabla respecto del resto
    /// de la API.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub probability: Option<Decimal>,
    /// **El reparto por motivo `[F1 cartera agotada, F2 tasa inicial excedida, F3 la regla no
    /// llega a la necesidad]` de la EJECUCIÓN ENTERA — no del tramo hasta este mes.**
    ///
    /// Se repite igual en todas las filas y es deliberado, no un descuido de serialización:
    /// `McOutcome` clasifica el PRIMER fallo de cada camino sobre todo el horizonte y no lo
    /// desglosa por mes. El reparto es por tanto **exacto en la última fila** —la que cuenta el
    /// mismo conjunto de caminos— y en las anteriores describe la composición del horizonte
    /// completo, no la del tramo. Inventarse aquí un reparto por tramo sería una segunda
    /// implementación de una clasificación que el motor ya posee: si hace falta, se pide en
    /// `crates/engine-stochastic`.
    ///
    /// Es la MISMA convención que
    /// [`crate::handlers::retirement_solver::PlanExtras::failure_probability_by_age`], para que la
    /// tabla de esta respuesta y la del bloque «plan» de la serie no signifiquen dos cosas
    /// distintas con el mismo nombre.
    pub by_kind: [u32; 3],
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionBandsResponse {
    /// Siempre `"mine"`. Se ecoa igual que en el resto de respuestas con scope, aunque solo
    /// pueda valer una cosa: quien lee una respuesta no tiene por qué saber qué vistas existen.
    pub view: &'static str,
    /// Horizonte simulado en meses (el mismo que `GET /v1/projection/series` para este usuario).
    pub months: u32,
    /// De dónde sale `months`: `lifespan_age` | `fallback_no_demographics`. Nunca
    /// `months_override`: este endpoint no acepta `?months=` — un horizonte a medida cambiaría
    /// la banda y no cabe en la clave del cache.
    pub horizon_basis: String,
    /// Mes 0 de la rejilla (`YYYY-MM-DD`), en el calendario de la instalación.
    pub anchor_date_ymd: String,
    /// Caminos efectivamente sorteados. **Viaja siempre y se lee siempre**: una probabilidad sin
    /// su tamaño de muestra no se puede comparar con otra, ni consigo misma de ayer.
    pub paths: u32,
    /// La semilla usada, **como cadena de dígitos decimales**.
    ///
    /// No es un número JSON a propósito: es un entero sin signo de 64 bits y `JSON.parse` de
    /// cualquier navegador lo redondea por encima de 2^53. Un cliente que leyera la semilla como
    /// número y la devolviera para «repetir el mismo sorteo» obtendría OTRO mercado sin que nada
    /// fallara — el fallo silencioso exacto que la reproducibilidad existe para evitar. Se acepta
    /// también como string en `?seed=`.
    pub seed: String,
    /// Los percentiles publicados, en el orden de los campos de `points[]`. Fijo `[10, 50, 90]`.
    pub percentiles: Vec<u8>,
    /// Bandas puntuales, decimadas a `hybrid` (mes 0..12 mensual, luego anual, más el último mes
    /// del horizonte). Misma rejilla que `points[]` de la serie.
    pub points: Vec<ProjectionBandPoint>,
    /// **El éxito DEL PLAN: la fracción de caminos en los que el plan no se rompe** (E9, modelo
    /// v2). Estimador puntual, `1 − fallos/paths`. `0.87` = 87 de cada 100 escenarios.
    ///
    /// Un camino «se rompe» cuando el bucle le anota un fallo, y hay **tres motivos y solo tres**
    /// (ver [`Self::failures_by_kind`]): F1 la cartera se agota, F2 la tasa inicial de retirada
    /// excede el tope, F3 la regla por saldo no llega a la necesidad ordinaria **con el líquido
    /// con el que se entra en la jubilación**. F2 y F3 se juzgan en ese mes y solo en ese mes;
    /// después manda F1.
    ///
    /// **Ya no mide «¿ocurre el plan?»**, y por eso desaparecieron `never_retired_probability` y
    /// `success_given_retired`: en la v2 la fecha de jubilación es un DATO del plan —la misma en
    /// los `paths` caminos, forzada por el solver— y no un suceso que cada camino pueda o no
    /// alcanzar. Si esa fecha existe o no lo dice `retirement_date_basis` en la serie.
    ///
    /// **`null` cuando el escenario sorteado no lleva mes de jubilación**, y entonces
    /// [`Self::success_absent_reason`] dice por qué (5.0.0, WP A12). Es el caso en que el motor no
    /// puede clasificar ningún fallo —solo los clasifica estando jubilado o en media jornada—, así
    /// que el sorteo devolvería mecánicamente un `1` que significa «este plan sin jubilación no se
    /// rompe» y se leería como «llegas». Un hueco se publica como hueco. Ver el doc del módulo.
    ///
    /// **Identidad con la serie**: con el sorteo por defecto ([`DEFAULT_BANDS_PATHS`] caminos y la
    /// semilla estable del usuario) esta cifra ES, bit a bit, el `success_of_plan` del bloque
    /// «plan» de `GET /v1/projection/series`. Con `?seed=` o `?paths=` se re-sortea y la identidad
    /// no aplica: lo que no se mueve es la FECHA del plan. Ver el doc del módulo.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_of_plan: Option<Decimal>,
    /// **El umbral de éxito del perfil, `80..=100`** (C3), ecoado porque es **la restricción que
    /// decidió la fecha** y el listón contra el que se mide [`Self::success_verdict`]. Un color
    /// sin su umbral no se puede auditar, y dos personas con umbrales distintos no están mirando
    /// el mismo verde.
    pub success_threshold_pct: u32,
    /// **La cota inferior del intervalo de Wilson al 95 %** de [`Self::success_of_plan`], y **el
    /// número contra el que se compara el umbral** por debajo de 100 (C3).
    ///
    /// Wilson y no la aproximación normal por la razón por la que existe: con un estimador puntual
    /// de 1 la normal da una barra de error exactamente cero y declararía «100 % seguro» con 2.500
    /// caminos. Aquí, con cero fallos de 2.500, la cota vale 0,998466 — estrictamente menor que 1,
    /// que es lo honesto.
    ///
    /// `null` con [`Self::success_absent_reason`], por la misma razón que [`Self::success_of_plan`].
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_wilson_low: Option<Decimal>,
    /// Distancia del estimador puntual a [`Self::success_wilson_low`], en **puntos porcentuales**,
    /// con **un decimal**. Es la barra que se dibuja hacia abajo, que es el lado que decide el
    /// umbral.
    ///
    /// **Nunca es 0 con cero fallos**: 0 de 2.500 vale 0,1534 pp y se publica `"0.2"`. Ese es el
    /// caso que existe para cubrir — la aproximación normal daría exactamente cero ahí y
    /// declararía «100 % seguro».
    ///
    /// **El único 0 posible es el simétrico**, con TODOS los caminos fallidos: ahí `p̂ = 0`, la
    /// cota inferior de Wilson vale 0 exacto —una probabilidad no baja de cero— y la barra HACIA
    /// ABAJO no tiene dónde ir. No es «medición sin error»: es que toda la incertidumbre está del
    /// otro lado, y este campo publica el lado que decide el umbral.
    ///
    /// La serie publica esta misma medición con cuatro decimales (es una cifra auditable del
    /// solve, no una barra de un gráfico); ver [`SAMPLING_ERROR_DP`].
    ///
    /// `null` con [`Self::success_absent_reason`]: sin éxito que medir no hay barra que dibujar, y
    /// un `"0"` ahí se leería como «medición sin error», justo lo contrario de lo que este campo
    /// existe para decir.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_sampling_error_pp: Option<Decimal>,
    /// `green` | `amber` | `red` — **el semáforo, medido contra el umbral DEL PERFIL** y con la
    /// MISMA regla con la que el solver decidió la fecha (`SuccessAt::meets`, C3):
    ///
    /// ```text
    ///   verde  ⟺  cumple:  umbral < 100 ⇒ success_wilson_low ≥ umbral/100
    ///                      umbral = 100 ⇒ success_of_plan == 1  (cero fallos de N)
    ///   ámbar  ⟺  no cumple pero success_of_plan ≥ umbral/100
    ///             (el estimador puntual llega; el intervalo, no)
    ///   rojo   ⟺  ni el estimador puntual llega
    /// ```
    ///
    /// El corte FIJO al 100 % de la primera vuelta de 5.0.0 (V7) se retiró con ella: el umbral
    /// volvió al perfil como restricción de producto —es lo que decide la fecha—, así que un
    /// semáforo con otro listón estaría coloreando un plan distinto del que se publicó.
    ///
    /// **Con `umbral = 100` no existe el ámbar** y es correcto: ahí «cumple» es «cero fallos», y
    /// cero fallos es exactamente `success_of_plan == 1`, que es también la condición del ámbar.
    /// O ningún camino se rompe, o es rojo.
    ///
    /// **`null` cuando no hay plan con fecha que colorear** ([`Self::success_absent_reason`]).
    /// Hasta 5.0.0 ese caso salía `"green"` —el escenario sin jubilación no puede fallar— y la SPA
    /// pintaba de verde un plan que no existe; el semáforo de un plan ausente es la ausencia de
    /// semáforo, no un color.
    #[schema(value_type = Option<String>)]
    pub success_verdict: Option<&'static str>,
    /// **Por qué no hay éxito que publicar** (5.0.0, WP A12). `null` ⟺ las cuatro cifras de
    /// arriba viajan resueltas.
    ///
    /// Se decide **antes del sorteo**, mirando el nivel 1 del plan, y toma uno de estos literales:
    ///
    /// - `not_reachable` — hay plan y **ningún mes del horizonte cumple el umbral**
    ///   ([`crate::handlers::retirement_solver::DATE_BASIS_NOT_REACHABLE`], el mismo literal que
    ///   `retirement_date_basis` en `GET /v1/projection/series`, para que las dos superficies no
    ///   nombren la misma situación de dos maneras);
    /// - el mismo literal que `plan_absent_reason` de la serie cuando **no hay plan**:
    ///   `birth_date_missing` es el único alcanzable por esta ruta (`months_override` necesita un
    ///   `?months=` que este endpoint no acepta, y `household_not_solved` un `view=household` que
    ///   aquí es 400 `household_bands_unavailable`) — se propaga el que traiga el ensamblado en vez
    ///   de reescribirlo, porque la causa es la misma y un literal propio sería un segundo
    ///   vocabulario para el mismo hecho.
    ///
    /// **Lo demás de la respuesta sigue viajando**: las bandas de percentiles,
    /// [`Self::failures_by_kind`], [`Self::failure_probability_by_age`],
    /// [`Self::months_below_need_p50`] y [`Self::withdrawal_to_need_ratio_p50`] describen la
    /// trayectoria del patrimonio **sin jubilarse**, que es una respuesta legítima. Sus ceros se
    /// leen con esa etiqueta: «sin jubilación no hay fallo que contar», no «riesgo cero».
    ///
    /// **Viaja siempre, también como `null`** (nada de `skip_serializing_if`): es el campo que
    /// convierte un hueco en una respuesta, y una clave que desaparece obliga al consumidor a
    /// distinguir «no falta nada» de «este servidor no publica el motivo».
    #[schema(value_type = Option<String>)]
    pub success_absent_reason: Option<&'static str>,
    /// **Fallos por motivo, contando el PRIMER fallo de cada camino**, en el orden fijo
    /// `[F1 cartera agotada, F2 tasa inicial excedida, F3 la regla no llega a la necesidad]` —
    /// los mismos índices que `KIND_PORTFOLIO_DEPLETED`/`KIND_INITIAL_RATE_EXCEEDED`/
    /// `KIND_RULE_BELOW_NEED` del crate, para que las dos capas no diverjan en el orden.
    ///
    /// Suma exactamente `paths − paths·success_of_plan`. Son **contadores**, no probabilidades:
    /// dividir entre `paths` es cosa de quien los lee, y publicarlos crudos es lo que permite
    /// distinguir «se me acabó el dinero» de «retiré demasiado el primer año» — dos problemas con
    /// arreglos opuestos que una sola probabilidad de ruina confundía.
    ///
    /// **F2 y F3 se deciden en el mes de jubilación**, así que sus contadores describen la FECHA;
    /// solo F1 puede firmar más tarde.
    pub failures_by_kind: [u32; 3],
    /// **Cuándo se rompe el plan**: probabilidad acumulada de fallo cada cinco años desde la
    /// jubilación del plan, cerrando siempre en el horizonte. Ver [`FailureProbabilityPoint`].
    ///
    /// **Con un plan sin fecha alcanzable la tabla trae UNA sola fila**, la del horizonte
    /// (`months − 1` en la rejilla), y vale `0`: el ancla es el mes forzado (`horizonte + 1`), no
    /// cabe ningún nodo dentro, y el cierre en el horizonte se emite igual. **Ese 0 no dice «plan
    /// seguro», dice «sin jubilación no hay fallo que contar»** — y quién no tiene fecha lo dice
    /// [`ProjectionBandsResponse::success_absent_reason`] en esta misma respuesta, sin tener que
    /// ir a buscarlo a la serie. Ver el doc del módulo.
    ///
    /// **Vacía** solo si el llamante trae un `ProjectionInput` legacy por CRUCE en el que ningún
    /// camino se jubila: ahí no hay ancla que valga, y una tabla inventada sería peor que un hueco.
    pub failure_probability_by_age: Vec<FailureProbabilityPoint>,
    /// Mediana entre caminos del **NÚMERO DE MESES jubilados en que el hogar no cubrió su
    /// gasto**: cuenta el recorte de la regla (`withdrawal_shortfall > 0`) **y** el gasto que la
    /// cartera no pudo financiar (`unmet_need > 0`).
    ///
    /// Contar solo el recorte lo dejaba en `0` por construcción con `fixed_real` —la regla sin
    /// techo no recorta nunca—, también en los caminos que se quedaban sin cartera en el mes 35
    /// de 400: el mes sin dinero no aparecía en ninguna cifra publicada.
    pub months_below_need_p50: u32,
    /// Mediana entre caminos de **qué FRACCIÓN de su necesidad ordinaria cubrió el hogar de
    /// verdad** sobre los meses jubilados. `1` = la cubrió entera. `null` cuando ningún camino
    /// tiene meses jubilados con necesidad positiva.
    ///
    /// **Corregida en E9 (bug B2)**: el numerador descuenta el `withdrawal_excess` de
    /// `rule_is_spend` —la regla ES el gasto y vende lo permitido aunque sobre—, y el denominador
    /// es la necesidad neta, clampada mes a mes porque **puede ser negativa** desde que la pensión
    /// supera el gasto. Sin esas dos correcciones el cociente pasaba de 1,0 por arriba en los
    /// meses con superávit y valía 1,0 exacto en caminos que cubrían el 8,65 % de lo que
    /// necesitaban.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub withdrawal_to_need_ratio_p50: Option<Decimal>,
    /// `false` ⟺ **ningún activo declara volatilidad**: entonces las tres bandas coinciden entre
    /// sí y con la línea determinista, y la UI debe decirlo («sin volatilidad declarada: la banda
    /// es la línea») en vez de dibujar un abanico plano que se lee como certeza.
    pub any_volatility_declared: bool,
    /// Estrategia con la que se simuló: `asap` | `retire_at_age` | `coast` | `partial`. Mismo
    /// literal que `GET /v1/projection/series`.
    pub strategy: String,
    /// Milisegundos que costó el sorteo (0 en un HIT de cache). Es la mitad medible del
    /// presupuesto de tiempo del módulo: sin él, «cabe en un request» sería una promesa.
    pub computed_in_ms: u64,
    /// Supuestos con los que hay que leer todo lo de arriba. Ver [`BANDS_MODEL_NOTE`].
    pub model_note: String,
}

/// **El modelo, entero, incluida la lista de lo que NO representa.**
///
/// Vive en la respuesta y no solo en la documentación por la misma razón que
/// `PROJECTION_MODEL_NOTE`: un consumidor conversacional lee el JSON, no el repositorio, y una
/// probabilidad de ruina sin sus supuestos es un número que parece cierto.
pub(crate) const BANDS_MODEL_NOTE: &str = "Monte Carlo sobre el MISMO bucle que la línea determinista, con los factores de crecimiento sorteados. QUÉ SE SORTEA: EL PLAN, no el hogar en crudo — el escenario lleva ya el MES DE JUBILACIÓN que el solver fijó, el mismo en los `paths` caminos, así que la fecha es un DATO y lo único que se mide aquí es si ese plan se ROMPE. MODELO: un shock de mercado COMÚN por mes (un solo z~N(0,1) que viven todos los activos a la vez), escalado por la volatilidad de cada uno: factor = m·exp(σz) con σ = annual_volatility_percent/100/√12 y m el multiplicador mensual determinista. La rentabilidad que declaraste es COMPUESTA (CAGR), así que m es la MEDIANA del factor sorteado y la línea determinista es la CENTRAL de la banda, ni techo ni suelo; la media aritmética queda por encima, y esa diferencia no es un error, es el coste de la volatilidad. σ = 0 ⇒ el camino determinista exacto. PUNTUAL QUIERE DECIR PUNTUAL: cada percentil se calcula mes a mes sobre los caminos de ESE mes, así que la curva p50 NO es una simulación real y no cumple ninguna identidad contable — no la cites como «tu patrimonio probable» punto a punto. ÉXITO = la fracción de caminos SIN NINGÚN FALLO, y hay tres motivos y solo tres, que viajan contados aparte en `failures_by_kind`: F1 la cartera se agota, F2 la tasa inicial de retirada excede el tope del perfil, F3 la regla por saldo no llega a la necesidad ordinaria con el líquido de entrada. F2 y F3 se juzgan EN EL MES DE JUBILACIÓN y solo ahí —después manda F1, y el recorte que la regla haga más adelante viaja como lectura en `months_below_need_p50`, no como fallo—. Distinguirlos importa porque tienen arreglos OPUESTOS: F1 pide más capital o menos gasto, F2 pide retrasar la fecha, F3 pide cambiar la regla. UN CAMINO SOLO PUEDE FALLAR ESTANDO JUBILADO (o en media jornada): antes de la fecha, quedarse sin cartera es un problema de caja de hoy y no un plan de jubilación que se rompe. Consecuencia que se declara AQUÍ y no hay que ir a buscar a otro endpoint: si el escenario sorteado no lleva mes de jubilación —porque ningún mes cumple el umbral, o porque no hay plan que resolver— el sorteo no puede contar ni un fallo, así que `success_of_plan`, `success_wilson_low`, `success_sampling_error_pp` y `success_verdict` viajan a NULL y `success_absent_reason` dice por qué (`not_reachable`, o el mismo literal que `plan_absent_reason` de la serie). No hay un 100 % que desmentir: un hueco se publica como hueco. Lo que sí se sigue publicando son las bandas, `failures_by_kind` y `failure_probability_by_age`, porque describen la trayectoria SIN jubilarse — y ahí un 0 significa «sin jubilación no hay fallo que contar», no «riesgo cero». EL UMBRAL ES UNA RESTRICCIÓN, no un adorno del color: `success_threshold_pct` (80–100, tu perfil) es lo que decidió la FECHA, y el veredicto se mide contra él con la MISMA regla — por debajo de 100 se compara la cota inferior del intervalo de WILSON al 95 % (`success_wilson_low`), no el estimador puntual; en 100 se exige cero fallos de N. Verde = cumple; ámbar = el estimador puntual llega y el intervalo no; rojo = ni el puntual. Con umbral 100 no hay ámbar: o no se rompe ni un camino, o es rojo. N IMPORTA: `paths` viaja al lado porque una probabilidad sin su tamaño de muestra no se compara con nada, y `success_sampling_error_pp` es la barra hacia abajo — NUNCA cero, tampoco con cero fallos (0 de 2.500 son 0,15 pp). Y «100 %» no significa «seguro»: significa «ningún fallo en N caminos DE ESTE MODELO». SEMILLA estable por usuario: las mismas cifras hoy y dentro de un año, salvo que cambies los datos. Con el sorteo por defecto estas cifras son EXACTAMENTE las del plan que publica `/v1/projection/series`; si pasas `seed` o `paths` estás mirando otro mercado u otro tamaño de muestra, y entonces la fecha del plan NO se mueve — solo cambia la medición de su riesgo. `failure_probability_by_age` dice CUÁNDO se rompe: acumulada, cada cinco años desde la jubilación, y cierra SIEMPRE en el horizonte (esa última fila es 1 − éxito, y el paso hasta ella puede ser menor de cinco años; sin fecha de jubilación trae una sola fila valiendo 0, que es el caso de `success_absent_reason`). Su `by_kind` es el reparto de la ejecución ENTERA, exacto solo en esa última fila. El RECORTE de una regla de retirada no es por sí solo un fracaso y viaja aparte, en `months_below_need_p50` y `withdrawal_to_need_ratio_p50`, que cuentan el recorte de la regla Y el gasto que la cartera no pudo financiar. LO QUE NO SE MODELA, dicho para que nadie lo suponga: colas gruesas (el shock es log-normal, así que la probabilidad de ruina es OPTIMISTA en la cola), autocorrelación o reversión a la media (los meses son independientes: sin ciclos), correlación imperfecta entre activos (con un shock común la correlación es exactamente 1 y una cartera diversificada NO se beneficia aquí de su diversificación: el modelo es conservador en ese eje), bootstrap histórico (el sorteo es paramétrico: nada de esto es «lo que pasó entre 1929 y 1964»), volatilidad de la inflación, de los ingresos, del gasto o del tipo de la deuda (solo los activos sortean), y rebalanceo (cada activo compone por su cuenta). Los importes de las bandas son NOMINALES, como la serie. El patrimonio, el capital necesario y la aportación en EUROS siguen saliendo del camino exacto en Decimal: de aquí solo salen probabilidades, contadores y percentiles.";

/// Traduce un [`McError`] a la respuesta HTTP. Las tres variantes de configuración son 400 con
/// código estable; el fallo del motor reusa `map_engine_err`, que ya publica los códigos que el
/// catálogo conoce.
pub(crate) fn map_mc_err(e: McError) -> ApiError {
    match e {
        McError::Engine(e) => crate::handlers::projection::map_engine_err(e),
        McError::InvalidPaths(n) => ApiError::BadRequest(format!(
            "paths_out_of_range: paths must be between 1 and {MAX_PATHS} ({n} given)"
        )),
        McError::InvalidPercentiles => ApiError::BadRequest(
            "invalid_percentiles: percentiles must be between 1 and 99".into(),
        ),
        // Imposible por construcción: el vector se rellena en el MISMO `map` que los activos
        // (`build_installation_projection_input`). Si llegara aquí sería un bug del ensamblado y
        // no del llamante, así que se publica como 503 y no como un 400 que mandaría al usuario a
        // corregir unos datos correctos. El detalle va al log; el wire dice solo «no disponible».
        McError::VolatilityLengthMismatch(got, want) => {
            tracing::error!(
                got,
                want,
                "el vector de volatilidades se desalineó de input.assets — bug del ensamblado"
            );
            ApiError::Unavailable
        }
    }
}

/// **La frontera de entrada al camino estocástico**: las volatilidades del ensamblado, alineadas
/// con `input.assets`, convertidas a `f64`.
///
/// Vive aquí y no en `projection.rs` porque es la única función del API que produce `f64` para
/// alimentar al crate estocástico, y tenerla en un solo sitio es lo que garantiza que las bandas
/// y el eje `monte_carlo` de `simulate_projection` conviertan igual. La política de degradación
/// (negativa, no finita o cero ⇒ activo determinista) vive DENTRO del crate, declarada: aquí solo
/// se convierte, y un `Decimal` que no cabe en `f64` —imposible con la cota `[0, 100]` de la
/// API— degradaría a «sin volatilidad», que es la lectura conservadora.
pub(crate) fn volatilities_f64(
    built: &crate::handlers::projection::BuiltProjection,
) -> Vec<Option<f64>> {
    built
        .asset_volatility_percent
        .iter()
        .map(|v| v.and_then(|d| d.to_f64()))
        .collect()
}

/// Resuelve la semilla efectiva: el override del llamante, o la estable del usuario (D23).
pub(crate) fn resolve_seed(iid: Uuid, user_id: Uuid, override_seed: Option<u64>) -> u64 {
    override_seed.unwrap_or_else(|| seed_for(iid.as_u128(), user_id.as_u128()))
}

/// Parsea la semilla de un parámetro de texto. Se rechaza en vez de caer a la estable: una
/// semilla mal escrita que devolviera «el sorteo de siempre» sería indistinguible de haber
/// funcionado.
pub(crate) fn parse_seed(raw: Option<&str>) -> Result<Option<u64>, ApiError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => s.parse::<u64>().map(Some).map_err(|_| {
            ApiError::BadRequest(
                "invalid_seed: seed must be an unsigned 64-bit integer written in decimal digits"
                    .into(),
            )
        }),
    }
}

/// Valida `paths` contra el techo de la superficie. **Se rechaza, no se clampa** (misma doctrina
/// que `months`): devolver 200 con 5 000 caminos a quien pidió 50 000 es contestar otra pregunta.
///
/// El default es [`DEFAULT_BANDS_PATHS`] y no el `DEFAULT_PATHS` del crate: son dos números
/// distintos desde 5.0.0 (ver el doc de la constante).
pub(crate) fn resolve_paths(paths: Option<u32>, max: u32) -> Result<u32, ApiError> {
    let p = paths.unwrap_or(DEFAULT_BANDS_PATHS);
    if !(1..=max).contains(&p) {
        return Err(ApiError::BadRequest(format!(
            "paths_out_of_range: paths must be between 1 and {max}"
        )));
    }
    Ok(p)
}

/// **Core sin HTTP, con la política de cache dentro.** La comparten el handler GET, la tool MCP
/// `get_projection_bands` y el `/v1/summary.plan` (que lee de aquí para que el KPI «Éxito del
/// plan» sea EXACTAMENTE el número que dibuja el fan chart, y no una segunda ejecución con otra
/// muestra).
///
/// El contexto se resuelve ANTES de mirar la cache porque el **umbral del perfil está en la
/// clave**: sin él no se puede ni buscar la entrada. Son tres queries por clave (una consolidada a
/// `installation` y las dos DOB en paralelo) frente a un sorteo de medio segundo, y en el MISS se
/// reutiliza el mismo contexto en vez de resolverlo dos veces.
pub(crate) async fn projection_bands_cached(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    view: LedgerView,
    paths: u32,
    seed_override: Option<u64>,
) -> Result<Arc<ProjectionBandsResponse>, ApiError> {
    if matches!(view, LedgerView::Household) {
        return Err(ApiError::BadRequest(
            "household_bands_unavailable: percentile bands exist only for view=mine — percentiles do not add across members, and the market shock is common to all of them".into(),
        ));
    }
    let seed = resolve_seed(iid, user_id, seed_override);
    let ctx = resolve_projection_context(&state.pool, iid, user_id, None).await?;
    let threshold_pct = ctx.retirement_profile.success_threshold_pct;
    let key = BandsCacheKey {
        installation_id: iid,
        user_id,
        paths,
        seed,
        threshold_pct,
    };
    if let Some(cached) = state.bands_cache_get(&key).await {
        tracing::info!(
            installation_id = %iid, paths, seed, threshold_pct,
            "projection bands cache HIT"
        );
        return Ok(cached);
    }
    tracing::info!(
        installation_id = %iid, paths, seed, threshold_pct,
        "projection bands cache MISS, computing"
    );
    // Generación capturada ANTES del sorteo (WP A12): una banda insertada después de una
    // invalidación describiría unos activos que ya no existen, y lo haría al lado de una línea
    // determinista ya actualizada. Ver `AppState::projection_generation`.
    let generation = state.projection_generation(iid).await;
    let response = Arc::new(compute_projection_bands(state, user_id, iid, ctx, paths, seed).await?);
    state
        .bands_cache_insert_if_current(key, generation, response.clone())
        .await;
    Ok(response)
}

/// Calcula las bandas sin tocar el cache.
///
/// **El ensamblado es el MISMO** que el de la serie (`build_installation_projection_input` con
/// `LedgerView::Mine`, el perfil resuelto del usuario y su fecha de nacimiento): si las bandas
/// salieran de un input propio, la línea determinista y su abanico podrían describir dos planes
/// distintos sin que nada lo dijera. Las volatilidades viajan en el vector paralelo que ese
/// mismo ensamblado construye activo a activo.
///
/// **Y el escenario es el DEL PLAN**: sobre esa entrada se aplica `plan_scenario`, que fuerza el
/// mes de jubilación del nivel 1. Sortear la entrada en crudo dejaría a cada camino jubilándose
/// por su cuenta —el modelo de 4.15.x— y el éxito volvería a mezclar «¿ocurre?» con «¿aguanta?».
///
/// # De dónde sale ese mes
///
/// De [`BuiltProjection::plan_level1`](crate::handlers::projection::BuiltProjection) si el
/// ensamblado ya lo trae resuelto; y si no, **se resuelve aquí**, con
/// [`solve_plan_level1`] sobre `built.plan_profile` —el perfil traducido a meses del bucle, que
/// es lo único que el ensamblado puede construir porque es el único que conoce la fecha de
/// nacimiento de este miembro—. No es una segunda implementación del plan: es **la misma
/// función** que llama la serie, con los mismos presupuestos, así que las dos superficies no
/// pueden discrepar por construcción. Sin plan que resolver (`plan_absent_reason`: usuario sin
/// fecha de nacimiento, etc.) se sortea la entrada tal cual, que es exactamente la línea que la
/// serie dibuja en ese caso.
///
/// # Dos semillas, y la diferencia importa
///
/// El **solve** usa siempre la semilla ESTABLE del usuario (`seed_for`), nunca el `?seed=` del
/// llamante: la fecha del plan es una decisión de producto, no una vista, y no puede moverse
/// porque alguien quiera mirar otro mercado. El **sorteo** usa la semilla pedida. Cuando las dos
/// coinciden —el caso por defecto— y `paths` es [`DEFAULT_BANDS_PATHS`], la muestra del sorteo ES
/// la de la confirmación y el éxito publicado aquí es, bit a bit, el del plan.
///
/// # Un solo permiso
///
/// El solve y el sorteo van dentro de **una** llamada a `heavy::run_projection_sim`, igual que el
/// nivel 1 va dentro del permiso de la serie: pedir dos permisos dejaría a este endpoint
/// compitiendo consigo mismo por el semáforo que existe para que `/v1/ready` siga respondiendo.
async fn compute_projection_bands(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    ctx: ProjectionContext,
    paths: u32,
    seed: u64,
) -> Result<ProjectionBandsResponse, ApiError> {
    let built = build_installation_projection_input(
        &state.pool,
        iid,
        user_id,
        LedgerView::Mine,
        ctx.today,
        ctx.months,
        ctx.inflation_annual_percent,
        Some(&ctx.fire_settings),
        &ctx.retirement_profile,
        ctx.session_birth_date,
        None,
    )
    .await?;

    let volatilities = volatilities_f64(&built);

    let config = McConfig {
        seed,
        paths,
        percentiles: BANDS_PERCENTILES.to_vec(),
    };

    let input = built.input.clone();
    let level1 = built.plan_level1.clone();
    let plan_profile = built.plan_profile.clone();
    let plan_absent_reason = built.plan_absent_reason;
    let has_plan = plan_absent_reason.is_none();
    // La semilla del PLAN es la estable del usuario, pase lo que pase con `?seed=`.
    let plan_seed = resolve_seed(iid, user_id, None);

    // **El nivel 1 se busca en la cache ANTES de gastar dos a cinco segundos en volverlo a
    // resolver** (5.0.0, WP A12). `build_installation_projection_input` no resuelve plan —solo lo
    // hace `run_member_projection`, dentro del miss de la serie—, así que `built.plan_level1` llega
    // siempre vacío aquí y este endpoint estaba re-resolviendo un solve que la serie ya había
    // hecho, con la MISMA entrada, el MISMO perfil y la MISMA semilla estable. Y la SPA pide las
    // dos superficies al abrir Jubilación, así que era el caso normal.
    //
    // La clave es de CONTENIDO (`level1_fingerprint` sobre los cinco argumentos del solve), así
    // que un hit **es** el mismo resultado y no una aproximación: si algo del hogar, del perfil,
    // del presupuesto o de la semilla cambia, la clave cambia y no hay hit que servir.
    let level1_key = has_plan.then(|| {
        level1_fingerprint(
            &input,
            &volatilities,
            plan_seed,
            &plan_profile,
            PlanBudget::FULL,
        )
    });
    let cached_level1 = match level1_key {
        Some(k) => state.level1_cache_get(&k).await,
        None => None,
    };
    if cached_level1.is_some() {
        tracing::info!(
            installation_id = %iid,
            level1_key = level1_key.map(|k| k.as_u64()),
            "plan level 1 cache HIT (bands reuse the series solve)"
        );
    }

    // Bajo el MISMO semáforo que las proyecciones (`heavy::run_projection_sim`): el recurso
    // escaso es el mismo —núcleos— y este llamante es el más caro de todos, así que dejarlo
    // fuera del techo habría reabierto el agujero que el semáforo cerró.
    let t0 = std::time::Instant::now();
    // **El mes forzado sale del sorteo, no se adivina de su resultado** (WP A12). La tarea
    // devuelve el `forced_month` del escenario que ha simulado junto a la salida: es el único
    // dato que distingue «este plan aguanta» de «este escenario no se jubila y por eso no puede
    // fallar», y las dos cosas producen exactamente la misma salida de Monte Carlo.
    let (outcome, forced_month, solved) = crate::heavy::run_projection_sim(
        "monte carlo bands",
        move || {
            // Tres orígenes para el mismo escenario, en orden de coste creciente: el nivel 1 que
            // trajera el ensamblado, el que la cache guarde, y el que hay que resolver. El
            // tercero devuelve además el `PlanLevel1` para que el llamante lo guarde: así la
            // SERIE hereda este solve igual que las bandas heredan el suyo, y la primera de las
            // dos superficies que llegue paga por las dos.
            let (scenario, forced_month, solved) = match (level1, cached_level1) {
                (Some(l1), _) => (plan_scenario(&input, &l1), l1.forced_month, None),
                (None, Some(l1)) => (plan_scenario(&input, &l1), l1.forced_month, None),
                (None, None) if has_plan => {
                    let l1 = solve_plan_level1(&input, &volatilities, plan_seed, &plan_profile)?;
                    let scenario = plan_scenario(&input, &l1);
                    let forced_month = l1.forced_month;
                    (scenario, forced_month, Some(Arc::new(l1)))
                }
                (None, None) => (input.clone(), None, None),
            };
            project_percentile_bands(&scenario, &volatilities, &config)
                .map_err(map_mc_err)
                .map(|outcome| (outcome, forced_month, solved))
        },
    )
    .await??;
    let computed_in_ms = t0.elapsed().as_millis() as u64;

    // Solo se guarda lo que se ha resuelto AQUÍ: un hit no se reinserta (ya refrescó su TTL al
    // leerse) y el nivel 1 que venía del ensamblado tampoco, porque quien lo resolvió ya lo guardó.
    if let (Some(key), Some(l1)) = (level1_key, solved) {
        state.level1_cache_insert(key, l1).await;
    }

    Ok(assemble_bands_response(
        &outcome,
        ctx.months,
        ctx.horizon_basis,
        ctx.today,
        ctx.session_birth_date,
        strategy_label(ctx.retirement_profile.strategy),
        ctx.retirement_profile.success_threshold_pct,
        success_absent_reason(plan_absent_reason, forced_month),
        computed_in_ms,
    ))
}

/// **Por qué esta respuesta no puede publicar un éxito**, decidido con el PLAN y no con la salida
/// del sorteo (5.0.0, WP A12).
///
/// Dos causas y ninguna más, en este orden:
///
/// 1. **No hay plan** (`plan_absent_reason` del ensamblado): se propaga su literal tal cual —
///    `birth_date_missing` es el único alcanzable desde esta superficie, ver el doc de
///    [`ProjectionBandsResponse::success_absent_reason`]—. Reescribirlo aquí crearía un segundo
///    vocabulario para la misma causa.
/// 2. **Hay plan y no tiene fecha**: el nivel 1 devolvió `forced_month: None`, que es exactamente
///    [`DATE_BASIS_NOT_REACHABLE`] — se reusa esa constante en vez de escribir el literal, para
///    que este campo y `retirement_date_basis` de la serie no puedan divergir en una letra.
///
/// El orden importa: sin plan tampoco hay mes forzado, así que la segunda condición se cumple
/// también en el primer caso y el literal que gana tiene que ser el más específico —el que dice
/// que **no se llegó a preguntar**, frente al que dice que se preguntó y no había respuesta.
///
/// La comparte el eje `monte_carlo` de `simulate_projection` por lado
/// (`SimKpis::success_probability_absent_reason`): el sorteo es el mismo, el fallo silencioso es
/// el mismo y una segunda copia de esta decisión divergiría en el primer literal nuevo.
pub(crate) fn success_absent_reason(
    plan_absent_reason: Option<&'static str>,
    forced_month: Option<u32>,
) -> Option<&'static str> {
    match (plan_absent_reason, forced_month) {
        (Some(reason), _) => Some(reason),
        (None, None) => Some(DATE_BASIS_NOT_REACHABLE),
        (None, Some(_)) => None,
    }
}

/// Salida del motor estocástico → respuesta publicada. Función aparte, y sin I/O, porque es
/// donde viven las DOS traducciones que se pueden equivocar en silencio: los meses del bucle a
/// la rejilla 0-based (`engine_month_to_grid`) y las probabilidades `f64` a `Decimal`.
///
/// **`absent_reason` gobierna las cuatro cifras del éxito y solo esas** (WP A12): con motivo, las
/// cuatro van a `null` y el motivo viaja; sin él, se publican las del sorteo. Se pasa como
/// parámetro —y no se deduce aquí de `outcome`— porque de la salida del sorteo NO se puede
/// deducir: un plan sin fecha y un plan perfecto producen el mismo `success_probability = 1`.
#[allow(clippy::too_many_arguments)]
fn assemble_bands_response(
    outcome: &McOutcome,
    months: u32,
    horizon_basis: String,
    today: chrono::NaiveDate,
    birth_date: Option<chrono::NaiveDate>,
    strategy: String,
    threshold_pct: u32,
    absent_reason: Option<&'static str>,
    computed_in_ms: u64,
) -> ProjectionBandsResponse {
    let len = outcome
        .net_worth
        .first()
        .map(|b| b.len())
        .unwrap_or((months + 1) as usize);
    // Densidad `hybrid` SIEMPRE (§2.18): mismos índices que `points[]` de la serie, así que las
    // dos curvas se superponen sin traducir nada.
    let kept = density_month_indices(Density::Hybrid, len as u32);
    // **Dos decimales**, la misma resolución con la que `/v1/history/series` publica sus valores
    // de chart (`history_chart_values_are_published_with_two_decimals`). No es cosmética: el
    // valor viene de un `f64`, y `from_f64_retain` + `to_f64` reproduce sus 17 dígitos
    // significativos en el JSON — precisión INVENTADA para el percentil de una muestra, y ~40 %
    // más de payload por punto. La resolución real de estas cifras es el ancho de la banda, no el
    // céntimo.
    let at = |band: &[Vec<f64>], p: usize, i: usize| -> Decimal {
        band.get(p)
            .and_then(|row| row.get(i))
            .and_then(|v| Decimal::from_f64_retain(*v))
            .unwrap_or(Decimal::ZERO)
            .round_dp(BANDS_VALUE_DP)
    };
    let points: Vec<ProjectionBandPoint> = kept
        .iter()
        .map(|&i| {
            let idx = i as usize;
            ProjectionBandPoint {
                month_index: i,
                net_worth_p10: at(&outcome.net_worth, 0, idx),
                net_worth_p50: at(&outcome.net_worth, 1, idx),
                net_worth_p90: at(&outcome.net_worth, 2, idx),
                net_worth_liquid_p10: Some(at(&outcome.liquid_worth, 0, idx)),
                net_worth_liquid_p50: Some(at(&outcome.liquid_worth, 1, idx)),
                net_worth_liquid_p90: Some(at(&outcome.liquid_worth, 2, idx)),
            }
        })
        .collect();

    let failure_probability_by_age = failure_points(outcome, today, birth_date);

    ProjectionBandsResponse {
        view: LedgerView::Mine.as_str(),
        months,
        horizon_basis,
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        paths: outcome.paths,
        seed: outcome.seed.to_string(),
        percentiles: outcome.percentiles.clone(),
        points,
        // Las cuatro cifras del éxito viajan juntas o faltan juntas: publicar el intervalo sin el
        // punto —o la barra sin el intervalo— dejaría media medición suelta en una respuesta que
        // ya ha declarado que no hay nada que medir.
        success_of_plan: absent_reason
            .is_none()
            .then(|| probability_out(outcome.success_probability))
            .flatten(),
        success_threshold_pct: threshold_pct,
        success_wilson_low: absent_reason
            .is_none()
            .then(|| probability_out(outcome.wilson_low))
            .flatten(),
        success_sampling_error_pp: absent_reason
            .is_none()
            .then(|| sampling_error_out(outcome.half_width_pp)),
        success_verdict: absent_reason.is_none().then(|| {
            success_verdict(
                outcome.success_probability,
                outcome.wilson_low,
                threshold_pct,
            )
        }),
        success_absent_reason: absent_reason,
        failures_by_kind: outcome.failures_by_kind,
        failure_probability_by_age,
        months_below_need_p50: outcome.months_below_need_p50,
        withdrawal_to_need_ratio_p50: outcome
            .withdrawal_to_need_ratio_p50
            .and_then(probability_out),
        any_volatility_declared: outcome.any_volatility_declared,
        strategy,
        computed_in_ms,
        model_note: BANDS_MODEL_NOTE.into(),
    }
}

/// **La tabla «cuándo se rompe el plan»**, de la salida del sorteo a la respuesta publicada.
///
/// Vive aquí y no en cada llamante porque las dos superficies que la publican —esta respuesta y
/// los dos lados de `simulate_projection`— tienen que significar **lo mismo**: mismo paso de
/// rejilla, misma traducción de meses del bucle a la rejilla publicada
/// (`engine_month_to_grid`), misma edad civil y el mismo reparto por motivo repetido en todas las
/// filas (ver [`FailureProbabilityPoint::by_kind`]). Dos copias divergirían en el primer cambio
/// del crate, y la divergencia sería una tabla que se lee igual y cuenta otra cosa.
pub(crate) fn failure_points(
    outcome: &McOutcome,
    today: chrono::NaiveDate,
    birth_date: Option<chrono::NaiveDate>,
) -> Vec<FailureProbabilityPoint> {
    // El reparto por motivo es el de la EJECUCIÓN entera y se repite en todas las filas: el motor
    // clasifica el PRIMER fallo de cada camino sobre todo el horizonte y no lo desglosa por mes.
    let by_kind = outcome.failures_by_kind;
    outcome
        .cumulative_failure_by_age
        .iter()
        .map(|&(engine_month, p)| {
            // El motor cuenta meses 1-based; la respuesta habla la rejilla de `points[]`.
            let month_index = engine_month_to_grid(Some(engine_month)).unwrap_or(0);
            let (_, age) = jubilacion_civil(today, birth_date, Some(month_index));
            FailureProbabilityPoint {
                month_index,
                age,
                probability: probability_out(p),
                by_kind,
            }
        })
        .collect()
}

/// **El semáforo de D28 contra el umbral DEL PERFIL** (5.0.0, modelo v2, decisión C3).
///
/// ```text
///   verde  ⟺  cumple:  umbral < 100 ⇒ wilson_low ≥ umbral/100
///                      umbral = 100 ⇒ success == 1  (cero fallos de N)
///   ámbar  ⟺  no cumple pero success ≥ umbral/100
///   rojo   ⟺  ni eso
/// ```
///
/// # Por qué la condición de verde se escribe EXACTAMENTE así
///
/// Porque es la de `futurefin_engine_stochastic::SuccessAt::meets`, que es la que decidió la
/// FECHA. Un semáforo con otro listón —otro redondeo, otra comparación, otra tolerancia—
/// pintaría de ámbar el mes que el solver aceptó como válido, y esa contradicción no la
/// resolvería ningún campo de la respuesta. Se comparan **fracciones** (`umbral/100`) y no
/// puntos porcentuales por lo mismo: `meets` divide entre 100, así que dividir aquí también
/// garantiza el MISMO binario en el borde, incluido el 0,95 que no es representable en IEEE 754.
///
/// # Los dos bordes que no necesitan épsilon
///
/// - `umbral = 100` ⇒ se exige `success == 1.0`, que es `n/n` en `f64` — exacto para cualquier
///   `n` (numerador y denominador son el mismo entero), así que un `0,9999…` no puede colarse.
/// - `umbral = 100` **no tiene ámbar**: `success ≥ 1.0` ⟺ `success == 1.0` ⟺ verde. O ningún
///   camino se rompe, o es rojo. Es una consecuencia de la regla, no un caso especial escrito
///   aparte, y lo pinea `the_hundred_percent_threshold_has_no_amber_band`.
///
/// # Qué NO decide esta función
///
/// Si el plan **tiene** fecha. Con `retirement_date_basis = not_reachable` el escenario sorteado
/// es «no jubilarse dentro del horizonte», que no puede fallar, y esta función devolvería `green`
/// sobre un plan que no existe. Por eso **no se la llama en ese caso**: el llamante decide antes
/// con [`success_absent_reason`] y publica `success_verdict: null` con su motivo (WP A12). La
/// función se queda con una sola responsabilidad —comparar tres números— y quien la reutiliza
/// (`summary.rs`, `simulate_projection`) tiene que hacer la misma comprobación antes.
pub(crate) fn success_verdict(success: f64, wilson_low: f64, threshold_pct: u32) -> &'static str {
    let target = f64::from(threshold_pct) / 100.0;
    // La MISMA expresión que `SuccessAt::meets`, rama a rama.
    let meets = if threshold_pct >= 100 {
        threshold_pct == 100 && success == 1.0
    } else {
        wilson_low >= target
    };
    if meets {
        VERDICT_GREEN
    } else if success >= target {
        VERDICT_AMBER
    } else {
        VERDICT_RED
    }
}

#[utoipa::path(
    get,
    path = "/v1/projection/bands",
    tag = "projection",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default). `household` → 400 `household_bands_unavailable`: los percentiles no se suman entre miembros."),
        ("paths" = Option<u32>, Query, description = "Caminos de Monte Carlo, 1..=5000 (default 2500 = el presupuesto de confirmación del plan). Fuera de rango → 400 `paths_out_of_range`."),
        ("seed" = Option<String>, Query, description = "Semilla de 64 bits en dígitos decimales. Omitida = la estable del usuario (D23)."),
    ),
    responses(
        (status = 200, description = "Bandas puntuales p10/p50/p90 de patrimonio y líquido (densidad hybrid), éxito del plan con su umbral, su intervalo de Wilson y su veredicto, fallo acumulado por edad con el reparto por motivo, y las lecturas del recorte.", body = ProjectionBandsResponse),
        (status = 400, description = "`household_bands_unavailable`, `paths_out_of_range` o `invalid_seed`"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn get_projection_bands(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ProjectionBandsQuery>,
) -> Result<Json<ProjectionBandsResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let view = crate::handlers::person_view::LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve()?;
    let paths = resolve_paths(q.paths, HTTP_MAX_PATHS)?;
    let seed = parse_seed(q.seed.as_deref())?;
    let response = projection_bands_cached(&state, user.id.0, iid, view, paths, seed).await?;
    Ok(Json((*response).clone()))
}

pub fn projection_bands_router() -> Router {
    Router::new().route("/bands", get(get_projection_bands))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futurefin_engine_stochastic::SuccessAt;

    /// La cota inferior de Wilson al 95 % con **cero fallos** de `n` caminos: `n/(n + z²)`. Es la
    /// forma cerrada que documenta `wilson_lower_bound`, y se escribe aquí para poder elegir
    /// valores de prueba que caen a un lado y al otro del umbral SIN copiar la fórmula general.
    fn wilson_low_no_failures(n: u32) -> f64 {
        let z2 = 1.96f64 * 1.96;
        f64::from(n) / (f64::from(n) + z2)
    }

    /// **El verde es el «cumple» del solver, no una segunda opinión.**
    ///
    /// Es la aserción que de verdad importa de este módulo: si el veredicto usara otro criterio
    /// que `SuccessAt::meets`, la pantalla pintaría de ámbar el mes que el solver aceptó como
    /// fecha válida y ningún campo de la respuesta explicaría la contradicción. Se comprueba
    /// contra el tipo del crate, no contra una reimplementación.
    #[test]
    fn the_green_is_exactly_the_solvers_meets() {
        for threshold in [80u32, 90, 95, 99, 100] {
            for (paths, failures) in [
                (2_500u32, 0u32),
                (2_500, 1),
                (2_500, 13),
                (2_500, 25),
                (2_500, 126),
                (2_500, 500),
                (500, 0),
                (500, 4),
                (1, 0),
                (1, 1),
            ] {
                let s = SuccessAt::new(1, paths, failures, [failures, 0, 0]);
                let green = success_verdict(s.success, s.wilson_low, threshold) == VERDICT_GREEN;
                assert_eq!(
                    green,
                    s.meets(threshold),
                    "umbral {threshold}, {failures} fallos de {paths}: el verde y el «cumple» \
                     del solver tienen que ser la MISMA condición (success = {}, wilson_low = {})",
                    s.success,
                    s.wilson_low
                );
            }
        }
    }

    /// **Las tres regiones, con un umbral por debajo de 100** (aquí 95).
    ///
    /// El ámbar es la franja en la que el estimador puntual llega y el intervalo no — o sea,
    /// «puede que sí, pero la muestra no lo demuestra». Es la región que el corte fijo de la
    /// primera vuelta de 5.0.0 no sabía nombrar.
    #[test]
    fn the_verdict_has_three_regions_below_a_hundred() {
        const T: u32 = 95;
        // VERDE: cero fallos de 2.500 ⇒ wilson_low = 0,998466 ≥ 0,95.
        let clean = SuccessAt::new(1, 2_500, 0, [0, 0, 0]);
        assert!(clean.wilson_low >= 0.95, "wilson_low = {}", clean.wilson_low);
        assert_eq!(
            success_verdict(clean.success, clean.wilson_low, T),
            VERDICT_GREEN
        );

        // ÁMBAR: el puntual llega (0,96 ≥ 0,95) y el intervalo no (0,9522… con 100 fallos de
        // 2.500 sí llega, así que hace falta una muestra más pequeña para separarlos).
        let borderline = SuccessAt::new(1, 200, 6, [6, 0, 0]);
        assert!(
            borderline.success >= 0.95,
            "el puntual tiene que llegar: {}",
            borderline.success
        );
        assert!(
            borderline.wilson_low < 0.95,
            "y el intervalo NO: {}",
            borderline.wilson_low
        );
        assert_eq!(
            success_verdict(borderline.success, borderline.wilson_low, T),
            VERDICT_AMBER
        );

        // ROJO: ni el puntual.
        let bad = SuccessAt::new(1, 2_500, 500, [400, 50, 50]);
        assert!(bad.success < 0.95, "{}", bad.success);
        assert_eq!(success_verdict(bad.success, bad.wilson_low, T), VERDICT_RED);
    }

    /// **Con umbral 100 no hay ámbar**, y el borde no necesita épsilon.
    ///
    /// «Cumple» es «cero fallos», que es `success == 1.0` exacto: `n/n` en IEEE 754 vale
    /// exactamente 1 para cualquier `n`, así que un `(n−1)/n` no puede colarse como verde. Y como
    /// la condición del ámbar es `success ≥ 1`, coincide con la del verde: la franja intermedia
    /// está vacía por construcción, no por una regla escrita aparte.
    #[test]
    fn the_hundred_percent_threshold_has_no_amber_band() {
        for n in [1u32, 7, 24, 499, 500, 2_500, 5_000] {
            let clean = SuccessAt::new(1, n, 0, [0, 0, 0]);
            assert_eq!(clean.success, 1.0, "n/n debe ser exactamente 1 con n = {n}");
            assert_eq!(
                success_verdict(clean.success, clean.wilson_low, 100),
                VERDICT_GREEN,
                "n = {n}"
            );
            // Y la cota de Wilson con cero fallos es estrictamente menor que 1: el verde del 100 %
            // NO puede salir de ella, tiene que salir del contador de fallos.
            assert!(
                clean.wilson_low < 1.0 && clean.wilson_low > 0.0,
                "n = {n}: wilson_low = {}",
                clean.wilson_low
            );
            assert!(
                (clean.wilson_low - wilson_low_no_failures(n)).abs() < 1e-12,
                "la forma cerrada n/(n+z²) debe reproducir la cota: n = {n}"
            );

            if n > 1 {
                let one_short = SuccessAt::new(1, n, 1, [1, 0, 0]);
                let v = success_verdict(one_short.success, one_short.wilson_low, 100);
                assert_eq!(
                    v, VERDICT_RED,
                    "con umbral 100 un solo fallo es ROJO, nunca ámbar (n = {n})"
                );
            }
        }
    }

    /// El veredicto **se mueve con el umbral** sobre la MISMA muestra: es lo que obliga a que el
    /// umbral esté en la clave del cache y ecoado en la respuesta.
    #[test]
    fn the_same_sample_changes_colour_with_the_threshold() {
        // 25 fallos de 2.500: puntual 0,99, wilson_low ≈ 0,9856.
        let s = SuccessAt::new(1, 2_500, 25, [25, 0, 0]);
        assert_eq!(success_verdict(s.success, s.wilson_low, 80), VERDICT_GREEN);
        assert_eq!(success_verdict(s.success, s.wilson_low, 95), VERDICT_GREEN);
        assert_eq!(
            success_verdict(s.success, s.wilson_low, 99),
            VERDICT_AMBER,
            "el puntual es exactamente 0,99 y la cota no llega: {}",
            s.wilson_low
        );
        assert_eq!(
            success_verdict(s.success, s.wilson_low, 100),
            VERDICT_RED,
            "hay 25 fallos: el 100 % no admite ninguno"
        );
    }

    /// `paths` se rechaza fuera de rango, nunca se clampa, cada superficie trae su techo y el
    /// default es el de la SUPERFICIE (2.500), no el del crate (500).
    #[test]
    fn los_caminos_se_rechazan_fuera_de_rango() {
        assert_eq!(
            resolve_paths(None, HTTP_MAX_PATHS).unwrap(),
            DEFAULT_BANDS_PATHS
        );
        assert_eq!(DEFAULT_BANDS_PATHS, 2_500, "el default de la superficie");
        assert_ne!(
            DEFAULT_BANDS_PATHS,
            futurefin_engine_stochastic::DEFAULT_PATHS,
            "desde 5.0.0 el default de las bandas está DESACOPLADO del default del crate: el \
             primero publica, el segundo busca"
        );
        assert_eq!(resolve_paths(Some(1), HTTP_MAX_PATHS).unwrap(), 1);
        assert_eq!(resolve_paths(Some(5_000), HTTP_MAX_PATHS).unwrap(), 5_000);
        assert!(resolve_paths(Some(0), HTTP_MAX_PATHS).is_err());
        assert!(resolve_paths(Some(5_001), HTTP_MAX_PATHS).is_err());
        // El techo del MCP es la mitad: 5 000 es válido por HTTP y 400 por MCP.
        assert!(resolve_paths(Some(5_000), MCP_MAX_PATHS).is_err());
        assert_eq!(resolve_paths(Some(2_500), MCP_MAX_PATHS).unwrap(), 2_500);
    }

    /// La barra de error **nunca se publica como 0** en el caso que más se publica —cero fallos—,
    /// y se redondea a un decimal de punto porcentual.
    #[test]
    fn the_sampling_error_survives_the_rounding_to_one_decimal() {
        let clean = SuccessAt::new(1, 2_500, 0, [0, 0, 0]);
        let published = sampling_error_out(clean.half_width_pp);
        assert!(
            published > Decimal::ZERO,
            "0 de 2.500 son 0,1534 pp y se publican como 0,2 — nunca como «sin error»: {published}"
        );
        assert_eq!(published.scale(), SAMPLING_ERROR_DP, "un decimal");
        // Y con 500 caminos la barra es CUATRO veces mayor: es lo que justifica el default nuevo.
        let small = SuccessAt::new(1, 500, 0, [0, 0, 0]);
        assert!(
            small.half_width_pp > clean.half_width_pp * 3.0,
            "500 caminos: {} pp frente a {} pp con 2.500",
            small.half_width_pp,
            clean.half_width_pp
        );
        // **El caso simétrico**: con TODOS los caminos fallidos la cota inferior es 0 exacto —una
        // probabilidad no baja de cero— y la barra hacia abajo vale 0. No es un fallo del cálculo
        // ni «medición sin error»: es que toda la incertidumbre está del otro lado. Se pinea para
        // que nadie lo lea como el bug que la aproximación normal sí tiene con `p̂ = 1`.
        let all_fail = SuccessAt::new(1, 2_500, 2_500, [2_500, 0, 0]);
        assert_eq!(all_fail.success, 0.0);
        assert_eq!(all_fail.wilson_low, 0.0, "la cota no puede ser negativa");
        assert_eq!(sampling_error_out(all_fail.half_width_pp), Decimal::ZERO);
    }

    /// Una semilla de 64 bits entera sobrevive al viaje por texto — que es la razón de que viaje
    /// por texto.
    #[test]
    fn la_semilla_viaja_entera_por_texto() {
        assert_eq!(parse_seed(None).unwrap(), None);
        assert_eq!(parse_seed(Some("  ")).unwrap(), None);
        assert_eq!(parse_seed(Some("0")).unwrap(), Some(0));
        assert_eq!(
            parse_seed(Some("18446744073709551615")).unwrap(),
            Some(u64::MAX)
        );
        assert!(parse_seed(Some("-1")).is_err());
        assert!(parse_seed(Some("18446744073709551616")).is_err());
        assert!(parse_seed(Some("abc")).is_err());
    }
}
