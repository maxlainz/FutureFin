//! **La capa de Monte Carlo** (WP6 de 5.0.0, §B.5/§B.6 del plan de la issue #207, D11/D22/D23/D25).
//!
//! # El modelo de retornos, entero y sin letra pequeña
//!
//! Un **shock de mercado COMÚN** por mes (D11). Para cada mes `k` del horizonte se sortea **un
//! solo** normal estándar `z_k ~ N(0,1)` y todos los activos lo viven a la vez, escalado por su
//! propia volatilidad:
//!
//! ```text
//!   σ_i  = annual_volatility_percent_i / 100 / √12          (volatilidad MENSUAL del activo i)
//!   f_ik = m_i · exp(σ_i·z_k − σ_i²/2)                       (factor de crecimiento del mes k)
//!   σ_i = 0  ⇒  f_ik = m_i  exactamente
//! ```
//!
//! `m_i` es el multiplicador determinista del activo — la **raíz doceava del motor**
//! ([`futurefin_engine::monthly_growth_multiplier`]), no una segunda copia escrita aquí: si esa
//! conversión anual→mensual cambiara, «volatilidad cero» dejaría de significar «el camino
//! determinista» sin que nada fallara.
//!
//! El término `−σ_i²/2` es la corrección de Itô, y su efecto es **exacto, no aproximado**: para
//! `X ~ N(0, σ²)`, `E[exp(X)] = exp(σ²/2)`, luego
//!
//! ```text
//!   E[f_ik] = m_i · exp(σ_i²/2) · exp(−σ_i²/2) = m_i
//! ```
//!
//! Es decir: **la media aritmética del factor mensual es el factor determinista**, y como los
//! `z_k` son independientes, `E[Π_{12} f] = m^12 = 1 + rentabilidad anual declarada`. La
//! rentabilidad esperada que el usuario escribe en el activo es la aritmética, que es lo que
//! significa «rentabilidad media» en la literatura que el issue cita. La **geométrica** —la que
//! el hogar cobra de verdad— sale más baja, `≈ (1 + r)·exp(−σ²/2)` anualizada, y esa diferencia
//! no es un error del modelo: es el coste de la volatilidad, y es justo lo que Monte Carlo
//! existe para enseñar. `mc_mean_growth_matches_expected` mide la primera; la segunda se ve en
//! la banda p50, que queda por DEBAJO de la línea determinista.
//!
//! # Lo que este modelo NO representa — dicho aquí para que nadie lo suponga
//!
//! - **Colas gruesas.** El shock es log-normal. Los mercados reales tienen curtosis: octubre de
//!   1987 fue −20 % en un día, unas 20 desviaciones típicas bajo este modelo (probabilidad ≈ 0).
//!   La probabilidad de ruina que sale de aquí es, por construcción, **optimista en la cola**.
//! - **Autocorrelación / reversión a la media.** Los `z_k` son independientes mes a mes. Ni hay
//!   momentum ni hay reversión, y por tanto **no hay ciclos**: la dispersión a 35 años crece con
//!   `√H` limpia. La evidencia histórica apunta a algo de reversión a largo plazo, que ESTRECHARÍA
//!   las bandas lejanas. Esta ausencia tiene una consecuencia MEDIDA y contraintuitiva: el colchón
//!   de caja (P4) **empeora** el plan en este modelo (ver
//!   `mc_cash_buffer_costs_return_without_buying_safety_in_this_model`). Sin autocorrelación, un
//!   mes malo no dice nada del siguiente, así que no hay «mala racha que esperar sentado»: el
//!   colchón no compra información y su lastre —dinero fuera del mercado— se cobra entero.
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
//!   superávit con las reglas declaradas, igual que en el camino determinista. La ÚNICA
//!   recolocación entre activos es el relleno del colchón (P4), y solo cuando se pide.
//!
//! # El colchón de caja (P4, §B.6)
//!
//! Con [`McConfig::cash_buffer_months`] declarado, el activo líquido de menor rentabilidad hace
//! de colchón: la retirada del mes ya sale de él sola (es el primero del orden de drenaje) y en
//! los meses de **shock positivo** (`z_k > 0`) se rellena hasta `n` meses de gasto vendiendo del
//! resto de la cartera. El relleno lo ejecuta el motor (`refill_cash_buffer_g`), así que es una
//! venta de verdad: pasa por el gross-up, paga su plusvalía por tramos y baja la base de coste
//! del activo vendido.
//!
//! **No se instala** —y [`McOutcome::buffer_active`] lo dice— si no hay activo líquido que lo
//! albergue o si ningún activo declara volatilidad: sin volatilidad, `z_k` no mueve ningún
//! retorno y rellenar «en los meses buenos» sería trasvasar valor guiándose por un shock que no
//! afecta a nada.
//!
//! # De aquí no sale un euro
//!
//! Es la regla del crate (ver el doc de [`crate`]) y aquí es donde muerde: [`McOutcome`] son
//! **probabilidades, percentiles y contadores**. Ninguna de sus cifras se publica como un KPI
//! monetario. Las bandas se dibujan; el patrimonio, el objetivo y la aportación necesaria siguen
//! saliendo del camino `Decimal`.

use rand_chacha::rand_core::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use futurefin_engine::{
    monthly_growth_multiplier, safe_cash_buffer_index, simulate, CashBufferPlan, EngineError,
    EngineWarning, ProjectionInput, RetirementTrigger, SimInput, SimOutput,
};

use crate::F64Money;

// =================================================================================================
// Configuración
// =================================================================================================

/// Cota dura de caminos por ejecución. **No es un ajuste de producto**: es el punto a partir del
/// cual la memoria de las bandas (`2 · caminos · (horizonte+1) · 8 bytes`, ver
/// [`project_percentile_bands`]) y el tiempo dejan de caber en el presupuesto de un request. Con
/// 5 000 caminos y 840 meses son ~67 MB y varios segundos: el plan fija 2 000 por HTTP y 1 000
/// por MCP, y esos topes los aplica el handler, no este crate.
pub const MAX_PATHS: u32 = 5_000;

/// Caminos por defecto (§B.5 del plan).
pub const DEFAULT_PATHS: u32 = 500;

/// Percentiles por defecto: la banda p10/p50/p90 que la sección «Riesgo» dibuja (D28).
pub const DEFAULT_PERCENTILES: [u8; 3] = [10, 50, 90];

/// Cada cuántos meses se publica la probabilidad de agotamiento desde la jubilación efectiva
/// (§B.5: «cada 5 años»). El caller traduce meses a edades — este crate no sabe de fechas de
/// nacimiento.
pub const DEPLETION_STEP_MONTHS: u32 = 60;

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
    /// **El colchón de caja** (P4, §B.6): cuántos meses de gasto se intentan mantener en el
    /// activo líquido de menor rentabilidad.
    ///
    /// La retirada sale de ese activo sola —es el primero del orden de drenaje— y el colchón se
    /// **rellena** vendiendo del resto de la cartera **solo en los meses de shock positivo**
    /// (`z_k > 0`): se vende después de que el mercado suba, no después de que baje. El relleno es
    /// una venta de verdad y paga su plusvalía (`refill_cash_buffer_g` en el motor).
    ///
    /// `None` = sin colchón. Y `Some(n)` **no garantiza** que se simule: hace falta además un
    /// activo líquido que lo albergue y volatilidad declarada de la que protegerse. Lo dice
    /// [`McOutcome::buffer_active`].
    pub cash_buffer_months: Option<u32>,
}

impl Default for McConfig {
    fn default() -> Self {
        McConfig {
            seed: 0,
            paths: DEFAULT_PATHS,
            percentiles: DEFAULT_PERCENTILES.to_vec(),
            cash_buffer_months: None,
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
struct PathEngine {
    sim: SimInput<F64Money>,
    /// `m_i`, el multiplicador determinista de cada activo.
    base: Vec<F64Money>,
    /// `σ_i` mensual de cada activo.
    sigmas: Vec<f64>,
    seed: u64,
    buf: Option<Vec<Vec<F64Money>>>,
    /// **P4**: `(índice del activo colchón, meses de gasto objetivo)`. `None` = no se simula
    /// colchón, y entonces `sim.cash_buffer` nunca se rellena.
    buffer: Option<(usize, F64Money)>,
    /// Por qué NO se instaló el colchón. `None` ⟺ `buffer.is_some()`.
    buffer_inactive_reason: Option<BufferInactiveReason>,
    /// Autorizaciones de relleno del mes (`z_{k−1} > 0`, NO anticipativas), reutilizadas camino a
    /// camino: viajan al motor con `mem::take` y vuelven después de simular, igual que el buffer
    /// de factores. Vacío cuando no hay colchón.
    refill_buf: Vec<bool>,
}

/// **Por qué el colchón de caja (P4) no se está simulando.** Nunca es `None` cuando
/// [`McOutcome::buffer_active`] es `false`: un colchón inactivo sin motivo es un número que el
/// usuario pidió y no recibió, sin explicación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferInactiveReason {
    /// No se pidió (`McConfig::cash_buffer_months = None`).
    NotRequested,
    /// Ningún activo declara volatilidad: no hay riesgo de secuencia del que protegerse, y
    /// rellenar «en los meses buenos» sería trasvasar valor y pagar plusvalía guiándose por un
    /// shock que no mueve nada. El resultado es BIT A BIT el de no pedirlo.
    NoVolatility,
    /// No hay ningún activo **líquido con σ = 0** donde alojarlo. Un colchón volátil no protege
    /// de nada, y vender un activo ilíquido para financiarlo es exactamente el desastre que el
    /// colchón dice evitar: antes que eso, no se instala.
    NoSafeLiquidAsset,
}

impl BufferInactiveReason {
    /// El literal público, estable, que la API y la ayuda de la UI citan.
    pub fn code(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::NoVolatility => "no_volatility",
            Self::NoSafeLiquidAsset => "no_safe_liquid_asset",
        }
    }
}

impl PathEngine {
    fn new(
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
        let months = input.horizon_months as usize;
        let buf = Some(vec![vec![F64Money(0.0); sim.assets.len()]; months]);
        // **P4: cuándo se instala el colchón, y por qué las tres condiciones.**
        //
        // 1. `cash_buffer_months` declarado — el usuario lo pidió.
        // 2. Existe un activo LÍQUIDO que pueda albergarlo (`cash_buffer_index`, el motor decide
        //    cuál: el líquido de menor rentabilidad, que es el primero del orden de drenaje). Sin
        //    activo líquido no hay colchón posible.
        // 3. **Hay volatilidad declarada.** Sin ella, `z_k` se sigue sorteando (el flujo del RNG
        //    no depende de los datos) pero no mueve ningún retorno: rellenar «en los meses buenos»
        //    sería trasvasar valor y pagar plusvalías guiándose por un shock que no afecta a nada.
        //    Con σ=0 en toda la cartera no hay riesgo de secuencia del que protegerse, así que no
        //    se instala colchón y el resultado es BIT A BIT el de no pedirlo.
        //
        // La condición 3 es también lo que mantiene la puerta de degeneración: `σ=0 ⇒ la banda es
        // la línea determinista`, colchón pedido o no.
        let any_volatility = sigmas.iter().any(|s| *s > 0.0);
        // **El colchón exige un activo LÍQUIDO Y SIN RIESGO** (corrección de la revisión
        // adversarial). `cash_buffer_index` sale del orden de drenaje, que no sabe de
        // volatilidad, y en una cartera «RV líquida + vivienda» elegía la RENTA VARIABLE como
        // colchón: un colchón con σ = 17 % no es un colchón, es la misma cartera con más
        // impuestos. Si no hay dónde ponerlo, no se instala y se dice POR QUÉ.
        let risk_free: Vec<bool> = sigmas.iter().map(|s| *s == 0.0).collect();
        let (buffer, buffer_inactive_reason) = match config.cash_buffer_months {
            None => (None, Some(BufferInactiveReason::NotRequested)),
            Some(_) if !any_volatility => (None, Some(BufferInactiveReason::NoVolatility)),
            Some(n) => match safe_cash_buffer_index(&sim.assets, &risk_free) {
                Some(i) => (Some((i, F64Money(f64::from(n)))), None),
                None => (None, Some(BufferInactiveReason::NoSafeLiquidAsset)),
            },
        };
        let refill_buf = if buffer.is_some() {
            vec![false; months]
        } else {
            Vec::new()
        };
        Ok(PathEngine {
            sim,
            base,
            sigmas,
            seed: config.seed,
            buf,
            buffer,
            buffer_inactive_reason,
            refill_buf,
        })
    }

    /// ¿Hay algún activo con volatilidad declarada? Con `false`, todos los caminos son el camino
    /// determinista y la banda es una línea — lo que la SPA avisa (§G).
    fn any_volatility(&self) -> bool {
        self.sigmas.iter().any(|s| *s > 0.0)
    }

    /// Ejecuta el camino `path_index`: sortea sus factores en el buffer, los inyecta por
    /// `growth_overrides` y llama al MISMO `simulate` que produce el camino determinista.
    fn run(&mut self, path_index: u32) -> Result<SimOutput<F64Money>, EngineError> {
        let mut buf = self
            .buf
            .take()
            .expect("el buffer siempre vuelve al final de `run`");
        let mut rng = path_rng(self.seed, path_index);
        // El shock del mes ANTERIOR, que es el único que el hogar ha podido observar cuando
        // decide rellenar. Arranca en `false`: el mes 1 no tiene pasado.
        let mut prev_z_positive = false;
        for (k, row) in buf.iter_mut().enumerate() {
            // UN shock por mes, sorteado SIEMPRE — también con la cartera entera a σ=0. Que el
            // flujo del RNG no dependa de los datos es lo que hace comparables dos ejecuciones
            // sobre carteras distintas con la misma semilla.
            let z = standard_normal(&mut rng);
            // **P4, relleno NO ANTICIPATIVO** (corrección de la revisión adversarial). El mes `k`
            // se autoriza con el shock que YA OCURRIÓ, `z_{k−1}`; el mes 1 no rellena nunca
            // porque no hay mes anterior.
            //
            // Antes se usaba el `z` del propio mes, y el relleno se ejecuta ANTES del
            // crecimiento (`sim_core.rs`, bloque del colchón justo delante del paso de
            // crecimiento): eso vendía renta variable **al precio de antes de la subida, en el
            // mes en que iba a subir**. Es información del futuro, y se pagaba: con la cartera
            // volátil al 6,5 % y 10.000 caminos, el éxito bajaba de 0,8077 (regla `z_{k−1}`) a
            // 0,7828 (regla `z_k`), −2,5 pp; y en un McNemar pareado sobre las MISMAS sendas,
            // 249 caminos se arruinaban solo bajo la regla anticipativa y **ninguno** solo bajo
            // la retardada.
            if let Some(flag) = self.refill_buf.get_mut(k) {
                *flag = prev_z_positive;
            }
            prev_z_positive = z > 0.0;
            for (i, cell) in row.iter_mut().enumerate() {
                let s = self.sigmas[i];
                *cell = if s == 0.0 {
                    // Rama explícita, no `exp(0)`: `σ=0` significa «el camino determinista», y
                    // eso se escribe, no se deduce de que `1.0` sea neutro.
                    self.base[i]
                } else {
                    F64Money(self.base[i].0 * (s * z - 0.5 * s * s).exp())
                };
            }
        }
        self.sim.growth_overrides = Some(buf);
        if let Some((buffer_index, target_months)) = self.buffer {
            self.sim.cash_buffer = Some(CashBufferPlan {
                buffer_index,
                target_months,
                refill_months: core::mem::take(&mut self.refill_buf),
            });
        }
        let out = simulate(&self.sim);
        // Los dos buffers vuelven TAMBIÉN si la simulación falló: el `expect` de arriba y el
        // `get_mut` del sorteo dependen de ello.
        self.buf = self.sim.growth_overrides.take();
        if let Some(cb) = self.sim.cash_buffer.take() {
            self.refill_buf = cb.refill_months;
        }
        out
    }
}

/// **Un solo camino de Monte Carlo**, con toda su salida del motor.
///
/// Existe para lo que las bandas no pueden dar: verificar la reproducibilidad camino a camino,
/// medir la media del terminal contra `E[factor] = m` (`mc_mean_growth_matches_expected`) y
/// permitir que un caller inspeccione una realización concreta. Para dibujar bandas, use
/// [`project_percentile_bands`]: esta función reconstruye la maquinaria en cada llamada.
pub fn run_path(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    config: &McConfig,
    path_index: u32,
) -> Result<SimOutput<F64Money>, McError> {
    let mut engine = PathEngine::new(input, volatilities, config)?;
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
    /// **D22**: fracción de caminos en los que la cartera NO se agota nunca antes del horizonte
    /// (`assets_depleted_month_index.is_none()`). Las pensiones y las fases ya están dentro de la
    /// simulación, así que esto es el éxito del PLAN, no el de una regla de retirada.
    ///
    /// El recorte de una regla (`withdrawal_shortfall`) **no es fracaso** (D24) y se publica
    /// aparte en [`Self::months_below_need_p50`] y [`Self::withdrawal_to_need_ratio_p50`].
    pub success_probability: f64,
    /// **Fracción de caminos que NO se jubilan** dentro del horizonte
    /// (`retirement_month_index == None`). Con trigger por EDAD es 0 por construcción.
    ///
    /// Se publica porque es el denominador escondido del éxito: un plan por cruce con una
    /// probabilidad de éxito alta y un tercio de caminos que no se jubilan nunca no es un buen
    /// plan, es un plan que no ocurre.
    pub never_retired_probability: f64,
    /// Éxito **entre los caminos que sí se jubilan**: de los que llegan a la jubilación, cuántos
    /// no agotan la cartera. `None` si ningún camino se jubila.
    ///
    /// Junto a [`Self::success_probability`] separa las dos preguntas que D22 mezclaba: «¿ocurre
    /// el plan?» y «¿aguanta?».
    pub success_given_retired: Option<f64>,
    /// Probabilidad ACUMULADA de agotamiento en `(mes, p)`, cada
    /// [`DEPLETION_STEP_MONTHS`] meses desde la jubilación efectiva. `p` es la fracción de
    /// caminos con `assets_depleted_month_index ≤ mes`. El caller traduce meses a edades.
    ///
    /// El ancla es la jubilación efectiva del camino DETERMINISTA; si ese camino no se jubila
    /// dentro del horizonte, la mediana de los caminos sorteados; si no se jubila ninguno, el
    /// vector va **vacío** — sin jubilación no hay «probabilidad de agotar a los 75».
    pub depletion_probability_by_age: Vec<(u32, f64)>,
    /// Percentiles del mes de jubilación EFECTIVA, alineados con [`Self::percentiles`], **solo
    /// para planes que se jubilan por cruce**; `None` cuando el trigger es por edad (ahí el mes
    /// es un dato, no una distribución).
    ///
    /// Un `None` dentro del vector es un percentil que cae en un camino que **no se jubila** en
    /// todo el horizonte: los caminos sin jubilación ordenan los últimos, así que un `None` en
    /// p90 dice «uno de cada diez planes no llega nunca».
    pub retirement_month_index_percentiles: Option<Vec<Option<u32>>>,
    /// Solo para planes con jubilación por EDAD: fracción de caminos que llegan a `R` con el
    /// líquido por debajo del objetivo de `R−1` (D17, el «aviso rojo grande»). `None` si el plan
    /// no se jubila por edad.
    pub underfunded_probability: Option<f64>,
    /// Mediana, entre los caminos, del número de meses jubilados con recorte
    /// (`withdrawal_shortfall > 0`). Con `fixed_real` es 0 por construcción.
    pub months_below_need_p50: u32,
    /// Mediana, entre los caminos, de `Σ withdrawal / Σ (withdrawal + withdrawal_shortfall)`
    /// sobre los meses jubilados: **qué fracción de la necesidad cubrió la regla**. `1.0` = la
    /// cubrió entera. `None` si ningún camino tiene meses jubilados con denominador positivo.
    pub withdrawal_to_need_ratio_p50: Option<f64>,
    /// **P4 (§B.6): ¿se SIMULÓ el colchón?**
    ///
    /// `true` solo si se cumplieron las tres condiciones: [`McConfig::cash_buffer_months`]
    /// declarado, un activo líquido que pueda albergarlo y volatilidad declarada de la que
    /// protegerse. Un `false` con `cash_buffer_months = Some(n)` no es un fallo: es que en esta
    /// cartera el colchón no significa nada, y el resultado es idéntico al de no pedirlo.
    ///
    /// Con `false`, [`Self::buffer_refills_p50`] y [`Self::buffer_refill_net_total_p50`] son
    /// `None` — «no se midió», que no es lo mismo que «cero rellenos».
    pub buffer_active: bool,
    /// Por qué NO se simuló el colchón. `None` ⟺ [`Self::buffer_active`]. Nunca es `None` con
    /// `buffer_active = false`: el usuario que pidió un colchón y no lo tuvo merece el motivo.
    pub buffer_inactive_reason: Option<BufferInactiveReason>,
    /// Mediana, entre los caminos, del NÚMERO de meses con relleno efectivo del colchón.
    /// `None` ⟺ `!buffer_active`.
    ///
    /// Es un contador, no euros: cuántas veces el plan tuvo que reponer la caja.
    pub buffer_refills_p50: Option<u32>,
    /// Mediana, entre los caminos, del **total movido al colchón** en todo el horizonte
    /// (`Σ_k buffer_refill_net[k]`). `None` ⟺ `!buffer_active`.
    ///
    /// La regla del crate («de aquí no sale un euro») sigue en pie: esto es la MEDIANA DE UN
    /// TOTAL sobre una muestra sorteada, es decir un estadístico de la dispersión, no una cifra
    /// contable del hogar. Ningún KPI monetario de la app puede salir de aquí: el trasvase real
    /// del camino que la app dibuja lo da el motor `Decimal`.
    pub buffer_refill_net_total_p50: Option<f64>,
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
/// ordenación. Memoria: `2 · paths · (horizonte+1) · 8` bytes para las muestras (67 MB en el
/// extremo de 5 000 caminos × 840 meses), más el buffer de factores
/// (`meses · activos · 8` bytes) y las series de una simulación viva. Los números medidos están
/// en `tests/timing_mc.rs`.
///
/// # Determinismo
///
/// Dos ejecuciones con la misma entrada y la misma [`McConfig`] devuelven [`McOutcome`]s **bit a
/// bit iguales**: el sorteo depende solo de `(seed, path_index)`, el orden de los caminos es el
/// del bucle y el percentil es un índice entero sobre una muestra ordenada con un orden total.
/// Lo pinea `mc_same_seed_bit_identical`.
pub fn project_percentile_bands(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    config: &McConfig,
) -> Result<McOutcome, McError> {
    let mut engine = PathEngine::new(input, volatilities, config)?;
    let any_volatility_declared = engine.any_volatility();

    let n = config.paths as usize;
    let len = input.horizon_months as usize + 1;

    // Muestras transpuestas —`[mes][camino]`— porque lo que hay que ordenar es un mes entero.
    let mut nw_samples: Vec<Vec<f64>> = vec![vec![0.0; n]; len];
    let mut lq_samples: Vec<Vec<f64>> = vec![vec![0.0; n]; len];

    let mut depleted: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut retired_at: Vec<Option<u32>> = Vec::with_capacity(n);
    let mut underfunded_paths = 0usize;
    let mut months_below: Vec<f64> = Vec::with_capacity(n);
    let mut coverage_ratios: Vec<f64> = Vec::with_capacity(n);
    // P4: cuántas veces se rellenó el colchón y cuánto se movió, por camino.
    let buffer_active = engine.buffer.is_some();
    let mut refill_counts: Vec<f64> = Vec::with_capacity(if buffer_active { n } else { 0 });
    let mut refill_totals: Vec<f64> = Vec::with_capacity(if buffer_active { n } else { 0 });

    for p in 0..n {
        let out = engine.run(p as u32)?;
        debug_assert_eq!(out.net_worth.len(), len);
        for k in 0..len {
            nw_samples[k][p] = out.net_worth[k].0;
            lq_samples[k][p] = out.liquid_worth[k].0;
        }
        depleted.push(out.assets_depleted_month_index);
        retired_at.push(out.retirement_month_index);
        if out
            .warnings
            .contains(&EngineWarning::RetireAtAgeUnderfunded)
        {
            underfunded_paths += 1;
        }

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
                // 8,8 % de lo que necesitaban. Lo que faltaba estaba en la otra magnitud.
                let u = out.unmet_need[k].0;
                if s + u > 0.0 {
                    below += 1;
                }
                sum_w += w;
                sum_need += w + s + u;
            }
        }
        months_below.push(f64::from(below));
        if sum_need > 0.0 {
            coverage_ratios.push(sum_w / sum_need);
        }
        if buffer_active {
            refill_counts.push(f64::from(out.buffer_refill_months));
            refill_totals.push(out.buffer_refill_net.iter().map(|v| v.0).sum());
        }
    }

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
    // **Éxito y jubilación** (hallazgo #7 de la revisión, decisión de modelo). D22 decía «la
    // cartera no se agota nunca», y con un trigger por CRUCE eso premiaba al hogar que no se
    // jubila jamás: un camino que trabaja hasta los 105 años sin llegar al objetivo nunca drena
    // y por tanto nunca se agota. En el hogar medido, el 33,1 % de los caminos no se jubilaba y
    // los 1.000 se contaban como éxito: 0,960 publicado frente a 0,940 entre los que sí se
    // jubilan.
    //
    // Éxito = **el plan ocurre Y aguanta**: el hogar se jubila dentro del horizonte (o el plan es
    // por edad, y entonces la jubilación es un dato, no un suceso) y la cartera no se agota. La
    // fracción que no se jubila se publica aparte, y el condicional también.
    let age_triggered = input.phase_plan.retirement_trigger.forced_month().is_some();
    let never_retired = retired_at.iter().filter(|r| r.is_none()).count();
    let never_retired_probability = never_retired as f64 / n_f;
    let success_probability = (0..n)
        .filter(|&p| (age_triggered || retired_at[p].is_some()) && depleted[p].is_none())
        .count() as f64
        / n_f;
    let retired_count = n - never_retired;
    let success_given_retired = (retired_count > 0).then(|| {
        (0..n)
            .filter(|&p| retired_at[p].is_some() && depleted[p].is_none())
            .count() as f64
            / retired_count as f64
    });

    // Ancla de la tabla de agotamiento: la jubilación del camino DETERMINISTA (la que la app
    // dibuja) y, a falta de ella, la mediana de los sorteados.
    let deterministic = crate::simulate_f64(input)?;
    let mut retired_sorted = retired_at.clone();
    // Los caminos que no se jubilan ordenan los ÚLTIMOS: «nunca» es el peor mes posible.
    retired_sorted.sort_by(|a, b| match (a, b) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => core::cmp::Ordering::Less,
        (None, Some(_)) => core::cmp::Ordering::Greater,
        (None, None) => core::cmp::Ordering::Equal,
    });
    let anchor = deterministic
        .retirement_month_index
        .or(retired_sorted[nearest_rank_index(n, 50)]);

    let mut depletion_probability_by_age = Vec::new();
    if let Some(a) = anchor {
        let mut m = a;
        while m <= input.horizon_months {
            let hit = depleted
                .iter()
                .filter(|d| d.is_some_and(|x| x <= m))
                .count() as f64;
            depletion_probability_by_age.push((m, hit / n_f));
            m += DEPLETION_STEP_MONTHS;
        }
        // **La última fila es el HORIZONTE** (hallazgo #8 de la revisión). La rejilla avanza de
        // 60 en 60 desde el ancla y se paraba en el último múltiplo que cabía: con ancla 655 y
        // horizonte 840, la tabla terminaba en el mes 835 y dejaba 5 meses fuera sin decirlo.
        // Ahora siempre cierra en el horizonte, que es la fila que el usuario lee como «al final
        // del plan».
        if depletion_probability_by_age
            .last()
            .is_none_or(|(m, _)| *m < input.horizon_months)
        {
            let hit = depleted.iter().filter(|d| d.is_some()).count() as f64;
            depletion_probability_by_age.push((input.horizon_months, hit / n_f));
        }
    }

    // Percentiles del mes de jubilación: solo tienen sentido si la jubilación la decide el CRUCE.
    // Con `crossing_is_reading_only` el cruce no jubila (D17) y el mes vuelve a ser un dato.
    let plan = &input.phase_plan;
    let by_crossing = matches!(plan.retirement_trigger, RetirementTrigger::LiquidCrossing)
        && !plan.crossing_is_reading_only;
    let retirement_month_index_percentiles = by_crossing.then(|| {
        config
            .percentiles
            .iter()
            .map(|&p| retired_sorted[nearest_rank_index(n, p)])
            .collect()
    });

    // Infra-financiación: **la lee el propio motor**, no esta capa. `RetireAtAgeUnderfunded` se
    // emite dentro del bucle comparando `L(R−1) < T(R−1)` con los mismos escalares que el cruce
    // acaba de usar; recalcularlo aquí sería una segunda definición de «no llego», y la segunda
    // definición es la que se queda atrás.
    let underfunded_probability = plan
        .retirement_trigger
        .forced_month()
        .map(|_| underfunded_paths as f64 / n_f);

    sort_total(&mut months_below);
    let months_below_need_p50 = percentile_of_sorted(&months_below, 50) as u32;
    sort_total(&mut coverage_ratios);
    let withdrawal_to_need_ratio_p50 =
        (!coverage_ratios.is_empty()).then(|| percentile_of_sorted(&coverage_ratios, 50));

    // P4: las dos lecturas del colchón, `None` cuando no se simuló (un cero diría «se simuló y
    // nunca se rellenó», que es otra cosa).
    sort_total(&mut refill_counts);
    sort_total(&mut refill_totals);
    let buffer_refills_p50 = buffer_active.then(|| percentile_of_sorted(&refill_counts, 50) as u32);
    let buffer_refill_net_total_p50 =
        buffer_active.then(|| percentile_of_sorted(&refill_totals, 50));

    Ok(McOutcome {
        seed: config.seed,
        paths: config.paths,
        percentiles: config.percentiles.clone(),
        horizon_months: input.horizon_months,
        net_worth,
        liquid_worth,
        success_probability,
        never_retired_probability,
        success_given_retired,
        depletion_probability_by_age,
        retirement_month_index_percentiles,
        underfunded_probability,
        months_below_need_p50,
        withdrawal_to_need_ratio_p50,
        buffer_active,
        buffer_inactive_reason: engine.buffer_inactive_reason,
        buffer_refills_p50,
        buffer_refill_net_total_p50,
        any_volatility_declared,
    })
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
