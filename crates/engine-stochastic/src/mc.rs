//! **La capa de Monte Carlo** (WP6 de 5.0.0, §B.5/§B.6 del plan de la issue #207, D11/D22/D23/D25).
//!
//! # El modelo de retornos, entero y sin letra pequeña
//!
//! **La rentabilidad que el usuario declara en un activo es COMPUESTA (CAGR)**: la tasa de
//! crecimiento geométrico, la que publican los fondos y la que un hogar cobra de verdad (decisión
//! M8 del modelo v2 de jubilación, owner, 2026-09-06). El camino determinista la compone tal cual
//! —`m_i` es la raíz doceava del motor— y el sorteo se construye **alrededor** de esa línea: la
//! línea determinista es la CENTRAL, no un techo ni un suelo.
//!
//! Un **shock de mercado COMÚN** por mes (D11). Para cada mes `k` del horizonte se sortea **un
//! solo** normal estándar `z_k ~ N(0,1)` y todos los activos lo viven a la vez, escalado por su
//! propia volatilidad:
//!
//! ```text
//!   σ_i  = annual_volatility_percent_i / 100 / √12          (volatilidad MENSUAL del activo i)
//!   d_i  = m_i · exp(σ_i²/2)                                (DERIVA: la media aritmética)
//!   f_ik = d_i · exp(σ_i·z_k − σ_i²/2)   ( = m_i · exp(σ_i·z_k) )
//!   σ_i = 0  ⇒  d_i = m_i  y  f_ik = m_i  exactamente
//! ```
//!
//! `m_i` es el multiplicador determinista del activo — la **raíz doceava del motor**
//! ([`futurefin_engine::monthly_growth_multiplier`]), no una segunda copia escrita aquí: si esa
//! conversión anual→mensual cambiara, «volatilidad cero» dejaría de significar «el camino
//! determinista» sin que nada fallara.
//!
//! **La conversión CAGR → media aritmética vive en UN SOLO SITIO**, `PathEngine::new`, y se aplica
//! sobre la σ **MENSUAL**. Sumarla a la tasa ANUAL que el usuario escribió (`CAGR + σ_anual²/2`,
//! como porcentajes) es la fórmula equivocada: la prima de varianza pertenece al factor que se
//! sortea cada mes, y `σ_anual²/2` es 12 veces mayor que la corrección que ese factor necesita.
//!
//! El término `−σ_i²/2` es la corrección de Itô. `f_ik` se escribe con la deriva DENTRO y la
//! corrección FUERA —en vez de simplificarlo a `m_i·exp(σ_i·z_k)`, que es lo mismo— porque así las
//! dos propiedades que cargan peso se leen sin despejar nada: para `X ~ N(0, σ²)`,
//! `E[exp(X)] = exp(σ²/2)` y `mediana(exp(X)) = 1`, luego
//!
//! ```text
//!   E[f_ik]       = d_i · exp(σ_i²/2) · exp(−σ_i²/2) = d_i = m_i · exp(σ_i²/2)
//!   mediana(f_ik) = d_i · exp(−σ_i²/2)               = m_i
//! ```
//!
//! Es decir: **la MEDIANA del factor mensual es el factor determinista**, y la media aritmética
//! queda por ENCIMA en `exp(σ_m²/2)` — la **prima de varianza**, que es exactamente lo que hay que
//! pagarle a la volatilidad para que la geométrica siga siendo la declarada. Con `σ_anual = 15 %`:
//! `σ_m² = 0,001875`, `exp(σ_m²/2) = 1,000938` al mes ⇒ **+11,9 % de media** al cabo de 10 años
//! sobre la línea determinista, que sigue siendo la mediana.
//!
//! **Lo que esto SÍ garantiza y lo que NO — dicho aquí para que nadie prometa de más.** Sin flujos,
//! la mediana del patrimonio terminal ES la línea determinista y la igualdad es exacta
//! (`Π_k f = m^H · exp(σ·Σz)` y la mediana de esa log-normal es `m^H`): lo mide
//! `mc_median_is_the_deterministic_line`. **Con aportaciones o retiradas la igualdad deja de ser
//! exacta**: la cascada, el drenaje y la fiscalidad son funciones NO lineales del camino, y la
//! mediana del patrimonio se separa de la línea determinista **unos pocos puntos porcentuales**
//! (medido por el panel adversarial del modelo v2: **±2–4 % a 20–35 años**, con el signo según el
//! hogar esté aportando o retirando). La banda p50 del chart es por tanto una lectura MUY próxima
//! a la línea determinista, no una identidad contable — y eso es lo que la ayuda de la UI tiene
//! que decir, en vez de prometer una coincidencia que solo se cumple en el laboratorio sin flujos.
//!
//! # Lo que este modelo NO representa — dicho aquí para que nadie lo suponga
//!
//! - **Colas gruesas.** El shock es log-normal. Los mercados reales tienen curtosis: octubre de
//!   1987 fue −20 % en un día, unas 20 desviaciones típicas bajo este modelo (probabilidad ≈ 0).
//!   La probabilidad de ruina que sale de aquí es, por construcción, **optimista en la cola**.
//! - **Autocorrelación / reversión a la media.** Los `z_k` son independientes mes a mes. Ni hay
//!   momentum ni hay reversión, y por tanto **no hay ciclos**: la dispersión a 35 años crece con
//!   `√H` limpia. La evidencia histórica apunta a algo de reversión a largo plazo, que ESTRECHARÍA
//!   las bandas lejanas.
//! - **Correlación imperfecta entre activos.** Con un único `z` por mes, la correlación entre dos
//!   activos con `σ > 0` es **exactamente 1** (sus log-retornos son múltiplos del mismo número).
//!   Es la decisión D11, y su consecuencia hay que decirla: una cartera «diversificada» de RV
//!   global + RF + cripto **no se beneficia aquí de la diversificación**; la banda es tan ancha
//!   como la de una cartera de un solo activo con la volatilidad ponderada. El modelo es
//!   CONSERVADOR en ese eje y optimista en el de las colas.
//! - **Bootstrap histórico / secuencias reales.** No se remuestrea ninguna serie histórica: el
//!   sorteo es paramétrico. Nada de lo que sale de aquí es «lo que pasó entre 1929 y 1964».
//! - **Volatilidad de la inflación, de los ingresos, del gasto o de los tipos de la deuda.**
//!   Solo el crecimiento de los activos es estocástico. El IPC, la nómina, el presupuesto y el
//!   TIN de la hipoteca siguen siendo exactamente los del camino determinista.
//! - **Rebalanceo.** No lo hay: cada activo compone por su cuenta y la cascada reparte el
//!   superávit con las reglas declaradas, igual que en el camino determinista. No hay ninguna
//!   recolocación entre activos.
//!
//! # El colchón de caja — retirado antes de publicarse
//!
//! Esta capa llevó, durante el WP6 de 5.0.0 (P4, §B.6 del plan de #207), un mecanismo de colchón
//! de caja: un activo líquido absorbía la retirada y se rellenaba vendiendo del resto de la
//! cartera en los meses de shock positivo. La decisión del propietario (2026-09-06) lo retiró
//! ENTERO antes de que 5.0.0 se publicara: la caja es un activo más, y son las reglas de ahorro
//! —no un mecanismo aparte— las que deciden cuánto se guarda. No queda ni un tipo, ni un campo, ni
//! un test suyo en este crate ni en `crates/engine`.
//!
//! # De aquí no sale un euro
//!
//! Es la regla del crate (ver el doc de [`crate`]) y aquí es donde muerde: [`McOutcome`] son
//! **probabilidades, percentiles y contadores**. Ninguna de sus cifras se publica como un KPI
//! monetario. Las bandas se dibujan; el patrimonio, el objetivo y la aportación necesaria siguen
//! saliendo del camino `Decimal`.

use rand_chacha::rand_core::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;

use futurefin_engine::{
    monthly_growth_multiplier, simulate, EngineError, PathFailure, ProjectionInput,
    RetirementTrigger, SimInput, SimOutput,
};

use crate::{
    F64Money, SuccessAt, KIND_INITIAL_RATE_EXCEEDED, KIND_PORTFOLIO_DEPLETED, KIND_RULE_BELOW_NEED,
};

// =================================================================================================
// Configuración
// =================================================================================================

/// Cota dura de caminos por ejecución. **No es un ajuste de producto**: es el punto a partir del
/// cual la memoria de las bandas (`2 · caminos · (horizonte+1) · 8 bytes`, ver
/// [`project_percentile_bands`]) y el tiempo dejan de caber en el presupuesto de un request. Con
/// 5 000 caminos y 840 meses son ~67 MB y varios segundos. Los topes de producto por transporte
/// (`HTTP_MAX_PATHS` y `MCP_MAX_PATHS` en `apps/api/src/handlers/projection_bands.rs`) los aplica el
/// handler, no este crate, y son ≤ esta cota.
pub const MAX_PATHS: u32 = 5_000;

/// Caminos por defecto (§B.5 del plan).
pub const DEFAULT_PATHS: u32 = 500;

/// Percentiles por defecto: la banda p10/p50/p90 que la sección «Riesgo» dibuja (D28).
pub const DEFAULT_PERCENTILES: [u8; 3] = [10, 50, 90];

/// Cada cuántos meses se publica el FALLO acumulado desde el mes de jubilación forzado (§B.5:
/// «cada 5 años»). El caller traduce meses a edades — este crate no sabe de fechas de nacimiento.
///
/// Renombrada desde `DEPLETION_STEP_MONTHS` en E9 (modelo v2, McOutcome v2): la tabla ya no mide
/// solo agotamiento de cartera (F1), mide CUALQUIER fallo del camino (F1 cartera agotada / F2
/// tasa inicial excedida / F3 la regla no llega a la necesidad). El valor —60— no cambia.
pub const FAILURE_STEP_MONTHS: u32 = 60;

/// **La configuración de una ejecución de Monte Carlo.**
///
/// `seed` y `paths` son parte de la ENTRADA, no del entorno: la misma configuración sobre los
/// mismos datos produce las mismas bandas, en cualquier máquina y en cualquier orden. Es la
/// diferencia entre una herramienta en la que se puede confiar y una que cambia de número al
/// refrescar la página.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McConfig {
    /// La semilla del sorteo. Para un usuario, [`seed_for`] la deriva de sus identificadores
    /// (D23) para que las realizaciones de mercado no cambien porque haya editado un activo.
    pub seed: u64,
    /// Número de caminos, `1..=`[`MAX_PATHS`].
    pub paths: u32,
    /// Percentiles a publicar, cada uno en `1..=99`. **Se respeta el orden dado** y se permite
    /// repetir: las bandas salen en las mismas posiciones que este vector.
    pub percentiles: Vec<u8>,
    /// **Hilos con los que repartir los caminos** (E12). `None` = el pool compartido del crate
    /// ([`crate::parallel::pool_threads`]); `Some(1)` = secuencial en el hilo llamante.
    ///
    /// **Es el ÚNICO campo de esta estructura que no cambia ni un bit del resultado**, y por eso
    /// se escribe aquí en vez de en un parámetro suelto: viaja con la ejecución, se puede fijar en
    /// un test, y su presencia en `PartialEq` dice la verdad —dos configuraciones que reparten el
    /// trabajo distinto SON dos configuraciones distintas—, aunque produzcan el mismo
    /// [`McOutcome`] bit a bit. Que lo produzcan es un contrato, no una casualidad: los caminos
    /// son independientes (`path_rng` depende solo de `(seed, path_index)`) y el pliegue se hace
    /// SIEMPRE en orden de índice de camino. Ver [`crate::parallel`] y
    /// `tests/parallel_determinism.rs`.
    ///
    /// La API **no lo rellena**: en producción manda el pool compartido, que es lo que mantiene
    /// acotado el total de CPU cuando hay varias simulaciones en vuelo bajo el semáforo de
    /// `heavy.rs`.
    pub threads: Option<usize>,
}

impl Default for McConfig {
    fn default() -> Self {
        McConfig {
            seed: 0,
            paths: DEFAULT_PATHS,
            percentiles: DEFAULT_PERCENTILES.to_vec(),
            threads: None,
        }
    }
}

/// Lo que puede salir mal antes de sortear nada.
///
/// Tipo PROPIO y no una variante nueva de `EngineError`: los errores de configuración de Monte
/// Carlo son de este crate, y `crates/engine` no tiene por qué crecer un enum por una capa que
/// vive fuera.
///
/// Sin `Clone` porque `EngineError` no lo es (lleva `thiserror` y datos por valor) — y añadírselo
/// sería tocar `crates/engine`, que en este WP está fuera de alcance.
#[derive(Debug, PartialEq, Eq)]
pub enum McError {
    /// `paths` fuera de `1..=`[`MAX_PATHS`].
    InvalidPaths(u32),
    /// Un percentil fuera de `1..=99`, o la lista vacía.
    InvalidPercentiles,
    /// El vector de volatilidades no está alineado con `input.assets`: `(dadas, esperadas)`.
    ///
    /// **Falla en vez de rellenar con ceros**: una volatilidad que se pierde por el camino
    /// produce bandas estrechas y creíbles, que es el peor fallo posible aquí.
    VolatilityLengthMismatch(usize, usize),
    /// El motor falló en un camino. **Un camino que falla tumba la ejecución entera**: descartarlo
    /// sesgaría la probabilidad de éxito hacia arriba justo en los escenarios extremos, que son
    /// los únicos que pueden hacer fallar al motor.
    Engine(EngineError),
}

impl core::fmt::Display for McError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            McError::InvalidPaths(n) => {
                write!(f, "invalid_paths: {n} fuera de 1..={MAX_PATHS}")
            }
            McError::InvalidPercentiles => {
                write!(
                    f,
                    "invalid_percentiles: lista vacía o algún valor fuera de 1..=99"
                )
            }
            McError::VolatilityLengthMismatch(got, want) => write!(
                f,
                "volatility_length_mismatch: {got} volatilidades para {want} activos"
            ),
            McError::Engine(e) => write!(f, "engine: {e}"),
        }
    }
}

impl std::error::Error for McError {}

impl From<EngineError> for McError {
    fn from(e: EngineError) -> Self {
        McError::Engine(e)
    }
}

// =================================================================================================
// Semilla estable por usuario (D23)
// =================================================================================================

/// **La semilla estable de un usuario** (D23): misma instalación y mismo usuario ⇒ mismas
/// realizaciones de mercado, hoy y dentro de un año, haya editado sus datos o no.
///
/// Sin esto, cada request sortearía otro mercado y la probabilidad de éxito bailaría al refrescar
/// — el fallo exacto que la skill `futurefin-research-frontier` §6 le reprocha a las herramientas
/// de consumo («fresh RNG per view: the number changes on refresh, killing trust»).
///
/// # El hash, dicho entero
///
/// `FNV-1a` de 64 bits sobre los **32 bytes** `installation_id ‖ user_id` en **big-endian**
/// (`u128::to_be_bytes`, en ese orden), seguido del finalizador de `splitmix64`:
///
/// ```text
///   h ← 0xcbf29ce484222325
///   por cada byte b:  h ← (h XOR b) · 0x100000001b3        (mod 2^64)
///   semilla ← splitmix64_finalize(h)
/// ```
///
/// FNV-1a es total y estable pero tiene mala avalancha (dos UUID que difieren en un bit dan
/// semillas cercanas); el finalizador arregla eso. **Nada de esto es criptografía** y no
/// pretende serlo: el trabajo del hash es ser DETERMINISTA y estar bien repartido, y quien
/// produce aleatoriedad de verdad es ChaCha8 a partir de esta semilla.
///
/// No se usa `DefaultHasher`/`SipHash` de la biblioteca estándar **a propósito**: su algoritmo no
/// está garantizado entre versiones de Rust, y una semilla que cambia al actualizar el toolchain
/// es una semilla que no existe.
pub fn seed_for(installation_id: u128, user_id: u128) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for b in installation_id
        .to_be_bytes()
        .into_iter()
        .chain(user_id.to_be_bytes())
    {
        h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
    }
    splitmix64_finalize(h)
}

/// El finalizador de `splitmix64` (Steele/Lea/Flood, 2014): tres rondas de xor-shift y
/// multiplicación que reparten cada bit de entrada por los 64 de salida.
#[inline]
fn splitmix64_finalize(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Un paso completo de `splitmix64` sobre un estado mutable.
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    splitmix64_finalize(*state)
}

// =================================================================================================
// El sorteo
// =================================================================================================

/// El RNG del camino `path_index`: **un flujo propio por camino**, derivado de la semilla.
///
/// Los 32 bytes de clave se construyen AQUÍ con `splitmix64` sembrado en
/// `seed XOR (path · φ⁻¹·2⁶⁴)`, y no con [`SeedableRng::seed_from_u64`]: la expansión de ese
/// método es un detalle de implementación de `rand_core` y atarle la reproducibilidad de la app
/// haría que una actualización de dependencia moviera las bandas de todo el mundo en silencio.
///
/// Que cada camino tenga su propio flujo tiene dos consecuencias que valen la pena: el camino `p`
/// es el mismo con `paths = 500` que con `paths = 2 000` (ampliar la muestra no reescribe la
/// muestra que ya había), y la ejecución es paralelizable el día que haga falta sin cambiar ni un
/// dígito.
fn path_rng(seed: u64, path_index: u32) -> ChaCha8Rng {
    let mut state = seed ^ u64::from(path_index).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut key = [0u8; 32];
    for chunk in key.chunks_exact_mut(8) {
        chunk.copy_from_slice(&splitmix64(&mut state).to_le_bytes());
    }
    ChaCha8Rng::from_seed(key)
}

/// `2^-53`, el paso de la rejilla uniforme que se extrae de un `u64`.
const TWO_POW_MINUS_53: f64 = 1.0 / 9_007_199_254_740_992.0;

/// Uniforme en `[0, 1)` con los 53 bits altos — la rejilla más fina que un `f64` representa sin
/// huecos.
#[inline]
fn unit_half_open(bits: u64) -> f64 {
    (bits >> 11) as f64 * TWO_POW_MINUS_53
}

/// Uniforme en `(0, 1]`. El cero **debe** quedar fuera: es el argumento de un logaritmo.
#[inline]
fn unit_half_open_upper(bits: u64) -> f64 {
    ((bits >> 11) + 1) as f64 * TWO_POW_MINUS_53
}

/// **Un normal estándar por Box–Muller**, en su forma trigonométrica:
///
/// ```text
///   z = √(−2·ln u₁) · cos(2π·u₂),   u₁ ∈ (0,1],  u₂ ∈ [0,1)
/// ```
///
/// La transformación es EXACTA (no una aproximación de la inversa de la normal): si `u₁` y `u₂`
/// son uniformes independientes, `z` es exactamente `N(0,1)`. Los tests
/// `box_muller_has_the_moments_of_a_standard_normal` y `the_chacha_stream_is_pinned` la miden y
/// la fijan.
///
/// **Se escribe aquí en vez de traer `rand_distr`** por una dependencia menos en un binario
/// autocontenido, y porque `rand_distr` usa el método del zigurat, cuyas tablas son un detalle de
/// implementación de esa caja: la secuencia de normales cambiaría con una actualización y las
/// bandas de todos los usuarios se moverían sin que ningún test lo dijera. Box–Muller es cuatro
/// líneas que no dependen de nadie.
///
/// El segundo normal que el método produce (`sin` en vez de `cos`) **se descarta**. Guardarlo
/// ahorraría la mitad de las llamadas al RNG, pero ataría el flujo a la paridad de las llamadas
/// —un mes de más y toda la simulación cambia de sorteo—, y el coste medido es del orden de 17 µs
/// por camino de 840 meses frente a ~1 ms de simulación: 2 % por una propiedad que no interesa
/// perder.
#[inline]
fn standard_normal(rng: &mut ChaCha8Rng) -> f64 {
    let u1 = unit_half_open_upper(rng.next_u64());
    let u2 = unit_half_open(rng.next_u64());
    (-2.0 * u1.ln()).sqrt() * (core::f64::consts::TAU * u2).cos()
}

/// Volatilidad MENSUAL por activo a partir de la anual en % (§A.2 del plan).
///
/// **Política declarada de degradación**: `None`, negativa, no finita o cero ⇒ `σ = 0`, es decir
/// activo determinista. La API ya acota el campo a `[0, 100]`; esta guarda protege frente a un
/// valor absurdo ya persistido y frente a un `NaN` que envenenaría TODOS los caminos con un
/// factor no finito (y, por `checked_mul`, con un `AssetValueOverflow` que no explicaría nada).
fn monthly_sigma(annual_volatility_percent: Option<f64>) -> f64 {
    match annual_volatility_percent {
        Some(v) if v.is_finite() && v > 0.0 => v / 100.0 / 12f64.sqrt(),
        _ => 0.0,
    }
}

// =================================================================================================
// El motor de caminos
// =================================================================================================

/// La maquinaria de un camino, con **todo lo caro hecho una sola vez**: la conversión de la
/// entrada al tipo del núcleo, los multiplicadores deterministas `m_i`, las `σ_i` y el buffer de
/// factores (`meses × activos`).
///
/// El buffer viaja hacia dentro y hacia fuera de [`SimInput::growth_overrides`] con `Option::take`
/// en cada camino: **cero asignaciones por mes y cero por camino** para los factores. Lo que
/// `simulate` asigna por su cuenta (sus series de salida) no lo puede evitar esta capa sin tocar
/// el núcleo.
/// `pub(crate)` para el módulo hermano `solve_mc`: los solves estocásticos evalúan el
/// MISMO sorteo en muchos meses de jubilación distintos y necesitan sostener esta maquinaria entre
/// evaluaciones (mutando `sim.phase_plan.retirement_trigger`) en vez de reconstruirla por sorteo.
/// **No es API pública del crate**: `lib.rs` no lo reexporta.
pub(crate) struct PathEngine {
    pub(crate) sim: SimInput<F64Money>,
    /// `m_i`, el multiplicador determinista de cada activo: la CAGR declarada compuesta a mes, y
    /// —desde el modelo v2— la **MEDIANA** del factor que se sortea.
    base: Vec<F64Money>,
    /// `d_i = m_i · exp(σ_i²/2)`, la **deriva**: la MEDIA aritmética del factor que se sortea.
    /// Es la conversión CAGR → aritmética, y vive solo aquí.
    drift: Vec<F64Money>,
    /// `σ_i` mensual de cada activo.
    sigmas: Vec<f64>,
    seed: u64,
    /// **Primer mes (1-based) en el que el factor se SORTEA.** Antes de él el factor es el
    /// determinista `m_i`, exactamente el que usa el camino sin overrides. Default `1` = sortear
    /// todo el horizonte, que es el comportamiento de siempre.
    ///
    /// Lo usa la definición CONDICIONADA del capital necesario
    /// (`crate::needed_capital`): «lo que necesitas TENER a esa edad» fija la acumulación hasta
    /// `k−1` en la línea determinista y solo sortea el tramo que viene DESPUÉS de jubilarse. Sin
    /// este eje, el nodo `k` de la curva arrastraba la dispersión de treinta años de acumulación
    /// (medido: ×3,19 a 30 años) y publicaba un percentil del hogar escalado en vez de un capital.
    stochastic_from_month: u32,
    buf: Option<Vec<Vec<F64Money>>>,
}

impl PathEngine {
    pub(crate) fn new(
        input: &ProjectionInput,
        volatilities: &[Option<f64>],
        config: &McConfig,
    ) -> Result<Self, McError> {
        if config.paths == 0 || config.paths > MAX_PATHS {
            return Err(McError::InvalidPaths(config.paths));
        }
        if config.percentiles.is_empty() || config.percentiles.iter().any(|p| *p == 0 || *p > 99) {
            return Err(McError::InvalidPercentiles);
        }
        if volatilities.len() != input.assets.len() {
            return Err(McError::VolatilityLengthMismatch(
                volatilities.len(),
                input.assets.len(),
            ));
        }
        let sim = SimInput::<F64Money>::from(input);
        // MISMA raíz doceava que el camino determinista: la del motor, no una copia.
        let base: Vec<F64Money> = sim
            .assets
            .iter()
            .map(|a| monthly_growth_multiplier(a.expected_annual_return_percent))
            .collect();
        let sigmas: Vec<f64> = volatilities.iter().copied().map(monthly_sigma).collect();
        // **La conversión CAGR → media aritmética, en su ÚNICO sitio.** La rentabilidad declarada
        // es COMPUESTA (decisión M8), así que `m_i` tiene que ser la MEDIANA del factor sorteado;
        // para que lo sea, la deriva de la log-normal sube `exp(σ_m²/2)` y la corrección de Itô se
        // la come justo hasta dejar la mediana en `m_i`.
        //
        // La σ es la **MENSUAL**: la prima de varianza pertenece al factor que se sortea cada mes.
        // `CAGR + σ_anual²/2` sobre los porcentajes anuales es la fórmula equivocada — y es 12
        // veces mayor que la corrección que este factor necesita.
        let drift: Vec<F64Money> = base
            .iter()
            .zip(sigmas.iter())
            .map(|(m, s)| {
                if *s == 0.0 {
                    // Rama explícita, no `m · exp(0)`: «sin volatilidad declarada» significa «el
                    // camino determinista», y eso se ESCRIBE. Misma disciplina que la rama σ = 0
                    // de `run`, aunque `x · 1.0 == x` en IEEE-754 la haga redundante hoy.
                    *m
                } else {
                    F64Money(m.0 * (0.5 * s * s).exp())
                }
            })
            .collect();
        let months = input.horizon_months as usize;
        let buf = Some(vec![vec![F64Money(0.0); sim.assets.len()]; months]);
        Ok(PathEngine {
            sim,
            base,
            drift,
            sigmas,
            seed: config.seed,
            stochastic_from_month: 1,
            buf,
        })
    }

    /// **Arranca el sorteo en `month`** (1-based): los meses `1..month−1` crecen con el factor
    /// DETERMINISTA `m_i` y solo desde `month` se aplica el shock.
    ///
    /// `month ≤ 1` (y `0`) es el default: sortear todo el horizonte.
    pub(crate) fn set_stochastic_from_month(&mut self, month: u32) {
        self.stochastic_from_month = month.max(1);
    }

    /// ¿Hay algún activo con volatilidad declarada? Con `false`, todos los caminos son el camino
    /// determinista y la banda es una línea — lo que la SPA avisa (§G).
    fn any_volatility(&self) -> bool {
        self.sigmas.iter().any(|s| *s > 0.0)
    }

    /// Ejecuta el camino `path_index`: sortea sus factores en el buffer, los inyecta por
    /// `growth_overrides` y llama al MISMO `simulate` que produce el camino determinista.
    pub(crate) fn run(&mut self, path_index: u32) -> Result<SimOutput<F64Money>, EngineError> {
        let mut buf = self
            .buf
            .take()
            .expect("el buffer siempre vuelve al final de `run`");
        let mut rng = path_rng(self.seed, path_index);
        for (m, row) in buf.iter_mut().enumerate() {
            // UN shock por mes, sorteado SIEMPRE — también con la cartera entera a σ=0, y también
            // en los meses del PREFIJO DETERMINISTA. Que el flujo del RNG no dependa de los datos
            // (ni del punto en que arranca el sorteo) es lo que hace comparables dos ejecuciones
            // con la misma semilla.
            //
            // **Y es una decisión, no una casualidad**: con `stochastic_from_month = k` el camino
            // `p` ve en el mes `k` EXACTAMENTE el mismo `z` que vería con `stochastic_from_month
            // = 1`, o que en cualquier otro nodo de la curva. Consumir solo desde `k` desplazaría
            // el flujo un mes por cada mes de prefijo y cada nodo mediría con otra muestra: la
            // curva dejaría de ser comparable consigo misma y la bisección de un nodo se movería
            // por cambiar el nodo, no por cambiar el capital. Números aleatorios comunes, la
            // misma disciplina que `solve_mc` aplica entre presupuestos.
            //
            // El coste es un `standard_normal` (dos `next_u64`, un `ln`, un `cos`) por mes de
            // prefijo — se paga a cambio de que la muestra no se mueva.
            let z = standard_normal(&mut rng);
            // `m` es 0-based sobre el buffer; el mes del bucle es `m + 1`.
            let deterministic_prefix = (m as u32) + 1 < self.stochastic_from_month;
            for (i, cell) in row.iter_mut().enumerate() {
                let s = self.sigmas[i];
                *cell = if s == 0.0 || deterministic_prefix {
                    // Rama explícita, no `exp(0)`: `σ=0` significa «el camino determinista», y
                    // eso se escribe, no se deduce de que `1.0` sea neutro. Un mes del prefijo
                    // entra por la MISMA puerta y con el MISMO `m_i`, así que el tramo `1..k−1`
                    // de cualquier camino es, factor a factor, el del camino determinista.
                    self.base[i]
                } else {
                    // `d_i · exp(σz − σ²/2)`, que se SIMPLIFICA a `m_i · exp(σz)` — y se escribe
                    // sin simplificar a propósito: con la deriva dentro y la corrección de Itô
                    // fuera, las dos propiedades se leen sin despejar nada.
                    //   E[f]       = d_i          = m_i · exp(σ²/2)   (media aritmética)
                    //   mediana(f) = d_i·exp(−σ²/2) = m_i             (la línea determinista)
                    F64Money(self.drift[i].0 * (s * z - 0.5 * s * s).exp())
                };
            }
        }
        self.sim.growth_overrides = Some(buf);
        let out = simulate(&self.sim);
        // El buffer vuelve TAMBIÉN si la simulación falló: el `expect` de arriba depende de ello.
        self.buf = self.sim.growth_overrides.take();
        out
    }

    /// **Una copia independiente de esta maquinaria**, para que un hilo pueda correr caminos sin
    /// compartir nada (E12).
    ///
    /// Lo que se copia es solo maquinaria: la entrada convertida, los `m_i`, las `d_i`, las `σ_i`,
    /// la semilla y el punto de arranque del sorteo. **No hay estado que arrastrar entre caminos**
    /// —el RNG se construye desde `(seed, path_index)` en cada `run`— así que un camino corrido
    /// sobre una copia es, bit a bit, el mismo camino corrido sobre el original. Esa es toda la
    /// justificación del paralelismo, y es la razón de que esta función no tenga que decidir nada.
    ///
    /// El buffer se crea **vacío y nuevo** en vez de clonarse: su contenido es basura del camino
    /// anterior y `run` lo reescribe entero antes de mirarlo.
    pub(crate) fn fork(&self) -> PathEngine {
        let mut sim = self.sim.clone();
        // Fuera del `run` esto ya es `None`; se fuerza para que la copia no herede jamás una fila
        // de factores del camino del original.
        sim.growth_overrides = None;
        let months = sim.horizon_months as usize;
        let assets = sim.assets.len();
        PathEngine {
            sim,
            base: self.base.clone(),
            drift: self.drift.clone(),
            sigmas: self.sigmas.clone(),
            seed: self.seed,
            stochastic_from_month: self.stochastic_from_month,
            buf: Some(vec![vec![F64Money(0.0); assets]; months]),
        }
    }
}

/// **Cuántos caminos toca cada hilo dentro de un bloque** (E12).
///
/// El reparto es por BLOQUES —`threads · PATHS_PER_THREAD_BLOCK` caminos cada uno— y no de una
/// sola tacada por dos razones, ninguna de las cuales es el determinismo (ese lo da el índice, no
/// el tamaño del bloque):
///
/// - **Memoria.** Con bandas, el resultado por camino son dos vectores de `horizonte+1` `f64`
///   (~13,5 KB con 840 meses). Recoger los 5 000 caminos de golpe antes de transponerlos
///   duplicaría el pico de las muestras (67 MB → 134 MB) dentro de un contenedor que lleva el
///   PostgreSQL dentro. Por bloques, el excedente es `threads · 16 · 13,5 KB` ≈ 1,7 MB.
/// - **Coste del `fork`.** Cada hilo copia la maquinaria una vez por bloque: una copia (clonar la
///   entrada convertida y reservar el buffer de factores, decenas de µs) por cada 16 caminos
///   (varios ms). Queda por debajo del 1 %; bajar el número a 1 lo multiplicaría por dieciséis sin
///   ganar reparto, porque los caminos cuestan todos lo mismo — el bucle del motor recorre el
///   horizonte entero, sin salidas anticipadas.
///
/// Y una tercera razón para NO hacerlo de una tacada, que se ve en las medidas: en una máquina
/// heterogénea (núcleos de rendimiento + de eficiencia, o una VM con vecinos ruidosos) el reparto
/// estático de todo el sorteo hace que el trozo que cayó en el núcleo lento marque el tiempo
/// total. Los bloques reequilibran cada `threads · 16` caminos.
const PATHS_PER_THREAD_BLOCK: usize = 16;

/// **Recorre `0..paths` y entrega los resultados EN ORDEN DE ÍNDICE DE CAMINO** (E12).
///
/// Es el único sitio del crate donde se decide cómo se reparte el trabajo, y el contrato es de una
/// línea: `extract` puede ejecutarse en cualquier hilo y en cualquier orden; `sink` se llama
/// **siempre** con `p = 0, 1, 2, …` desde el hilo llamante. Toda reducción que un llamante haga en
/// `sink` —sumar `f64`, contar, empujar a un vector— hereda por tanto el orden secuencial de
/// siempre, que es lo que hace que el resultado sea bit a bit el mismo con uno o con ocho hilos.
///
/// Con `threads <= 1` no se toca rayon: se recorre en el hilo llamante sobre una única copia de la
/// maquinaria. Es el modo con el que la batería de determinismo compara.
///
/// Un camino que falla **aborta la ejecución entera** (`?` en el llamante), igual que antes:
/// descartarlo sesgaría la probabilidad de éxito hacia arriba justo en los escenarios extremos. Y
/// **el error que sale es el del camino de índice más bajo que falló**, con uno o con ocho hilos —
/// ver el comentario del `collect` de abajo: colapsar el `Result` en paralelo devolvería el error
/// del hilo que llegara antes, y dos ejecuciones «iguales» podrían fallar con motivos distintos.
pub(crate) fn for_each_path<T, F, S>(
    engine: &PathEngine,
    paths: u32,
    threads: usize,
    extract: F,
    mut sink: S,
) -> Result<(), EngineError>
where
    F: Fn(u32, &SimOutput<F64Money>) -> T + Sync + Send,
    S: FnMut(u32, T) + Send,
    T: Send,
{
    if threads <= 1 {
        let mut engine = engine.fork();
        for p in 0..paths {
            let out = engine.run(p)?;
            sink(p, extract(p, &out));
        }
        return Ok(());
    }

    // **Una sola entrada al pool para TODA la ejecución**, no una por bloque. Entrar y salir
    // despierta y vuelve a dormir a los workers, y rayon los deja girando un rato antes de
    // dormirse: con un sorteo de 500 caminos son cuatro ciclos de eso, y en una máquina saturada
    // —la suite de integración, o un contenedor con el PostgreSQL dentro— ese giro le quita CPU al
    // reactor sin adelantar nada. Con el bucle DENTRO, los workers se despiertan una vez y
    // trabajan hasta el final.
    //
    // El precio es que el pliegue (`sink`) corre en un hilo del pool en vez de en el llamante. No
    // cambia nada de lo que importa: **sigue siendo UN solo hilo y sigue yendo en orden de
    // índice**, que es lo que hace el resultado reproducible; por eso `S: Send`.
    crate::parallel::install(threads, move || {
        for_each_path_blocks(engine, paths, threads, &extract, &mut sink)
    })
}

/// El bucle de bloques de [`for_each_path`], ya **dentro** del pool. Separado solo para que la
/// entrada al pool sea una sola línea y no un bloque de treinta.
fn for_each_path_blocks<T, F, S>(
    engine: &PathEngine,
    paths: u32,
    threads: usize,
    extract: &F,
    sink: &mut S,
) -> Result<(), EngineError>
where
    F: Fn(u32, &SimOutput<F64Money>) -> T + Sync,
    S: FnMut(u32, T),
    T: Send,
{
    let block = (threads * PATHS_PER_THREAD_BLOCK) as u32;
    let mut start = 0u32;
    while start < paths {
        let end = start.saturating_add(block).min(paths);
        let indices: Vec<u32> = (start..end).collect();
        let per_chunk = indices.len().div_ceil(threads).max(1);
        // **El `Result` se recoge por trozo, no se colapsa en paralelo**, y esa distinción es parte
        // del determinismo: `collect::<Result<_, _>>()` sobre un iterador de rayon devuelve UNO de
        // los errores, y cuál depende de qué hilo llegó antes. Recogiendo `Vec<Result<…>>` —que
        // conserva el orden— y desenvolviendo abajo en orden de índice, el error que sale es
        // siempre el del camino de índice MÁS BAJO que falló, igual que en el bucle secuencial. Un
        // `EngineError` distinto según el número de hilos sería la misma clase de fallo silencioso
        // que este módulo existe para no tener.
        let per_chunk: Vec<Result<Vec<T>, EngineError>> = indices
            .par_chunks(per_chunk)
            .map(|chunk| {
                // Una copia de la maquinaria por hilo y por bloque. Nada se comparte.
                let mut local = engine.fork();
                // Dentro del trozo sí se cortocircuita: los índices de un trozo son
                // contiguos y ascendentes, así que el primero que falla ES el más bajo.
                chunk
                    .iter()
                    .map(|&p| local.run(p).map(|out| extract(p, &out)))
                    .collect::<Result<Vec<T>, EngineError>>()
            })
            .collect();
        let mut results: Vec<Vec<T>> = Vec::with_capacity(per_chunk.len());
        for chunk in per_chunk {
            results.push(chunk?);
        }
        // **Aquí es donde el paralelismo deja de existir.** `par_chunks` conserva el orden de los
        // trozos y cada trozo conserva el suyo, así que aplanar reconstruye exactamente
        // `start..end`.
        let mut p = start;
        for t in results.into_iter().flatten() {
            sink(p, t);
            p += 1;
        }
        debug_assert_eq!(p, end, "el pliegue debe cubrir el bloque entero, en orden");
        start = end;
    }
    Ok(())
}

/// **Un solo camino de Monte Carlo**, con toda su salida del motor.
///
/// Existe para lo que las bandas no pueden dar: verificar la reproducibilidad camino a camino,
/// medir la MEDIANA del terminal contra la línea determinista y la MEDIA contra la prima de
/// varianza (`mc_median_is_the_deterministic_line`) y permitir que un caller inspeccione una
/// realización concreta. Para dibujar bandas, use
/// [`project_percentile_bands`]: esta función reconstruye la maquinaria en cada llamada.
pub fn run_path(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    config: &McConfig,
    path_index: u32,
) -> Result<SimOutput<F64Money>, McError> {
    run_path_from(input, volatilities, config, path_index, 1)
}

/// **Un camino con PREFIJO DETERMINISTA**: los meses `1..stochastic_from_month−1` crecen con el
/// multiplicador determinista del motor y solo desde `stochastic_from_month` se aplica el shock.
///
/// Es el eje que la definición CONDICIONADA del capital necesario necesita («lo que hay que TENER
/// a esa edad»: la acumulación no se sortea, el tramo jubilado sí) y se expone aquí para que la
/// propiedad que la sostiene sea comprobable desde fuera del crate: **el `z` del mes `k` es el
/// mismo con prefijo y sin él**, porque el RNG se consume también en los meses deterministas.
/// Regresión: `the_deterministic_prefix_consumes_the_same_random_numbers`.
///
/// `stochastic_from_month ≤ 1` es exactamente [`run_path`].
pub fn run_path_from(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    config: &McConfig,
    path_index: u32,
    stochastic_from_month: u32,
) -> Result<SimOutput<F64Money>, McError> {
    let mut engine = PathEngine::new(input, volatilities, config)?;
    engine.set_stochastic_from_month(stochastic_from_month);
    engine.run(path_index).map_err(McError::Engine)
}

// =================================================================================================
// Salida
// =================================================================================================

/// **El resultado de una ejecución de Monte Carlo.** Todo estadístico; ni un euro publicable.
#[derive(Debug, Clone, PartialEq)]
pub struct McOutcome {
    /// La semilla usada, ecoada: sin ella el resultado no es reproducible y por tanto no es un
    /// resultado.
    pub seed: u64,
    /// Caminos efectivamente simulados.
    pub paths: u32,
    /// Percentiles publicados, en el MISMO orden que las bandas.
    pub percentiles: Vec<u8>,
    /// Horizonte simulado, en meses. Cada banda tiene `horizon_months + 1` puntos.
    pub horizon_months: u32,
    /// Bandas **puntuales** de `net_worth`: `net_worth[j][k]` es el percentil
    /// `percentiles[j]` de los `paths` valores del mes `k`.
    ///
    /// **Puntual quiere decir puntual**: la banda p50 NO es un camino. El hogar que en el mes 100
    /// está en la mediana no tiene por qué ser el que está en la mediana en el mes 400, así que
    /// la curva p50 no corresponde a ninguna simulación real y no cumple ninguna identidad
    /// contable (su patrimonio no es la suma de sus activos). Es lo que la ayuda de la UI tiene
    /// que decir.
    pub net_worth: Vec<Vec<f64>>,
    /// Bandas puntuales de `liquid_worth`, con la misma forma y la misma advertencia.
    pub liquid_worth: Vec<Vec<f64>>,
    /// **Éxito v2 (E9, modelo de jubilación v2)**: fracción de caminos SIN fallo, es decir con
    /// `failure_month_index.is_none()` — **ningún fallo F1/F2/F3 en el camino**. Los tres motivos
    /// (F1 cartera agotada, F2 tasa inicial excedida, F3 la regla por saldo no llega a la
    /// necesidad ordinaria) los clasifica el bucle del motor (`crates/engine`); aquí solo se
    /// CUENTAN.
    ///
    /// Ya **no depende de si el hogar se jubila**: con la API v2 todo plan se sortea con
    /// `RetirementTrigger::AtMonth` — un mes forzado que decide el llamante (el solver
    /// `crates/engine-stochastic::solve_mc` o quien construya el `ProjectionInput`) — así que
    /// «no jubilarse nunca» ha dejado de ser un desenlace posible del sorteo y los campos que
    /// separaban esa pregunta (`never_retired_probability`, `success_given_retired`, y
    /// `underfunded_probability`, que leía la infra-financiación de un trigger por edad) se
    /// retiraron: esa lectura es ahora `1 − éxito(R)` del solver estocástico
    /// (`solve_mc::valid_retirement_month`/`success_at_month`).
    pub success_probability: f64,
    /// **Límite inferior del intervalo de Wilson al 95 %** de [`Self::success_probability`] —
    /// calculado reutilizando [`crate::SuccessAt::new`] (nunca reimplementado aquí). Es el número
    /// estable frente a semilla y a `N` con el que se compara un umbral de producto, y con
    /// `failures == 0` es **estrictamente menor que 1** (nunca «100 % seguro» solo porque no se
    /// vio ningún fallo en la muestra).
    pub wilson_low: f64,
    /// Distancia de [`Self::success_probability`] a [`Self::wilson_low`], en PUNTOS
    /// PORCENTUALES: la barra de error que se dibuja hacia abajo. Ver
    /// [`crate::SuccessAt::half_width_pp`] — misma cifra, misma fórmula.
    pub half_width_pp: f64,
    /// Fallos por motivo, contando el PRIMER fallo de cada camino, en el orden
    /// [`PathFailure::PortfolioDepleted`] (F1) / [`PathFailure::InitialRateExceeded`] (F2) /
    /// [`PathFailure::RuleBelowNeed`] (F3) — los mismos índices que
    /// [`crate::KIND_PORTFOLIO_DEPLETED`]/[`crate::KIND_INITIAL_RATE_EXCEEDED`]/
    /// [`crate::KIND_RULE_BELOW_NEED`] de `solve_mc`, para que las dos capas no diverjan en el
    /// orden. Suma exactamente `paths − paths·`[`Self::success_probability`].
    pub failures_by_kind: [u32; 3],
    /// Fracción ACUMULADA de caminos con ALGÚN fallo (F1, F2 o F3) en `(mes, p)`, cada
    /// [`FAILURE_STEP_MONTHS`] meses desde el ancla. `p` es la fracción de caminos con
    /// `failure_month_index ≤ mes`. El caller traduce meses a edades.
    ///
    /// **Sustituye a `depletion_probability_by_age`** (que solo contaba F1): con la puerta de
    /// tasa inicial (F2) y la regla por saldo (F3) dentro del bucle, «agotamiento» dejó de ser el
    /// único motivo por el que un camino deja de cumplir el plan.
    ///
    /// El ancla es el mes de jubilación FORZADO del plan
    /// (`input.phase_plan.retirement_trigger.forced_month()`); si el plan todavía trae
    /// `RetirementTrigger::LiquidCrossing` (llamantes/tests legacy que no han migrado al mes
    /// forzado de la v2), el ancla es la jubilación efectiva del camino DETERMINISTA como ANTES
    /// —y, a falta de ella, la mediana de los caminos sorteados—; si ninguno se jubila, el vector
    /// va **vacío**. La última fila es SIEMPRE el horizonte (cierra ahí aunque no sea múltiplo del
    /// paso), y esa fila coincide, al bit, con `1 − `[`Self::success_probability`].
    pub cumulative_failure_by_age: Vec<(u32, f64)>,
    /// Mediana, entre los caminos, del número de meses jubilados con recorte
    /// (`withdrawal_shortfall > 0`). Con `fixed_real` es 0 por construcción.
    pub months_below_need_p50: u32,
    /// Mediana, entre los caminos, de la cobertura de la necesidad ORDINARIA sobre los meses
    /// jubilados: `Σ max(0, w − excess) / Σ max(0, w + s + u − excess)`, cada término clampado a
    /// `≥ 0` **mes a mes** antes de sumar. `1.0` = la cubrió entera. `None` si ningún camino tiene
    /// meses jubilados con denominador positivo.
    ///
    /// **Corregida en E9 (bug B2)**: hasta este cambio el numerador era `Σ w` sin más, y bajo
    /// `rule_is_spend` (D5) `withdrawal` incluye el EXCESO sobre la necesidad
    /// (`withdrawal_excess`, D24) — la regla ES el gasto y vende `permitido` aunque sobre. Un mes
    /// con superávit inflaba la cobertura por encima de 1,0 sin que nada lo dijera. La identidad
    /// del motor (`fuzz_invariants.rs`, `crates/engine`) es
    /// `withdrawal + withdrawal_shortfall + unmet_need − withdrawal_excess = need_net`, así que
    /// `w + s + u − excess` ES `need_net` exactamente — y **puede ser negativo** desde el mes en
    /// que la pensión supera el gasto (need_net negativo), lo que exige el clamp mes a mes en vez
    /// de uno solo al final.
    pub withdrawal_to_need_ratio_p50: Option<f64>,
    /// ¿Algún activo declaró volatilidad? Con `false` todas las bandas coinciden con la línea
    /// determinista y la UI debe decirlo («sin volatilidad declarada: la banda es la línea»).
    pub any_volatility_declared: bool,
}

// =================================================================================================
// Percentiles
// =================================================================================================

/// **Rango más cercano** (`nearest-rank`), en aritmética entera:
///
/// ```text
///   rango = ⌈p·n/100⌉        índice = rango − 1, acotado a [0, n−1]
/// ```
///
/// Es el percentil «de orden»: **siempre devuelve un valor observado**, nunca una interpolación
/// entre dos caminos. Se elige frente a la interpolación lineal por dos razones: el valor
/// publicado corresponde a un escenario que la simulación produjo de verdad, y el cálculo es
/// entero, así que no hay un redondeo de coma flotante decidiendo de qué lado cae el índice
/// —justo el tipo de detalle que rompe la reproducibilidad entre plataformas—.
///
/// `⌈p·n/100⌉` se calcula con `div_ceil` sobre enteros: exacto, sin `ceil` ni `f64`.
fn nearest_rank_index(n: usize, p: u8) -> usize {
    debug_assert!(n > 0 && (1..=99).contains(&p));
    let rank = (usize::from(p) * n).div_ceil(100);
    rank.clamp(1, n) - 1
}

/// El percentil de una muestra YA ordenada ascendentemente.
fn percentile_of_sorted(sorted: &[f64], p: u8) -> f64 {
    sorted[nearest_rank_index(sorted.len(), p)]
}

/// Ordena `f64` con el orden TOTAL (`total_cmp`): `partial_cmp` se rinde con `NaN` y un
/// comparador que se rinde deja el vector en un orden que depende del algoritmo.
fn sort_total(values: &mut [f64]) {
    values.sort_unstable_by(f64::total_cmp);
}

// =================================================================================================
// La ejecución completa
// =================================================================================================

/// **Monte Carlo completo**: `config.paths` caminos, bandas puntuales y las probabilidades de
/// §B.5.
///
/// # Coste
///
/// Tiempo: `paths` simulaciones completas del motor en `f64` más `O(paths·log paths)` por mes de
/// ordenación. Las simulaciones se reparten entre núcleos (E12, ver [`for_each_path`]); la
/// ordenación y el resto del pliegue son secuenciales por contrato. Memoria:
/// `2 · paths · (horizonte+1) · 8` bytes para las muestras (67 MB en el extremo de 5 000 caminos ×
/// 840 meses), más `hilos · 16 · 2 · (horizonte+1) · 8` bytes de resúmenes en vuelo (~1,7 MB), un
/// buffer de factores por hilo (`meses · activos · 8` bytes) y las series de las simulaciones
/// vivas. Los números medidos están en `tests/timing_mc.rs`.
///
/// # Determinismo
///
/// Dos ejecuciones con la misma entrada y la misma [`McConfig`] devuelven [`McOutcome`]s **bit a
/// bit iguales**: el sorteo depende solo de `(seed, path_index)`, el orden de los caminos es el
/// del bucle y el percentil es un índice entero sobre una muestra ordenada con un orden total.
/// Lo pinea `mc_same_seed_bit_identical`.
///
/// **Y son iguales también con cualquier número de hilos** (E12). Los caminos se reparten entre
/// núcleos, pero cada uno produce su resumen por su cuenta —ninguna suma cruza caminos dentro de
/// un camino— y el pliegue (bandas, conteos, medianas, Wilson, tabla acumulada) se hace en ORDEN
/// DE ÍNDICE, secuencialmente, sobre lo que devuelve [`for_each_path`]. `McConfig::threads =
/// Some(1)` recorre los caminos en el hilo llamante y es el modo de comparar. Lo pinean
/// `parallel_and_sequential_runs_are_bit_identical` y
/// `results_do_not_depend_on_the_thread_count`.
pub fn project_percentile_bands(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    config: &McConfig,
) -> Result<McOutcome, McError> {
    let engine = PathEngine::new(input, volatilities, config)?;
    let any_volatility_declared = engine.any_volatility();

    let n = config.paths as usize;
    let len = input.horizon_months as usize + 1;

    // Muestras transpuestas —`[mes][camino]`— porque lo que hay que ordenar es un mes entero.
    let mut nw_samples: Vec<Vec<f64>> = vec![vec![0.0; n]; len];
    let mut lq_samples: Vec<Vec<f64>> = vec![vec![0.0; n]; len];

    // `retired_at` solo alimenta el ANCLA legacy de `cumulative_failure_by_age` (plan por CRUCE,
    // ver más abajo): con el mes forzado de la v2 el ancla es directa y no necesita este vector,
    // pero un `ProjectionInput` que todavía traiga `RetirementTrigger::LiquidCrossing` (llamantes
    // o tests que no han migrado) sigue leyendo la jubilación efectiva del sorteo, como antes.
    let mut retired_at: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut failure_month: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut failure_kind: Vec<Option<PathFailure>> = Vec::with_capacity(n);
    let mut months_below: Vec<f64> = Vec::with_capacity(n);
    let mut coverage_ratios: Vec<f64> = Vec::with_capacity(n);

    // **Lo que un camino aporta al resultado**, recogido dentro del propio camino (E12). Todo lo
    // que aquí se calcula es INTRA-camino: dos vectores de muestras, cuatro escalares y dos sumas
    // sobre los meses de ESE camino. Ninguna suma cruza caminos, así que repartirlos entre hilos
    // no puede mover un bit; lo que sí cruza caminos —las bandas, los conteos, las medianas— se
    // pliega más abajo, en orden de índice y en un solo hilo.
    struct PathDigest {
        net_worth: Vec<f64>,
        liquid_worth: Vec<f64>,
        retired_at: Option<u32>,
        failure_month: Option<u32>,
        failure_kind: Option<PathFailure>,
        months_below: u32,
        /// `Σ w_net` y `Σ need_net` de los meses jubilados; `None` si el camino no aporta
        /// cobertura (sin jubilación, o con denominador no positivo).
        coverage: Option<f64>,
    }

    let digest = |_p: u32, out: &SimOutput<F64Money>| -> PathDigest {
        debug_assert_eq!(out.net_worth.len(), len);
        debug_assert_eq!(
            out.failure_month_index.is_some(),
            out.failure_kind.is_some(),
            "un camino fallido sin motivo (o un motivo sin fallo) rompería `failures_by_kind`"
        );

        // Las dos magnitudes del RECORTE (D24), sobre los meses JUBILADOS de este camino. Fuera
        // de la jubilación el motor no aplica techo alguno, así que el recorte solo puede vivir
        // ahí; se acota explícitamente de todos modos para que la lectura no dependa de eso.
        let (mut below, mut sum_w, mut sum_need) = (0u32, 0.0f64, 0.0f64);
        if let Some(r) = out.retirement_month_index {
            for k in (r as usize)..len {
                let w = out.withdrawal[k].0;
                let s = out.withdrawal_shortfall[k].0;
                // **La necesidad no cubierta entra en el denominador** (hallazgo #4 de la
                // revisión). Con `fixed_real` el recorte `s` es CERO por construcción —el
                // permitido ES la necesidad—, así que `Σw / Σ(w+s)` valía 1,0 siempre, también
                // en los caminos que se quedaban sin cartera en el mes 35 de 400 y cubrían el
                // 8,65 % de lo que necesitaban. Lo que faltaba estaba en la otra magnitud.
                let u = out.unmet_need[k].0;
                if s + u > 0.0 {
                    below += 1;
                }
                // **B2**: bajo `rule_is_spend` (D5) `withdrawal` incluye el EXCESO sobre la
                // necesidad (`withdrawal_excess`, D24) — la regla ES el gasto y vende `permitido`
                // aunque sobre. Sin descontarlo, un mes con superávit inflaba la cobertura. La
                // identidad del motor es `w + s + u − excess = need_net`
                // (`fuzz_invariants.rs:385`), y `need_net` puede ser NEGATIVO desde el mes en que
                // la pensión supera el gasto: se clampa a `≥ 0` MES A MES, no una sola vez al
                // final, para que un mes de superávit no reste cobertura a los demás.
                let e = out.withdrawal_excess[k].0;
                let w_net = (w - e).max(0.0);
                let need_net = (w + s + u - e).max(0.0);
                sum_w += w_net;
                sum_need += need_net;
            }
        }
        PathDigest {
            net_worth: out.net_worth.iter().map(|m| m.0).collect(),
            liquid_worth: out.liquid_worth.iter().map(|m| m.0).collect(),
            retired_at: out.retirement_month_index,
            failure_month: out.failure_month_index,
            failure_kind: out.failure_kind,
            months_below: below,
            coverage: (sum_need > 0.0).then(|| sum_w / sum_need),
        }
    };

    // El pliegue, en ORDEN DE ÍNDICE DE CAMINO y en un solo hilo — con uno o con ocho hilos
    // sorteando, esta parte se ejecuta exactamente igual. Ver `for_each_path`.
    for_each_path(
        &engine,
        config.paths,
        crate::parallel::resolve_threads(config),
        digest,
        |p, d| {
            let p = p as usize;
            for k in 0..len {
                nw_samples[k][p] = d.net_worth[k];
                lq_samples[k][p] = d.liquid_worth[k];
            }
            retired_at.push(d.retired_at);
            failure_month.push(d.failure_month);
            failure_kind.push(d.failure_kind);
            months_below.push(f64::from(d.months_below));
            if let Some(c) = d.coverage {
                coverage_ratios.push(c);
            }
        },
    )?;

    // ------------------------------------------------------------------------------------------
    // Bandas
    // ------------------------------------------------------------------------------------------
    for row in nw_samples.iter_mut() {
        sort_total(row);
    }
    for row in lq_samples.iter_mut() {
        sort_total(row);
    }
    let band = |samples: &[Vec<f64>]| -> Vec<Vec<f64>> {
        config
            .percentiles
            .iter()
            .map(|&p| {
                samples
                    .iter()
                    .map(|row| percentile_of_sorted(row, p))
                    .collect()
            })
            .collect()
    };
    let net_worth = band(&nw_samples);
    let liquid_worth = band(&lq_samples);

    // ------------------------------------------------------------------------------------------
    // Probabilidades
    // ------------------------------------------------------------------------------------------
    let n_f = n as f64;
    // **Éxito v2 (E9, modelo de jubilación v2)**: un camino falla ⟺ `failure_month_index.is_some()`
    // — ningún fallo F1/F2/F3 en el camino. Ya no depende de si el hogar se jubila: con la API v2
    // todo plan trae `RetirementTrigger::AtMonth` (el mes forzado que decide el solver externo o
    // el llamante), así que «no jubilarse nunca» dejó de ser un desenlace posible del sorteo. Las
    // lecturas que D22/D24 separaban para esa pregunta (`never_retired_probability`,
    // `success_given_retired`, `underfunded_probability`) se retiraron: esa infra-financiación es
    // ahora `1 − éxito(R)` del solver estocástico (`solve_mc`).
    let successes = (0..n).filter(|&p| failure_month[p].is_none()).count();
    let success_probability = successes as f64 / n_f;

    // Fallos por motivo: el PRIMER (y único, por construcción del bucle) motivo de cada camino
    // fallido, en el orden común con `solve_mc::SuccessAt::by_kind`.
    let mut failures_by_kind = [0u32; 3];
    for kind in failure_kind.iter().flatten() {
        failures_by_kind[kind_index(*kind)] += 1;
    }
    let failures = n - successes;
    debug_assert_eq!(
        failures_by_kind.iter().sum::<u32>() as usize,
        failures,
        "el reparto por motivo debe sumar exactamente los caminos fallidos"
    );

    // Wilson del éxito, reutilizando el MISMO constructor que `solve_mc` — nunca reimplementado
    // aquí. El `month` que pide la firma no se publica en `McOutcome` (esta banda no es la
    // medición de UN mes concreto de un solve, es la lectura completa del plan tal como llegó);
    // se pasa el mes forzado cuando existe, solo por trazabilidad de logs si se llegara a volcar
    // el valor, y `0` si el plan es legacy por cruce.
    let wilson_month = input.phase_plan.retirement_trigger.forced_month().unwrap_or(0);
    let success_at = SuccessAt::new(wilson_month, config.paths, failures as u32, failures_by_kind);
    let wilson_low = success_at.wilson_low;
    let half_width_pp = success_at.half_width_pp;

    // Ancla de `cumulative_failure_by_age`: el mes de jubilación FORZADO del plan (la v2 lo trae
    // siempre que el llamante ya haya resuelto la fecha). Con un `ProjectionInput` legacy que
    // todavía use `RetirementTrigger::LiquidCrossing`, el ancla es la jubilación efectiva del
    // camino DETERMINISTA —como ANTES de E9— y, a falta de ella, la mediana de los sorteados.
    let anchor = match input.phase_plan.retirement_trigger {
        RetirementTrigger::AtMonth(m) => Some(m),
        RetirementTrigger::LiquidCrossing => {
            let deterministic = crate::simulate_f64(input)?;
            let mut retired_sorted = retired_at.clone();
            // Los caminos que no se jubilan ordenan los ÚLTIMOS: «nunca» es el peor mes posible.
            retired_sorted.sort_by(|a, b| match (a, b) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => core::cmp::Ordering::Less,
                (None, Some(_)) => core::cmp::Ordering::Greater,
                (None, None) => core::cmp::Ordering::Equal,
            });
            deterministic
                .retirement_month_index
                .or(retired_sorted[nearest_rank_index(n, 50)])
        }
    };

    let mut cumulative_failure_by_age = Vec::new();
    if let Some(a) = anchor {
        let mut m = a;
        while m <= input.horizon_months {
            let hit = failure_month
                .iter()
                .filter(|f| f.is_some_and(|x| x <= m))
                .count() as f64;
            cumulative_failure_by_age.push((m, hit / n_f));
            m += FAILURE_STEP_MONTHS;
        }
        // **La última fila es el HORIZONTE** (hallazgo #8 de la revisión, conservado en E9). La
        // rejilla avanza de 60 en 60 desde el ancla y se pararía en el último múltiplo que
        // cupiera si no se forzara el cierre: con ancla 655 y horizonte 840 dejaría 5 meses fuera
        // sin decirlo. Ahora siempre cierra en el horizonte, y esa fila coincide al bit con
        // `1 − éxito` — ambas cuentan el mismo conjunto de caminos (`failure_month_index.is_some()`).
        if cumulative_failure_by_age
            .last()
            .is_none_or(|(m, _)| *m < input.horizon_months)
        {
            let hit = failure_month.iter().filter(|f| f.is_some()).count() as f64;
            cumulative_failure_by_age.push((input.horizon_months, hit / n_f));
        }
    }

    sort_total(&mut months_below);
    let months_below_need_p50 = percentile_of_sorted(&months_below, 50) as u32;
    sort_total(&mut coverage_ratios);
    let withdrawal_to_need_ratio_p50 =
        (!coverage_ratios.is_empty()).then(|| percentile_of_sorted(&coverage_ratios, 50));

    Ok(McOutcome {
        seed: config.seed,
        paths: config.paths,
        percentiles: config.percentiles.clone(),
        horizon_months: input.horizon_months,
        net_worth,
        liquid_worth,
        success_probability,
        wilson_low,
        half_width_pp,
        failures_by_kind,
        cumulative_failure_by_age,
        months_below_need_p50,
        withdrawal_to_need_ratio_p50,
        any_volatility_declared,
    })
}

/// El índice de un [`PathFailure`] en [`McOutcome::failures_by_kind`] — los MISMOS índices que
/// [`crate::KIND_PORTFOLIO_DEPLETED`]/[`crate::KIND_INITIAL_RATE_EXCEEDED`]/
/// [`crate::KIND_RULE_BELOW_NEED`] de `solve_mc`, para que las dos capas cuenten en el mismo
/// orden. Se escribe aquí (y no se importa de `solve_mc`, que lo mantiene privado) porque es un
/// `match` de tres líneas sobre un enum exhaustivo: si `PathFailure` ganara una variante, el
/// compilador obligaría a actualizar las DOS copias, y eso es más barato que acoplar los módulos.
fn kind_index(kind: PathFailure) -> usize {
    match kind {
        PathFailure::PortfolioDepleted => KIND_PORTFOLIO_DEPLETED,
        PathFailure::InitialRateExceeded => KIND_INITIAL_RATE_EXCEEDED,
        PathFailure::RuleBelowNeed => KIND_RULE_BELOW_NEED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_is_the_order_statistic_and_never_interpolates() {
        // Muestra 1..=10: p10 = el 1.º, p50 = el 5.º, p90 = el 9.º, p100 no existe (cota 99).
        let sorted: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(percentile_of_sorted(&sorted, 10), 1.0);
        assert_eq!(percentile_of_sorted(&sorted, 50), 5.0);
        assert_eq!(percentile_of_sorted(&sorted, 90), 9.0);
        assert_eq!(percentile_of_sorted(&sorted, 99), 10.0);
        assert_eq!(percentile_of_sorted(&sorted, 1), 1.0);
        // Muestra de uno: todo percentil es ese uno.
        assert_eq!(percentile_of_sorted(&[7.0], 10), 7.0);
        assert_eq!(percentile_of_sorted(&[7.0], 90), 7.0);
        // El índice nunca se sale.
        for n in 1..50usize {
            for p in 1..=99u8 {
                assert!(nearest_rank_index(n, p) < n);
            }
        }
    }

    #[test]
    fn nearest_rank_is_monotone_in_p_so_bands_can_never_cross() {
        for n in [1usize, 2, 3, 7, 500, 5000] {
            let mut prev = 0usize;
            for p in 1..=99u8 {
                let i = nearest_rank_index(n, p);
                assert!(i >= prev, "n={n} p={p}: el índice retrocede");
                prev = i;
            }
        }
    }

    /// **Los momentos del normal**, medidos sobre 100 000 sorteos.
    ///
    /// Tolerancias DERIVADAS, no elegidas a ojo: con `n = 1e5`, el error típico de la media es
    /// `1/√n = 3,16e-3` y el de la varianza muestral `√(2/n) = 4,47e-3`. Se exige 4 σ en cada
    /// una (0,0127 y 0,0179), redondeado hacia arriba a 0,02 y 0,03. La fracción dentro de ±1σ
    /// debe rondar 0,6827 con error típico `√(0,6827·0,3173/n) = 1,5e-3`; se exige 0,01 (≈ 6,7 σ).
    #[test]
    fn box_muller_has_the_moments_of_a_standard_normal() {
        let n = 100_000usize;
        let mut rng = path_rng(0xF0F0_1234_5678_9ABC, 0);
        let (mut sum, mut sum_sq, mut within) = (0.0f64, 0.0f64, 0usize);
        let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
        for _ in 0..n {
            let z = standard_normal(&mut rng);
            assert!(
                z.is_finite(),
                "un normal no finito envenenaría el camino entero"
            );
            sum += z;
            sum_sq += z * z;
            if z.abs() <= 1.0 {
                within += 1;
            }
            min = min.min(z);
            max = max.max(z);
        }
        let mean = sum / n as f64;
        let var = (sum_sq - n as f64 * mean * mean) / (n as f64 - 1.0);
        let frac = within as f64 / n as f64;
        println!(
            "[box-muller] n={n}  media={mean:+.6} (|·| < 0,02)  var={var:.6} (|·−1| < 0,03)  \
             P(|z|≤1)={frac:.4} (0,6827 ± 0,01)  rango=[{min:.3}, {max:.3}]"
        );
        assert!(mean.abs() < 0.02, "media {mean}");
        assert!((var - 1.0).abs() < 0.03, "varianza {var}");
        assert!((frac - 0.6827).abs() < 0.01, "P(|z| ≤ 1) = {frac}");
        // Con 1e5 sorteos el máximo esperado ronda 4,3: si no se pasa de 3 el generador está
        // truncando la cola, y la cola es justo lo que Monte Carlo mide.
        assert!(
            max > 3.0 && min < -3.0,
            "las colas no aparecen: [{min}, {max}]"
        );
    }

    /// **El flujo de ChaCha8 está pineado.** Si una actualización de `rand_chacha` —o un cambio
    /// en `path_rng`, o en Box–Muller— moviera la secuencia, todas las bandas de todos los
    /// usuarios cambiarían en silencio. Falla aquí, no allí.
    ///
    /// Los valores se generaron con esta misma implementación y se copiaron con 17 dígitos
    /// significativos, que es lo que hace falta para reconstruir un `f64` sin pérdida.
    #[test]
    fn the_chacha_stream_is_pinned() {
        let mut rng = path_rng(42, 0);
        let z: Vec<f64> = (0..3).map(|_| standard_normal(&mut rng)).collect();
        println!("[pin] los tres primeros normales de (seed=42, path=0): {z:?}");
        assert_eq!(
            z,
            vec![
                1.5274819065768688,
                -0.04280065792124935,
                -0.020950906822275454
            ],
            "el flujo del RNG se ha movido: ninguna banda publicada antes de este cambio es \
             reproducible ya. Si el cambio es deliberado, actualiza el pin Y dilo en el CHANGELOG."
        );
    }

    /// Dos caminos distintos de la misma semilla no comparten flujo.
    #[test]
    fn each_path_gets_its_own_stream() {
        let a: Vec<f64> = {
            let mut r = path_rng(7, 0);
            (0..5).map(|_| standard_normal(&mut r)).collect()
        };
        let b: Vec<f64> = {
            let mut r = path_rng(7, 1);
            (0..5).map(|_| standard_normal(&mut r)).collect()
        };
        assert_ne!(a, b);
        // Y el camino 0 es el MISMO se pidan 1 camino o 2 000: reconstruirlo no lo mueve.
        let a2: Vec<f64> = {
            let mut r = path_rng(7, 0);
            (0..5).map(|_| standard_normal(&mut r)).collect()
        };
        assert_eq!(a, a2);
    }

    #[test]
    fn sigma_degrades_absurd_values_to_a_deterministic_asset() {
        assert_eq!(monthly_sigma(None), 0.0);
        assert_eq!(monthly_sigma(Some(0.0)), 0.0);
        assert_eq!(monthly_sigma(Some(-5.0)), 0.0);
        assert_eq!(monthly_sigma(Some(f64::NAN)), 0.0);
        assert_eq!(monthly_sigma(Some(f64::INFINITY)), 0.0);
        // 17 % anual ⇒ 17/100/√12 mensual.
        let s = monthly_sigma(Some(17.0)).unwrap_finite();
        assert!((s - 0.17 / 12f64.sqrt()).abs() < 1e-15);
    }

    /// Azucarillo local del test de arriba: hace explícito que se está midiendo un número finito.
    trait Finite {
        fn unwrap_finite(self) -> f64;
    }
    impl Finite for f64 {
        fn unwrap_finite(self) -> f64 {
            assert!(self.is_finite());
            self
        }
    }

    #[test]
    fn seed_for_is_a_pure_stable_function_of_the_two_ids() {
        assert_eq!(seed_for(1, 2), seed_for(1, 2));
        assert_ne!(seed_for(1, 2), seed_for(2, 1), "el orden importa");
        assert_ne!(seed_for(1, 2), seed_for(1, 3));
        // Un solo bit de diferencia debe cambiar la semilla entera (avalancha del finalizador).
        let a = seed_for(0, 0);
        let b = seed_for(0, 1);
        let differing_bits = (a ^ b).count_ones();
        println!("[seed_for] 0/0 = {a:#018x}  0/1 = {b:#018x}  bits distintos = {differing_bits}");
        assert!(
            differing_bits > 16,
            "avalancha pobre: solo {differing_bits} bits cambian"
        );
    }

    #[test]
    fn config_is_validated_before_anything_is_drawn() {
        assert_eq!(
            PathEngine::new(
                &dummy_input(),
                &[None],
                &McConfig {
                    paths: 0,
                    ..Default::default()
                }
            )
            .err(),
            Some(McError::InvalidPaths(0))
        );
        assert_eq!(
            PathEngine::new(
                &dummy_input(),
                &[None],
                &McConfig {
                    paths: MAX_PATHS + 1,
                    ..Default::default()
                }
            )
            .err(),
            Some(McError::InvalidPaths(MAX_PATHS + 1))
        );
        assert_eq!(
            PathEngine::new(
                &dummy_input(),
                &[None],
                &McConfig {
                    percentiles: vec![],
                    ..Default::default()
                }
            )
            .err(),
            Some(McError::InvalidPercentiles)
        );
        assert_eq!(
            PathEngine::new(
                &dummy_input(),
                &[None],
                &McConfig {
                    percentiles: vec![0],
                    ..Default::default()
                }
            )
            .err(),
            Some(McError::InvalidPercentiles)
        );
        assert_eq!(
            PathEngine::new(&dummy_input(), &[], &McConfig::default()).err(),
            Some(McError::VolatilityLengthMismatch(0, 1))
        );
    }

    /// Un input mínimo de un activo: lo justo para ejercitar la validación.
    fn dummy_input() -> ProjectionInput {
        use futurefin_engine::{PhasePlan, SimAsset};
        use rust_decimal::Decimal;
        ProjectionInput {
            ref_date: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            horizon_months: 12,
            annual_inflation_percent: Decimal::ZERO,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            income_regular_monthly: Decimal::ZERO,
            expense_regular_monthly: Decimal::ZERO,
            assets: vec![SimAsset {
                id: uuid::Uuid::from_u128(1),
                value: Decimal::from(1_000),
                purchase_price: None,
                is_liquid: true,
                expected_annual_return_percent: None,
            }],
            allocation_rules: Vec::new(),
            liabilities: Vec::new(),
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 12],
            phase_plan: PhasePlan::classic(Decimal::ZERO, Decimal::ZERO),
            fire_target: None,
        }
    }
}
