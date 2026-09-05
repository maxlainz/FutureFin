//! **Bandas de percentil de Monte Carlo** (5.0.0, WP6b — §B.5/§B.6 y §F del plan de la issue
//! #207; decisiones D11, D12, D22, D23, D25, D28).
//!
//! `GET /v1/projection/bands` es la superficie HTTP de
//! [`futurefin_engine_stochastic::project_percentile_bands`]: corre `paths` caminos del MISMO
//! bucle que produce la línea determinista, con los factores de crecimiento sorteados, y publica
//! bandas puntuales p10/p50/p90 más las probabilidades del plan.
//!
//! # Las cuatro decisiones que definen este endpoint
//!
//! 1. **Solo `view=mine`.** `household` devuelve 400 `household_bands_unavailable`: los
//!    percentiles **no suman**. El p90 del hogar no es el p90 de Ana más el p90 de Bea — eso
//!    solo sería cierto si sus mercados fueran independientes, y con el shock común de D11 no lo
//!    son ni de lejos; sumar dos bandas produciría una banda demasiado ancha en el centro y
//!    demasiado estrecha en las colas, sin que ninguna cifra de la respuesta lo dijera. Es la
//!    misma razón por la que `simulate_projection` rechaza el hogar
//!    (`household_not_simulable`): un plan es de una persona.
//! 2. **Sin parámetro `density`** (arqueología §2.18, veto 22). Siempre `hybrid`. Servir la
//!    banda mensual completa multiplicaría por cinco un payload que ya lleva SEIS series
//!    donde la proyección lleva una, y no respondería a ninguna pregunta nueva.
//! 3. **Semilla estable por usuario** (D23): `seed_for(installation_id, user_id)`. Sin ella la
//!    probabilidad de éxito bailaría a cada refresco, que es exactamente lo que hace inservible
//!    a una herramienta de este tipo. El override existe para poder mirar OTRO mercado a
//!    propósito, y viaja ecoado en la respuesta: una banda sin su semilla no es reproducible y
//!    por tanto no es un resultado.
//! 4. **Cache propio** (`AppState::bands_cache`), con clave `(instalación, usuario, paths,
//!    seed)` y el TTL de la proyección. Se invalida en los MISMOS dos sitios que la serie
//!    (`invalidate_projection_by_installation` / `..._by_user`), porque sale del MISMO
//!    `ProjectionInput`: una banda vieja junto a una línea nueva son dos cifras que se
//!    contradicen en la misma pantalla.
//!
//! # El presupuesto de tiempo, dicho con números
//!
//! Medido en release sobre el caso P9 del motor (840 meses, 5 activos, 2 pasivos, cascada con
//! topes, impuestos por tramos) — `crates/engine-stochastic/tests/timing_mc.rs`:
//!
//! ```text
//!   100 caminos ·  20,5 ms     500 caminos · 104,2 ms  (el default)
//!  1000 caminos · 204,1 ms    2000 caminos · 391,4 ms  (el techo HTTP)
//! ```
//!
//! El presupuesto **se aplica a priori, acotando `paths`**, y no con un `timeout` alrededor de
//! la tarea: `spawn_blocking` no se puede cancelar, así que un timeout solo liberaría al
//! llamante mientras la CPU sigue ardiendo — y con un cliente que reintenta, empeoraría
//! exactamente el problema que pretende resolver. Como el horizonte ya está acotado a 840 meses,
//! `paths ≤ 2000` acota el trabajo entero por construcción, y `computed_in_ms` publica lo que
//! costó de verdad para que el presupuesto sea auditable en vez de una promesa.
//!
//! Memoria: `2 · paths · (horizonte+1) · 8` bytes de muestras — 25,7 MB en el extremo de 2 000
//! caminos × 840 meses. El semáforo de simulaciones (`heavy::run_projection_sim`, 2–8 permisos)
//! es lo que acota el pico agregado.

use crate::error::ApiError;
use crate::handlers::cash_buffer::ResolvedCashBuffer;
use crate::handlers::installation::require_installation_member;
use crate::handlers::person_view::LedgerView;
use crate::handlers::projection::{
    build_installation_projection_input, density_month_indices, engine_month_to_grid,
    jubilacion_civil, resolve_projection_context, serialize_decimal_as_f64, strategy_label,
};
use crate::handlers::session::require_session_user;
use crate::state::{AppState, BandsCacheKey, Density};
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use futurefin_engine_stochastic::{
    project_percentile_bands, seed_for, BufferInactiveReason, McConfig, McError, McOutcome,
    DEFAULT_PATHS, MAX_PATHS,
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

/// Caminos por defecto — el `DEFAULT_PATHS` del crate, re-exportado con nombre propio para que
/// los llamantes de la API (el KPI del Resumen, la tool MCP) no tengan que importar el crate
/// estocástico solo para nombrar un default.
pub(crate) const DEFAULT_BANDS_PATHS: u32 = DEFAULT_PATHS;

/// Techo de caminos por HTTP. 391 ms medidos a 840 meses (ver el doc del módulo).
pub(crate) const HTTP_MAX_PATHS: u32 = 2_000;

/// Techo de caminos por MCP y por el eje `monte_carlo` de `simulate_projection`. La mitad del de
/// HTTP **a propósito**: un agente en bucle es el llamante que más fácil satura el semáforo, y
/// `simulate_projection` es cache-neutral por diseño (cada what-if paga sus caminos enteros).
/// 1 000 caminos son 204 ms medidos, y la diferencia estadística con 2 000 es menor que el ancho
/// de la propia banda.
pub(crate) const MCP_MAX_PATHS: u32 = 1_000;

/// **El suelo del VERDE, en puntos porcentuales** (5.0.0, decisión V7 del owner): 100. Verde
/// significa **cero caminos que agotan la cartera**, y nada menos.
///
/// Hasta 5.0.0 era el `success_threshold_pct` del perfil (default 95). Se retiró: era un ajuste
/// que casi nadie tocaba y que, cuando se tocaba, movía el color sin mover el plan — un mando
/// para cambiar de opinión sobre el mismo resultado. Con el corte fijo, el color dice siempre lo
/// mismo y se puede comparar entre personas.
pub(crate) const VERDICT_GREEN_FLOOR_PCT: u32 = 100;

/// Distancia en PUNTOS PORCENTUALES por debajo del suelo verde en que el semáforo pasa de ámbar
/// a rojo (D28). Con el suelo en 100, ámbar es `[90, 100)` y rojo `< 90`.
pub(crate) const VERDICT_AMBER_MARGIN_PP: u32 = 10;

pub(crate) const VERDICT_GREEN: &str = "green";
pub(crate) const VERDICT_AMBER: &str = "amber";
pub(crate) const VERDICT_RED: &str = "red";

/// Decimales de los importes de las bandas. Dos, como las series de `/v1/history/series`: son
/// geometría de un fan chart salida de un `f64`, y publicar sus 17 dígitos significativos sería
/// precisión inventada sobre el percentil de una muestra.
pub(crate) const BANDS_VALUE_DP: u32 = 2;

/// Decimales de fracción de las probabilidades. **La misma política que `savings_rate`**
/// (`handlers/summary.rs::RATIO_DP`): 6 decimales de fracción = 4 de porcentaje. Con 2 000
/// caminos la resolución real del estimador es `1/2000 = 0,0005`, así que estos 6 decimales
/// sobran holgadamente — y son de PRESENTACIÓN: el redondeo se aplica al publicar, nunca a un
/// valor que alimente otra cuenta.
pub(crate) const BANDS_RATIO_DP: u32 = crate::handlers::summary::RATIO_DP;

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

#[derive(Debug, Deserialize)]
pub struct ProjectionBandsQuery {
    /// `mine` (default) o `household` — este último es 400 `household_bands_unavailable`.
    #[serde(default)]
    pub view: Option<String>,
    /// Caminos a sortear (1..=2000; default 500). Fuera de rango es 400, nunca un clamp.
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

/// Probabilidad ACUMULADA de haber agotado la cartera en un mes, cada cinco años desde la
/// jubilación efectiva (D28: «Probabilidad de agotar a los 70/75/80/85/90»).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DepletionProbabilityPoint {
    /// Mes en la MISMA rejilla que `points[].month_index`.
    pub month_index: u32,
    /// Años cumplidos en ese mes. `null` ⟺ el usuario no tiene fecha de nacimiento: la tabla se
    /// sigue publicando por meses, porque la cifra existe aunque no se pueda rotular con una edad.
    pub age: Option<u32>,
    /// Fracción de caminos con la cartera agotada **en ese mes o antes** (es acumulada, así que
    /// crece monótonamente). `0.12` = 12 de cada 100 escenarios.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub probability: Option<Decimal>,
}

/// Percentiles del MES de jubilación, cuando la jubilación la decide el cruce del líquido con el
/// objetivo. Con una estrategia por EDAD el objeto entero es `null`: ahí el mes es un dato del
/// plan, no una distribución, y publicar tres veces el mismo número sugeriría lo contrario.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RetirementMonthPercentiles {
    /// El 10 % de los mercados más favorables se jubilan **como muy tarde** este mes.
    pub p10: Option<u32>,
    pub p50: Option<u32>,
    /// Un `null` aquí NO es «no calculado»: es un percentil que cae sobre un camino que **no se
    /// jubila dentro del horizonte** (los caminos sin jubilación ordenan los últimos). `p90:
    /// null` se lee «uno de cada diez planes no llega nunca».
    pub p90: Option<u32>,
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
    /// Caminos efectivamente sorteados.
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
    /// **La definición de éxito: el plan OCURRE y AGUANTA.** Fracción de caminos en los que el
    /// hogar **se jubila dentro del horizonte** —o la estrategia es por EDAD, y entonces la
    /// jubilación es un dato del plan y no un suceso— **Y** la cartera no se agota nunca, con las
    /// pensiones y las fases ya dentro de la simulación. `0.87` = 87 de cada 100 escenarios.
    ///
    /// **Cambió en el pase de correcciones de la revisión adversarial** (hallazgo #7). D22 decía
    /// solo «la cartera no se agota nunca», y con un trigger por CRUCE eso premiaba al hogar que
    /// **no se jubila jamás**: quien trabaja hasta los 105 años sin llegar al objetivo nunca
    /// drena, así que nunca se agota. Medido sobre un hogar sintético: 0,960 publicados con el
    /// 33,1 % de los caminos sin jubilarse. Hoy ese mismo hogar publica 0,629, con
    /// `never_retired_probability = 0,331` y `success_given_retired = 0,940` al lado — tres
    /// cifras que se leen juntas, y ninguna de las tres es la que se publicaba antes.
    ///
    /// El RECORTE de una regla de retirada **no es fracaso** (D24) y viaja aparte, en
    /// `months_below_need_p50` / `withdrawal_to_need_ratio_p50`.
    ///
    /// **Identidad comprobable**: `success_probability ≤ 1 − never_retired_probability` con
    /// trigger por cruce (los caminos que no se jubilan no pueden contar como éxito); con trigger
    /// por edad `never_retired_probability` es `"0"` y la cota es trivial.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_probability: Option<Decimal>,
    /// **Fracción de caminos que NO se jubilan dentro del horizonte.** Con trigger por EDAD es
    /// `"0"` por construcción: ahí la jubilación llega llegue o no el capital.
    ///
    /// Se publica porque es el **denominador escondido** del éxito: un plan por cruce con una
    /// probabilidad de éxito alta y un tercio de caminos que no se jubilan nunca no es un buen
    /// plan, es un plan que no ocurre. `null` solo si el `f64` no fuera representable —
    /// inalcanzable con el cociente de dos contadores—, nunca para decir «cero».
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub never_retired_probability: Option<Decimal>,
    /// **Éxito entre los caminos que SÍ se jubilan**: de los que llegan a la jubilación, cuántos
    /// no agotan la cartera. `null` ⟺ ningún camino se jubila dentro del horizonte — ahí la
    /// pregunta «¿aguanta?» no tiene sobre qué formularse, y un `0` la respondería en falso.
    ///
    /// Junto a `success_probability` separa las dos preguntas que la definición vieja mezclaba:
    /// «¿ocurre el plan?» y «¿aguanta?». Un `success_given_retired` alto con un
    /// `never_retired_probability` alto describe un plan sólido que casi nunca llega a empezar.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub success_given_retired: Option<Decimal>,
    /// `green` | `amber` | `red` (D28, corte FIJO desde 5.0.0 — V7): **verde solo con
    /// `success_probability == 1`** (ni un camino agota la cartera), ámbar en `[0,90, 1)`, rojo
    /// por debajo de 0,90. El umbral configurable del perfil se retiró: el corte ya no depende de
    /// nada que el cliente tenga que leer, así que no hay nada que ecoar.
    pub success_verdict: &'static str,
    /// Probabilidad acumulada de agotamiento cada cinco años desde la jubilación efectiva.
    /// **Vacío** cuando ningún camino se jubila dentro del horizonte: sin jubilación no existe
    /// «la probabilidad de agotar a los 75».
    ///
    /// **La última fila es siempre el HORIZONTE** (hallazgo #8 de la revisión adversarial), o
    /// sea la ruina total del plan: «cuántos escenarios se quedan sin cartera antes de que se
    /// acabe el plan». La rejilla avanza de 60 en 60 desde el ancla y antes se paraba en el
    /// último múltiplo que cabía — con ancla en el mes 655 y horizonte 840 la tabla terminaba en
    /// el 835 y dejaba cinco meses fuera sin decirlo. La consecuencia visible es que la última
    /// fila puede estar a menos de cinco años de la anterior: el paso NO es uniforme al final.
    pub depletion_probability_by_age: Vec<DepletionProbabilityPoint>,
    /// Percentiles del mes de jubilación. `null` con trigger por EDAD (ver el tipo).
    pub retirement_month_index_percentiles: Option<RetirementMonthPercentiles>,
    /// **D17 en versión probabilística**: fracción de caminos que llegan a la edad objetivo con
    /// el líquido por debajo del objetivo. `null` ⟺ el plan no se jubila por edad — la pregunta
    /// no existe con un trigger por cruce.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub underfunded_probability: Option<Decimal>,
    /// Mediana entre caminos del **NÚMERO DE MESES jubilados en que el hogar no cubrió su
    /// gasto**: cuenta el recorte de la regla (`withdrawal_shortfall > 0`) **y** el gasto que la
    /// cartera no pudo financiar (`unmet_need > 0`).
    ///
    /// Contar solo el recorte lo dejaba en `0` por construcción con `fixed_real` —la regla sin
    /// techo no recorta nunca—, también en los caminos que se quedaban sin cartera en el mes 35
    /// de 400: el mes sin dinero no aparecía en ninguna cifra publicada.
    pub months_below_need_p50: u32,
    /// Mediana entre caminos de `Σ retirada / Σ (retirada + recorte + descubierto)` sobre los
    /// meses jubilados: **qué FRACCIÓN de su gasto cubrió el hogar de verdad**. `1` = lo cubrió
    /// entero. `null` cuando ningún camino tiene meses jubilados con necesidad positiva.
    ///
    /// **El descubierto entra en el denominador** (hallazgo #4 de la revisión adversarial). Con
    /// `fixed_real` el recorte es cero por construcción —el permitido ES la necesidad—, así que
    /// el cociente valía `1,0` SIEMPRE, también sobre caminos que se quedaban sin cartera en el
    /// mes 35 de 400. El hogar sintético medido publica hoy `0.086500` donde antes publicaba
    /// `1.000000`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub withdrawal_to_need_ratio_p50: Option<Decimal>,
    /// `false` ⟺ **ningún activo declara volatilidad**: entonces las tres bandas coinciden entre
    /// sí y con la línea determinista, y la UI debe decirlo («sin volatilidad declarada: la banda
    /// es la línea») en vez de dibujar un abanico plano que se lee como certeza.
    pub any_volatility_declared: bool,
    /// **P4 (§B.6): ¿se SIMULÓ el colchón de caja?** `true` solo con las tres condiciones a la
    /// vez: `cash_buffer_months` declarado en el perfil, un activo líquido que pueda albergarlo y
    /// volatilidad declarada de la que protegerse. Un `false` con el colchón configurado **no es
    /// un fallo**: es que en esta cartera no significa nada y el resultado es idéntico al de no
    /// pedirlo — decirlo es lo que impide leer «no pasó nada» como «no funcionó».
    ///
    /// **Solo vive en Monte Carlo.** El camino determinista que la app publica como dinero no
    /// tiene colchón: sin sorteo no hay mes bueno ni malo que distinguir, así que el trasvase no
    /// tendría criterio.
    pub buffer_active: bool,
    /// **De dónde sale el colchón** (5.0.0, V6): `explicit` (el perfil —o el `profile_overrides`
    /// del what-if— declara `cash_buffer_months`; una elección no se deriva) | `allocation_cap`
    /// (se DERIVA del tope de tu regla de ahorro: el importe que la cascada persigue mientras
    /// ahorras es el que el colchón mantiene jubilado) | `none` (no hay colchón;
    /// `buffer_inactive_reason` dice por qué).
    ///
    /// Se publica porque un colchón derivado que no dijera de dónde sale sería un número que el
    /// usuario no pidió y no puede cambiar.
    pub buffer_source: &'static str,
    /// **El objetivo del colchón en euros NOMINALES**, y nominales de verdad: no se indexa nunca,
    /// igual que el tope de la regla del que sale (P2). `null` salvo con
    /// `buffer_source: "allocation_cap"` — con un colchón en meses el objetivo se re-dimensiona
    /// cada mes contra el gasto ya indexado, y publicar un escalar sería publicar una sola de sus
    /// caras. Se publica con la escala de la casa (`money_out`, 4 decimales), como todo importe
    /// escalar del API.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub buffer_target_amount: Option<Decimal>,
    /// **Meses de gasto que cubre el colchón.** Con `explicit`, los que el usuario escribió (y
    /// son los que el motor usa). Con `allocation_cap`, el equivalente **informativo**
    /// `floor(tope / gasto de jubilación mensual de hoy)`: el motor persigue el IMPORTE, no estos
    /// meses. `null` sin colchón, y también cuando el gasto de jubilación no es positivo.
    pub buffer_months_effective: Option<u32>,
    /// La regla de asignación cuyo tope fijó el objetivo. `null` salvo con `allocation_cap`. Es
    /// lo que permite a la UI enlazar «cámbialo en tu regla de ahorro» en vez de describirlo.
    #[schema(value_type = Option<String>, format = "uuid")]
    pub buffer_source_rule_id: Option<Uuid>,
    /// El activo que HACE de colchón: el líquido con σ = 0 de menor rentabilidad, el mismo que
    /// elige el motor (`safe_cash_buffer_index`). Se publica también con `explicit` y también con
    /// `cap_is_zero`/`no_capped_rule` —ahí el activo existe, lo que falta es el importe—; `null`
    /// solo cuando no hay ningún líquido sin riesgo.
    pub buffer_source_asset_name: Option<String>,
    /// **Por qué NO se simuló el colchón.** UN solo campo con motivos de dos capas:
    ///
    /// - Del **handler** (5.0.0): `no_capped_rule` (ninguna regla habilitada y con tope apunta al
    ///   líquido sin riesgo — el caso de la pauta «todo al fondo», donde ese líquido es el
    ///   sumidero sin tope) | `cap_is_zero` (la hay, pero su techo resuelto es 0 €) |
    ///   `no_safe_liquid_asset` (no hay ningún activo líquido con σ = 0 donde alojarlo).
    /// - Del **motor**: `no_volatility` (ningún activo declara volatilidad: no hay riesgo de
    ///   secuencia del que protegerse, y el resultado es BIT A BIT el de no pedirlo).
    ///
    /// El `not_requested` del motor **ya no se publica**: desde que el colchón se deriva, «no se
    /// pidió» no es un motivo — el motivo es cuál de las condiciones de la derivación falló.
    /// `null` ⟺ `buffer_active: true`, y nunca es `null` con `buffer_active: false`.
    #[schema(value_type = Option<String>)]
    pub buffer_inactive_reason: Option<&'static str>,
    /// Mediana entre caminos del NÚMERO de meses con relleno efectivo. **`null` ⟺
    /// `buffer_active: false`** — «no se midió», que no es «cero rellenos».
    pub buffer_refills_p50: Option<u32>,
    /// Mediana entre caminos del **total movido al colchón** en todo el horizonte.
    ///
    /// Va en euros y aun así **no es un KPI monetario del hogar**: es la mediana de un total
    /// sobre una muestra sorteada, o sea un estadístico de la dispersión, y la regla del crate
    /// estocástico («de aquí no sale un euro») sigue en pie — el trasvase del camino que la app
    /// dibuja lo da el motor `Decimal`. Se publica a 2 decimales, como los valores de las bandas
    /// y por el mismo motivo. `null` ⟺ `buffer_active: false`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub buffer_refill_net_total_p50: Option<Decimal>,
    /// Estrategia con la que se simuló: `asap` | `retire_at_age` | `coast` | `partial` |
    /// `pension_bridge`. Mismo literal que `GET /v1/projection/series`.
    pub strategy: String,
    /// `liquid_crossing` | `target_age`. Explica por qué
    /// `retirement_month_index_percentiles` y `underfunded_probability` son excluyentes.
    pub retirement_trigger: &'static str,
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
pub(crate) const BANDS_MODEL_NOTE: &str = "Monte Carlo sobre el MISMO bucle que la línea determinista, con los factores de crecimiento sorteados. MODELO: un shock de mercado COMÚN por mes (un solo z~N(0,1) que viven todos los activos a la vez), escalado por la volatilidad de cada uno: factor = m·exp(σz − σ²/2) con σ = annual_volatility_percent/100/√12. La corrección de Itô es exacta, así que la media del factor mensual ES el factor determinista y la rentabilidad esperada que escribiste se respeta; la GEOMÉTRICA —la que se cobra— sale más baja, y esa diferencia no es un error: es el coste de la volatilidad, y se ve en que la banda p50 queda por DEBAJO de la línea determinista. σ = 0 ⇒ el camino determinista exacto. PUNTUAL QUIERE DECIR PUNTUAL: cada percentil se calcula mes a mes sobre los caminos de ESE mes, así que la curva p50 NO es una simulación real y no cumple ninguna identidad contable — no la cites como «tu patrimonio probable» punto a punto. SEMILLA estable por usuario: las mismas cifras hoy y dentro de un año, salvo que cambies los datos o pases `seed`. ÉXITO = el plan OCURRE y AGUANTA: el hogar se jubila dentro del horizonte (o la estrategia es por EDAD, y entonces la jubilación es un dato) Y la cartera no se agota nunca, con pensiones y fases dentro de la simulación. Con un trigger por CRUCE, la definición vieja —solo «no se agota»— premiaba al hogar que NO se jubila jamás: quien nunca llega al objetivo nunca drena. Por eso viajan al lado `never_retired_probability` (cuántos caminos no se jubilan; 0 por construcción con trigger por edad) y `success_given_retired` (éxito entre los que sí se jubilan): las tres se leen juntas. El RECORTE de una regla de retirada no es fracaso y viaja aparte, en `months_below_need_p50` y `withdrawal_to_need_ratio_p50` — que cuentan el recorte de la regla Y el gasto que la cartera no pudo financiar, porque con `fixed_real` el recorte es cero por construcción y un cociente que solo lo mirara valdría 1,0 también en los caminos que se quedan sin cartera. `depletion_probability_by_age` cierra SIEMPRE en el horizonte: esa última fila es la ruina total del plan, y el paso hasta ella puede ser menor de cinco años. `buffer_inactive_reason` dice por qué no se simuló el colchón cuando `buffer_active` es false. LO QUE NO SE MODELA, dicho para que nadie lo suponga: colas gruesas (el shock es log-normal, así que la probabilidad de ruina es OPTIMISTA en la cola), autocorrelación o reversión a la media (los meses son independientes: sin ciclos), correlación imperfecta entre activos (con un shock común la correlación es exactamente 1 y una cartera diversificada NO se beneficia aquí de su diversificación: el modelo es conservador en ese eje), bootstrap histórico (el sorteo es paramétrico: nada de esto es «lo que pasó entre 1929 y 1964»), volatilidad de la inflación, de los ingresos, del gasto o del tipo de la deuda (solo los activos sortean), y rebalanceo (cada activo compone por su cuenta). El patrimonio, el objetivo y la aportación necesaria en EUROS siguen saliendo del camino exacto en Decimal: de aquí solo salen probabilidades y percentiles.";

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
/// que `months`): devolver 200 con 2 000 caminos a quien pidió 10 000 es contestar otra pregunta.
pub(crate) fn resolve_paths(paths: Option<u32>, max: u32) -> Result<u32, ApiError> {
    let p = paths.unwrap_or(DEFAULT_PATHS);
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
    let response = Arc::new(compute_projection_bands(state, user_id, iid, paths, seed).await?);
    state.bands_cache_insert(key, response.clone()).await;
    Ok(response)
}

/// Calcula las bandas sin tocar el cache.
///
/// **El ensamblado es el MISMO** que el de la serie (`build_installation_projection_input` con
/// `LedgerView::Mine`, el perfil resuelto del usuario y su fecha de nacimiento): si las bandas
/// salieran de un input propio, la línea determinista y su abanico podrían describir dos planes
/// distintos sin que nada lo dijera. Las volatilidades viajan en el vector paralelo que ese
/// mismo ensamblado construye activo a activo.
async fn compute_projection_bands(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    paths: u32,
    seed: u64,
) -> Result<ProjectionBandsResponse, ApiError> {
    let ctx = resolve_projection_context(&state.pool, iid, user_id, None).await?;
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
        // P4/V6: el colchón lo resolvió el ENSAMBLADO (`resolve_cash_buffer`), no este handler:
        // sale del tope de la regla de ahorro salvo que el perfil declare uno explícito, y el
        // what-if lee exactamente el mismo campo para que banda y simulación no describan dos
        // colchones distintos. Que se simule de verdad depende además de la cartera (un líquido
        // sin riesgo que lo albergue y volatilidad de la que protegerse): lo responde
        // `buffer_active` en la salida.
        cash_buffer: built.cash_buffer.spec,
    };

    // Bajo el MISMO semáforo que las proyecciones (`heavy::run_projection_sim`): el recurso
    // escaso es el mismo —núcleos— y este llamante es el más caro de todos, así que dejarlo
    // fuera del techo habría reabierto el agujero que el semáforo cerró.
    let mc_input = built.input.clone();
    let t0 = std::time::Instant::now();
    let outcome = crate::heavy::run_projection_sim("monte carlo bands", move || {
        project_percentile_bands(&mc_input, &volatilities, &config)
    })
    .await?
    .map_err(map_mc_err)?;
    let computed_in_ms = t0.elapsed().as_millis() as u64;

    Ok(assemble_bands_response(
        &outcome,
        ctx.months,
        ctx.horizon_basis,
        ctx.today,
        ctx.session_birth_date,
        strategy_label(ctx.retirement_profile.strategy),
        built.retirement_trigger,
        &built.cash_buffer,
        computed_in_ms,
    ))
}

/// Salida del motor estocástico → respuesta publicada. Función aparte, y sin I/O, porque es
/// donde viven las DOS traducciones que se pueden equivocar en silencio: los meses del bucle a
/// la rejilla 0-based (`engine_month_to_grid`) y las probabilidades `f64` a `Decimal`.
#[allow(clippy::too_many_arguments)]
fn assemble_bands_response(
    outcome: &McOutcome,
    months: u32,
    horizon_basis: String,
    today: chrono::NaiveDate,
    birth_date: Option<chrono::NaiveDate>,
    strategy: String,
    retirement_trigger: &'static str,
    cash_buffer: &ResolvedCashBuffer,
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
    // significativos en el JSON — precisión INVENTADA para el percentil de una muestra de 500
    // caminos, y ~40 % más de payload por punto. La resolución real de estas cifras es el ancho
    // de la banda, no el céntimo.
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

    let depletion_probability_by_age: Vec<DepletionProbabilityPoint> = outcome
        .depletion_probability_by_age
        .iter()
        .map(|&(engine_month, p)| {
            // El motor cuenta meses 1-based; la respuesta habla la rejilla de `points[]`.
            let month_index = engine_month_to_grid(Some(engine_month)).unwrap_or(0);
            let (_, age) = jubilacion_civil(today, birth_date, Some(month_index));
            DepletionProbabilityPoint {
                month_index,
                age,
                probability: probability_out(p),
            }
        })
        .collect();

    let retirement_month_index_percentiles =
        outcome
            .retirement_month_index_percentiles
            .as_ref()
            .map(|v| RetirementMonthPercentiles {
                p10: v.first().copied().flatten().and_then(|m| engine_month_to_grid(Some(m))),
                p50: v.get(1).copied().flatten().and_then(|m| engine_month_to_grid(Some(m))),
                p90: v.get(2).copied().flatten().and_then(|m| engine_month_to_grid(Some(m))),
            });

    ProjectionBandsResponse {
        view: LedgerView::Mine.as_str(),
        months,
        horizon_basis,
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        paths: outcome.paths,
        seed: outcome.seed.to_string(),
        percentiles: outcome.percentiles.clone(),
        points,
        success_probability: probability_out(outcome.success_probability),
        never_retired_probability: probability_out(outcome.never_retired_probability),
        success_given_retired: outcome.success_given_retired.and_then(probability_out),
        success_verdict: success_verdict(outcome.success_probability),
        depletion_probability_by_age,
        retirement_month_index_percentiles,
        underfunded_probability: outcome.underfunded_probability.and_then(probability_out),
        months_below_need_p50: outcome.months_below_need_p50,
        withdrawal_to_need_ratio_p50: outcome
            .withdrawal_to_need_ratio_p50
            .and_then(probability_out),
        any_volatility_declared: outcome.any_volatility_declared,
        buffer_active: outcome.buffer_active,
        buffer_source: cash_buffer.source,
        buffer_target_amount: cash_buffer.target_amount.map(crate::money::money_out),
        buffer_months_effective: cash_buffer.months_effective,
        buffer_source_rule_id: cash_buffer.source_rule_id,
        buffer_source_asset_name: cash_buffer.source_asset_name.clone(),
        buffer_inactive_reason: merge_buffer_inactive_reason(
            outcome.buffer_inactive_reason,
            cash_buffer,
        ),
        buffer_refills_p50: outcome.buffer_refills_p50,
        buffer_refill_net_total_p50: outcome
            .buffer_refill_net_total_p50
            .and_then(Decimal::from_f64_retain)
            .map(|d| d.round_dp(BANDS_VALUE_DP)),
        strategy,
        retirement_trigger,
        computed_in_ms,
        model_note: BANDS_MODEL_NOTE.into(),
    }
}

/// **UN solo `buffer_inactive_reason`, con motivos de dos capas** (5.0.0, V6).
///
/// El motor solo sabe tres cosas: no se pidió, no hay volatilidad, no hay líquido sin riesgo. Con
/// el colchón DERIVADO, «no se pidió» dejó de ser un motivo publicable: si no hay colchón es
/// porque falló una condición de la derivación, y ésa es la que el usuario necesita leer
/// (`no_capped_rule`, `cap_is_zero`, `no_safe_liquid_asset`). Así que el `not_requested` del
/// motor se sustituye por el motivo del handler; el resto pasa TAL CUAL, con el literal que pone
/// el propio crate (`BufferInactiveReason::code`) y no un `match` duplicado aquí.
///
/// El `unwrap_or` final es inalcanzable por construcción —`spec: None` ⟺ el handler puso motivo—
/// y existe para que un futuro camino nuevo degrade a un motivo honesto en vez de a `null`, que
/// se leería como «el colchón sí se simuló».
pub(crate) fn merge_buffer_inactive_reason(
    engine_reason: Option<BufferInactiveReason>,
    cash_buffer: &ResolvedCashBuffer,
) -> Option<&'static str> {
    match engine_reason {
        Some(BufferInactiveReason::NotRequested) => Some(
            cash_buffer
                .inactive_reason
                .unwrap_or(crate::handlers::cash_buffer::BUFFER_INACTIVE_NO_CAPPED_RULE),
        ),
        Some(r) => Some(r.code()),
        None => None,
    }
}

/// **El semáforo de D28 con el corte FIJO al 100 %** (5.0.0, V7).
///
/// La comparación se hace en PUNTOS PORCENTUALES enteros y no en fracciones —la probabilidad se
/// multiplica por 100, no el suelo se divide entre él— por la misma razón que cuando el umbral
/// era configurable: dividir entre 100 introduce un binario no representable justo en el borde.
///
/// **El verde es exacto y se puede confiar en él**: la probabilidad es `n/n` con `n` caminos
/// enteros, y en IEEE 754 esa división da `1.0` exactamente para cualquier `n` (numerador y
/// denominador son el mismo entero, y el cociente es representable). Por eso `p == 1.0` no
/// necesita épsilon, y por eso `p·100 >= 100` es la misma condición. Lo pinea
/// `el_verde_exige_todos_los_caminos`.
///
/// Consecuencia asumida (V7): con 500 caminos, **un solo fallo es ámbar**. Es deliberado — el
/// copy del tile verde dice «0 de 500 escenarios agotan el capital» y eso solo puede afirmarse
/// cuando es verdad.
pub(crate) fn success_verdict(success_probability: f64) -> &'static str {
    let pct = success_probability * 100.0;
    let green_floor = f64::from(VERDICT_GREEN_FLOOR_PCT);
    let amber_floor = green_floor - f64::from(VERDICT_AMBER_MARGIN_PP);
    if pct >= green_floor {
        VERDICT_GREEN
    } else if pct >= amber_floor {
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
        ("paths" = Option<u32>, Query, description = "Caminos de Monte Carlo, 1..=2000 (default 500). Fuera de rango → 400 `paths_out_of_range`."),
        ("seed" = Option<String>, Query, description = "Semilla de 64 bits en dígitos decimales. Omitida = la estable del usuario (D23)."),
    ),
    responses(
        (status = 200, description = "Bandas puntuales p10/p50/p90 de patrimonio y líquido (densidad hybrid), probabilidad de éxito con su veredicto, agotamiento por edad y las lecturas del recorte.", body = ProjectionBandsResponse),
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
    let response =
        projection_bands_cached(&state, user.id.0, iid, view, paths, seed).await?;
    Ok(Json((*response).clone()))
}

pub fn projection_bands_router() -> Router {
    Router::new().route("/bands", get(get_projection_bands))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **El verde exige TODOS los caminos** (V7), y el borde es exacto sin épsilon.
    ///
    /// La aserción que importa es la última: `n/n` en `f64` es `1.0` exactamente para cualquier
    /// número de caminos, así que «verde ⟺ ni un camino agota la cartera» no es una aproximación
    /// que el redondeo pueda romper con 499 o 2 000 caminos. Lo contrario —un `0.9999…` que se
    /// colara como verde— sería el único fallo silencioso posible aquí.
    #[test]
    fn el_verde_exige_todos_los_caminos() {
        assert_eq!(success_verdict(1.0), VERDICT_GREEN);
        // Un solo fallo entre 500 ya NO es verde: es la consecuencia asumida de V7.
        assert_eq!(success_verdict(499.0 / 500.0), VERDICT_AMBER);
        assert_eq!(success_verdict(0.999), VERDICT_AMBER);
        assert_eq!(success_verdict(0.95), VERDICT_AMBER);
        assert_eq!(success_verdict(0.90), VERDICT_AMBER);
        assert_eq!(success_verdict(0.8999), VERDICT_RED);
        assert_eq!(success_verdict(0.0), VERDICT_RED);

        // El borde exacto, camino a camino: `n/n == 1.0` en IEEE 754 para todo `n` razonable, así
        // que el verde no necesita tolerancia y `(n−1)/n` nunca se cuela.
        for n in [1u32, 7, 24, 499, 500, 1_000, 2_000] {
            let all = f64::from(n) / f64::from(n);
            assert_eq!(all, 1.0, "n/n debe ser exactamente 1 con n = {n}");
            assert_eq!(success_verdict(all), VERDICT_GREEN, "n = {n}");
            if n > 1 {
                let one_short = f64::from(n - 1) / f64::from(n);
                assert_ne!(
                    success_verdict(one_short),
                    VERDICT_GREEN,
                    "un camino fallido no puede ser verde con n = {n}"
                );
            }
        }
    }

    /// `paths` se rechaza fuera de rango, nunca se clampa, y cada superficie trae su techo.
    #[test]
    fn los_caminos_se_rechazan_fuera_de_rango() {
        assert_eq!(resolve_paths(None, HTTP_MAX_PATHS).unwrap(), DEFAULT_PATHS);
        assert_eq!(resolve_paths(Some(1), HTTP_MAX_PATHS).unwrap(), 1);
        assert_eq!(resolve_paths(Some(2000), HTTP_MAX_PATHS).unwrap(), 2000);
        assert!(resolve_paths(Some(0), HTTP_MAX_PATHS).is_err());
        assert!(resolve_paths(Some(2001), HTTP_MAX_PATHS).is_err());
        // El techo del MCP es la mitad: 2 000 es válido por HTTP y 400 por MCP.
        assert!(resolve_paths(Some(2000), MCP_MAX_PATHS).is_err());
        assert_eq!(resolve_paths(Some(1000), MCP_MAX_PATHS).unwrap(), 1000);
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
