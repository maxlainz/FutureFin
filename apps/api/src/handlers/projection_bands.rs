//! **Bandas de percentil de Monte Carlo** (5.0.0, WP A6; decisiones C3, D11, D23, D25 y D28).
//!
//! `GET /v1/projection/bands` es la superficie HTTP de
//! [`futurefin_engine_stochastic::project_percentile_bands`]: corre `paths` caminos del MISMO
//! bucle que produce la línea determinista, con los factores de crecimiento sorteados, y publica
//! bandas puntuales p10/p50/p90 más los contadores de fallo.
//!
//! # Lo que define este endpoint
//!
//! 1. **Se sortea la MISMA entrada que dibuja la serie** (`build_installation_projection_input`
//!    con `LedgerView::Mine`). Desde 6.0.0 (D-7) esa entrada no se jubila nunca —el modelo de
//!    jubilación se retiró—, así que el motor no puede clasificar ningún fallo (solo los cuenta
//!    estando jubilado) y el sorteo devolvería mecánicamente un éxito de `1`. Ese `1` NO se
//!    publica: las tres cifras del éxito viajan a `null` con
//!    `success_absent_reason: "no_retirement_phase"`. F2-3 redefine el éxito sobre el plan entero
//!    (D-9: poder pagar los gastos todos los meses hasta el horizonte).
//! 2. **Solo `view=mine`.** `household` devuelve 400 `household_bands_unavailable`: los
//!    percentiles **no suman** (con el shock común de D11 los mercados de los miembros no son
//!    independientes).
//! 3. **Sin parámetro `density`** (arqueología §2.18, veto 22). Siempre `hybrid`.
//! 4. **Semilla estable por usuario** (D23): `seed_for(installation_id, user_id)`. El override
//!    existe para mirar OTRO mercado a propósito, y viaja ecoado en la respuesta.
//! 5. **Cache propio** (`AppState::bands_cache`), con clave `(instalación, usuario, paths, seed)`
//!    y el TTL de la proyección. Se invalida en los MISMOS dos sitios que la serie, porque sale del
//!    MISMO `ProjectionInput`.
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
//! medidas. Memoria: `2 · paths · (horizonte+1) · 8` bytes de muestras — **33,6 MB** con el
//! default de 2 500 caminos × 840 meses y **67,3 MB** en el extremo de 5 000. El semáforo de
//! simulaciones (`heavy::run_projection_sim`) es lo que acota el pico agregado.
//!
//! El presupuesto **se aplica a priori, acotando `paths`**, y no con un `timeout` alrededor de
//! la tarea: `spawn_blocking` no se puede cancelar. `computed_in_ms` publica lo que costó de
//! verdad.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::person_view::LedgerView;
use crate::handlers::projection::{
    build_installation_projection_input, density_month_indices, engine_month_to_grid,
    jubilacion_civil, resolve_projection_context, serialize_decimal_as_f64, ProjectionContext,
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
/// **NO es el `DEFAULT_PATHS` del crate** (500, que basta para buscar): 2.500 caminos son los que
/// hacen falta para **publicar** — `2500/2503,84 = 0,99847`, o sea una barra de error de
/// **0,15 pp** hacia abajo con cero fallos.
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

/// `success_absent_reason` (y su gemelo `success_probability_absent_reason` de
/// `simulate_projection`) desde 6.0.0 (D-7): el escenario sorteado no tiene fase de jubilación, el
/// motor solo clasifica fallos estando jubilado y el sorteo devolvería mecánicamente un éxito de
/// `1`. F2-3 redefine el éxito sobre el plan entero (D-9) y retira este literal.
pub(crate) const SUCCESS_ABSENT_NO_RETIREMENT_PHASE: &str = "no_retirement_phase";

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
    /// Fracción de caminos con **algún fallo** (F1 o F2) en ese mes o antes — es acumulada,
    /// así que crece monótonamente. `0.12` = 12 de cada 100 escenarios.
    ///
    /// **La última fila es siempre el HORIZONTE**. La rejilla avanza de 60 en 60 desde la
    /// jubilación y se fuerza ese cierre; sin jubilación (6.0.0) la tabla trae esa única fila.
    ///
    /// **Ojo con el número de esa última fila**: el horizonte del BUCLE es el mes `months`, y en
    /// la rejilla publicada eso es `months − 1` (`engine_month_to_grid`: un hecho del mes `k` del
    /// bucle se anuncia en la casilla `k − 1`, la frontera en la que ya es cierto). O sea que la
    /// tabla **no** termina en el mismo índice que el último punto de `points[]`, que llega hasta
    /// `months`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub probability: Option<Decimal>,
    /// **El reparto por motivo `[F1 cartera agotada, F2 tasa inicial excedida]` de la EJECUCIÓN
    /// ENTERA — no del tramo hasta este mes.**
    ///
    /// Se repite igual en todas las filas y es deliberado, no un descuido de serialización:
    /// `McOutcome` clasifica el PRIMER fallo de cada camino sobre todo el horizonte y no lo
    /// desglosa por mes. El reparto es por tanto **exacto en la última fila** —la que cuenta el
    /// mismo conjunto de caminos— y en las anteriores describe la composición del horizonte
    /// completo, no la del tramo. Inventarse aquí un reparto por tramo sería una segunda
    /// implementación de una clasificación que el motor ya posee: si hace falta, se pide en
    /// `crates/engine-stochastic`.
    pub by_kind: [u32; 2],
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
    /// **El éxito: la fracción de caminos sin ningún fallo**. Estimador puntual,
    /// `1 − fallos/paths`.
    ///
    /// **`null` desde 6.0.0 (D-7)**, con [`Self::success_absent_reason`]: el escenario no se
    /// jubila y el motor solo clasifica fallos estando jubilado, así que el sorteo devolvería
    /// mecánicamente un `1` que se leería como «llegas». Un hueco se publica como hueco. F2-3 lo
    /// redefine sobre el plan entero (D-9).
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_of_plan: Option<Decimal>,
    /// **La cota inferior del intervalo de Wilson al 95 %** de [`Self::success_of_plan`].
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
    /// con **un decimal**. Es la barra que se dibuja hacia abajo.
    ///
    /// **Nunca es 0 con cero fallos**: 0 de 2.500 vale 0,1534 pp y se publica `"0.2"`. Ese es el
    /// caso que existe para cubrir — la aproximación normal daría exactamente cero ahí y
    /// declararía «100 % seguro».
    ///
    /// **El único 0 posible es el simétrico**, con TODOS los caminos fallidos: ahí `p̂ = 0`, la
    /// cota inferior de Wilson vale 0 exacto —una probabilidad no baja de cero— y la barra HACIA
    /// ABAJO no tiene dónde ir. No es «medición sin error»: es que toda la incertidumbre está del
    /// otro lado.
    ///
    /// `null` con [`Self::success_absent_reason`]: sin éxito que medir no hay barra que dibujar, y
    /// un `"0"` ahí se leería como «medición sin error», justo lo contrario de lo que este campo
    /// existe para decir.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_sampling_error_pp: Option<Decimal>,
    /// **Por qué no hay éxito que publicar**. `null` ⟺ las tres cifras de arriba viajan
    /// resueltas. Desde 6.0.0 vale siempre `no_retirement_phase`
    /// ([`SUCCESS_ABSENT_NO_RETIREMENT_PHASE`]).
    ///
    /// **Lo demás de la respuesta sigue viajando**: las bandas de percentiles,
    /// [`Self::failures_by_kind`] y [`Self::failure_probability_by_age`] describen la
    /// trayectoria del patrimonio **sin jubilarse**. Sus ceros se leen con esa etiqueta: «sin
    /// jubilación no hay fallo que contar», no «riesgo cero».
    ///
    /// **Viaja siempre, también como `null`** (nada de `skip_serializing_if`).
    #[schema(value_type = Option<String>)]
    pub success_absent_reason: Option<&'static str>,
    /// **Fallos por motivo, contando el PRIMER fallo de cada camino**, en el orden fijo
    /// `[F1 cartera agotada, F2 tasa inicial excedida]` — los mismos índices que
    /// `KIND_PORTFOLIO_DEPLETED`/`KIND_INITIAL_RATE_EXCEEDED` del crate, para que las dos capas no
    /// diverjan en el orden. (Era un array de tres hasta el 2026-10-10: el tercero contaba F3, el
    /// fallo de las reglas de retirada por saldo, retirado con ellas.)
    ///
    /// Son **contadores**, no probabilidades:
    /// dividir entre `paths` es cosa de quien los lee, y publicarlos crudos es lo que permite
    /// distinguir «se me acabó el dinero» de «retiré demasiado el primer año» — dos problemas con
    /// arreglos opuestos que una sola probabilidad de ruina confundía.
    ///
    pub failures_by_kind: [u32; 2],
    /// Probabilidad acumulada de fallo cada cinco años desde la jubilación, cerrando siempre en el
    /// horizonte. Ver [`FailureProbabilityPoint`]. **Sin jubilación (6.0.0) trae UNA sola fila**,
    /// la del horizonte, y vale `0`: «sin jubilación no hay fallo que contar», no «plan seguro».
    pub failure_probability_by_age: Vec<FailureProbabilityPoint>,
    // `months_below_need_p50` y `withdrawal_to_need_ratio_p50` viajaban aquí hasta el
    // 2026-10-10. Medían el recorte de una regla de retirada por saldo; sin reglas, un mes por
    // debajo del gasto ES un fallo F1, y sus medianas solo podían salir de «0 meses / 100 %» con
    // un éxito por debajo del 50 % — fuera de cualquier umbral aceptable (80–100). Lo que sí
    // dice cuándo y por qué se rompe el plan es `failure_probability_by_age` y `failures_by_kind`.
    /// `false` ⟺ **ningún activo declara volatilidad**: entonces las tres bandas coinciden entre
    /// sí y con la línea determinista, y la UI debe decirlo («sin volatilidad declarada: la banda
    /// es la línea») en vez de dibujar un abanico plano que se lee como certeza.
    pub any_volatility_declared: bool,
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
pub(crate) const BANDS_MODEL_NOTE: &str = "Monte Carlo sobre el MISMO bucle que la línea determinista, con los factores de crecimiento sorteados. MODELO: un shock de mercado COMÚN por mes (un solo z~N(0,1) que viven todos los activos a la vez), escalado por la volatilidad de cada uno: factor = m·exp(σz) con σ = annual_volatility_percent/100/√12 y m el multiplicador mensual determinista. La rentabilidad que declaraste es COMPUESTA (CAGR), así que m es la MEDIANA del factor sorteado y la línea determinista es la CENTRAL de la banda, ni techo ni suelo; la media aritmética queda por encima, y esa diferencia no es un error, es el coste de la volatilidad. σ = 0 ⇒ el camino determinista exacto. PUNTUAL QUIERE DECIR PUNTUAL: cada percentil se calcula mes a mes sobre los caminos de ESE mes, así que la curva p50 NO es una simulación real y no cumple ninguna identidad contable — no la cites como «tu patrimonio probable» punto a punto. SIN JUBILACIÓN (6.0.0): el modelo de jubilación se retiró y el escenario sorteado no se jubila; el motor solo cuenta fallos estando jubilado, así que `success_of_plan`, `success_wilson_low` y `success_sampling_error_pp` viajan a NULL con `success_absent_reason: no_retirement_phase` en vez de un 100 % que no mide nada. Las bandas, `failures_by_kind` y `failure_probability_by_age` sí se publican: describen la trayectoria SIN jubilarse, y ahí un 0 significa «sin jubilación no hay fallo que contar», no «riesgo cero». N IMPORTA: `paths` viaja al lado porque una probabilidad sin su tamaño de muestra no se compara con nada. SEMILLA estable por usuario: las mismas cifras hoy y dentro de un año, salvo que cambies los datos; con `seed` o `paths` miras otro mercado u otro tamaño de muestra. LO QUE NO SE MODELA, dicho para que nadie lo suponga: colas gruesas (el shock es log-normal), autocorrelación o reversión a la media (los meses son independientes: sin ciclos), correlación imperfecta entre activos (con un shock común la correlación es exactamente 1 y una cartera diversificada NO se beneficia aquí de su diversificación), bootstrap histórico (el sorteo es paramétrico), volatilidad de la inflación, de los ingresos, del gasto o del tipo de la deuda (solo los activos sortean), y rebalanceo (cada activo compone por su cuenta). Los importes de las bandas son NOMINALES, como la serie. De aquí solo salen probabilidades, contadores y percentiles: ningún euro.";

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

/// **Core sin HTTP, con la política de cache dentro.** La comparten el handler GET y la tool MCP
/// `get_projection_bands`.
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
    let key = BandsCacheKey {
        installation_id: iid,
        user_id,
        paths,
        seed,
    };
    if let Some(cached) = state.bands_cache_get(&key).await {
        tracing::info!(installation_id = %iid, paths, seed, "projection bands cache HIT");
        return Ok(cached);
    }
    tracing::info!(installation_id = %iid, paths, seed, "projection bands cache MISS, computing");
    // La generación se captura ANTES de computar (ver `AppState::projection_generation`).
    let generation = state.projection_generation(iid).await;
    let ctx = resolve_projection_context(&state.pool, iid, user_id, None).await?;
    let response = Arc::new(compute_projection_bands(state, user_id, iid, ctx, paths, seed).await?);
    state
        .bands_cache_insert_if_current(key, generation, response.clone())
        .await;
    Ok(response)
}

/// Calcula las bandas sin tocar el cache.
///
/// **El ensamblado es el MISMO** que el de la serie (`build_installation_projection_input` con
/// `LedgerView::Mine`): si las bandas salieran de un input propio, la línea determinista y su
/// abanico podrían describir dos escenarios distintos sin que nada lo dijera. Las volatilidades
/// viajan en el vector paralelo que ese mismo ensamblado construye activo a activo.
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
        None,
    )
    .await?;

    let volatilities = volatilities_f64(&built);
    let config = McConfig {
        seed,
        paths,
        percentiles: BANDS_PERCENTILES.to_vec(),
        // `None` = el pool compartido del crate estocástico. La API NUNCA fija hilos: ver la nota
        // de `heavy.rs` sobre permisos × hilos.
        threads: None,
    };
    let input = built.input;

    let t0 = std::time::Instant::now();
    let outcome = crate::heavy::run_projection_sim("monte carlo bands", move || {
        project_percentile_bands(&input, &volatilities, &config).map_err(map_mc_err)
    })
    .await??;
    let computed_in_ms = t0.elapsed().as_millis() as u64;

    Ok(assemble_bands_response(
        &outcome,
        ctx.months,
        ctx.horizon_basis,
        ctx.today,
        ctx.session_birth_date,
        Some(SUCCESS_ABSENT_NO_RETIREMENT_PHASE),
        computed_in_ms,
    ))
}

#[allow(clippy::too_many_arguments)]
fn assemble_bands_response(
    outcome: &McOutcome,
    months: u32,
    horizon_basis: String,
    today: chrono::NaiveDate,
    birth_date: Option<chrono::NaiveDate>,
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
        success_wilson_low: absent_reason
            .is_none()
            .then(|| probability_out(outcome.wilson_low))
            .flatten(),
        success_sampling_error_pp: absent_reason
            .is_none()
            .then(|| sampling_error_out(outcome.half_width_pp)),
        success_absent_reason: absent_reason,
        failures_by_kind: outcome.failures_by_kind,
        failure_probability_by_age,
        any_volatility_declared: outcome.any_volatility_declared,
        computed_in_ms,
        model_note: BANDS_MODEL_NOTE.into(),
    }
}

/// **La tabla de fallo acumulado por edad**, de la salida del sorteo a la respuesta publicada.
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

#[utoipa::path(
    get,
    path = "/v1/projection/bands",
    tag = "projection",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default). `household` → 400 `household_bands_unavailable`: los percentiles no se suman entre miembros."),
        ("paths" = Option<u32>, Query, description = "Caminos de Monte Carlo, 1..=5000 (default 2500). Fuera de rango → 400 `paths_out_of_range`."),
        ("seed" = Option<String>, Query, description = "Semilla de 64 bits en dígitos decimales. Omitida = la estable del usuario (D23)."),
    ),
    responses(
        (status = 200, description = "Bandas puntuales p10/p50/p90 de patrimonio y líquido (densidad hybrid), éxito (null con `success_absent_reason` desde 6.0.0: sin fase de jubilación), fallo acumulado por edad con el reparto por motivo.", body = ProjectionBandsResponse),
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
        let clean = SuccessAt::new(1, 2_500, 0, [0, 0]);
        let published = sampling_error_out(clean.half_width_pp);
        assert!(
            published > Decimal::ZERO,
            "0 de 2.500 son 0,1534 pp y se publican como 0,2 — nunca como «sin error»: {published}"
        );
        assert_eq!(published.scale(), SAMPLING_ERROR_DP, "un decimal");
        // Y con 500 caminos la barra es CUATRO veces mayor: es lo que justifica el default nuevo.
        let small = SuccessAt::new(1, 500, 0, [0, 0]);
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
        let all_fail = SuccessAt::new(1, 2_500, 2_500, [2_500, 0]);
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
