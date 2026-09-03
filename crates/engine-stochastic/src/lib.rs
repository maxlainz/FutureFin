//! **Evaluación estocástica del motor de proyección** (5.0.0, §B.4/§B.5 del plan de la issue #207).
//!
//! # Qué es este crate y qué NO es
//!
//! Este crate **no tiene bucle de simulación**. Tiene un TIPO —[`F64Money`]— que implementa
//! [`MoneyOps`], y con él instancia el bucle de `futurefin-engine`. Toda la matemática financiera
//! (fases, cascada, fiscalidad, drenaje, objetivo con puente, reglas de retirada) es exactamente
//! la misma función que produce el camino determinista: si alguien cambia el modelo, cambia en un
//! solo sitio y los dos caminos lo ven a la vez.
//!
//! Existe porque Monte Carlo necesita **miles** de caminos y el camino en `Decimal` cuesta ~12 ms
//! por proyección de 840 meses: 500 caminos serían seis segundos por request.
//!
//! # LA REGLA: de aquí NO sale un euro
//!
//! **Ninguna salida de este crate se publica como un KPI monetario.** Lo que sale son magnitudes
//! ESTADÍSTICAS —probabilidad de éxito, percentiles de una banda, probabilidad de agotamiento por
//! edad— y ahí un error relativo de 1e-15 no cambia ninguna decisión. El patrimonio, el objetivo
//! FIRE, la aportación necesaria y cualquier cifra en euros de la app salen del camino
//! `Decimal` del motor, que es exacto y sigue siendo el único que la API publica como dinero.
//!
//! Es la salvaguarda con la que la arqueología (§2.9, campaña #4) readmite la coma flotante: el
//! freezer `crates_engine_src_has_no_f64_outside_comments` de `crates/engine` **no se ha tocado**
//! ni se le ha añadido una excepción. La coma flotante vive aquí y solo aquí.
//!
//! # Las políticas del tipo, todas declaradas
//!
//! [`MoneyOps`] obliga a cada implementación a declarar cómo compara, cuándo se rinde y cuánto
//! pierde. Las de [`F64Money`] están en su propio doc-comment, una por una, con el porqué. La que
//! más importa es [`F64Money::gains_equal`]: la selección uniforme-vs-mixta de la fracción de
//! plusvalía se decide con una IGUALDAD, y una igualdad exacta en coma flotante haría que dos
//! activos «con la misma `g`» tomaran caminos fiscales distintos por el último bit.
//!
//! # La capa de Monte Carlo
//!
//! El módulo [`mc`] es lo que ese tipo hace posible: [`project_percentile_bands`] corre miles de
//! caminos del MISMO bucle con los factores de crecimiento sorteados
//! ([`SimInput::growth_overrides`](futurefin_engine::SimInput::growth_overrides)) y publica
//! bandas puntuales y probabilidades. El modelo de retornos, la semilla estable por usuario
//! ([`seed_for`]) y —sobre todo— **la lista de lo que el modelo NO representa** (colas gruesas,
//! autocorrelación, correlación imperfecta entre activos, bootstrap histórico) están escritos en
//! el doc de ese módulo, no en un comentario suelto: un modelo estocástico sin sus supuestos
//! declarados es un generador de números que parecen ciertos.

mod mc;

pub use mc::{
    project_percentile_bands, run_path, seed_for, McConfig, McError, McOutcome,
    DEFAULT_PATHS, DEFAULT_PERCENTILES, DEPLETION_STEP_MONTHS, MAX_PATHS,
};

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use futurefin_engine::{
    monthly_growth_multiplier, simulate, EngineError, MoneyOps, ProjectionInput, SimInput,
    SimOutput,
};

/// **Tolerancia de igualdad de la fracción de plusvalía gravable** (`g`) en el camino de coma
/// flotante.
///
/// El motor cortocircuita a la vía fiscal ESCALAR cuando todas las `g` de los activos vendibles
/// coinciden, y solo entonces; si difieren, recorre el mapa lineal a trozos por tramos. En
/// `Decimal` esa igualdad es exacta y la decisión es determinista. En coma flotante, `g_i` sale de
/// `1 − b_i/v_i` tras cientos de meses de aritmética: dos activos que en el camino exacto tienen
/// la MISMA `g` pueden diferir aquí en el último bit y mandar la simulación por la vía mixta, que
/// es algebraicamente igual pero numéricamente distinta.
///
/// `1e-12` es holgado frente al error acumulado esperable de una fracción en `[0, 1]` (~1e-15 por
/// operación) y estrecho frente a cualquier diferencia de `g` que signifique algo: dos activos con
/// bases de coste distintas difieren en órdenes de magnitud mayores. **Es una política declarada,
/// no una tolerancia escondida en un `PartialEq`**: `PartialEq` para [`F64Money`] sigue siendo la
/// igualdad exacta de `f64`, y quien decide el cortocircuito es [`MoneyOps::gains_equal`].
pub const GAIN_RATIO_EQ_TOLERANCE: f64 = 1e-12;

/// El dinero del camino estocástico: un `f64` con nombre.
///
/// El newtype no es decoración — es lo que permite implementar [`MoneyOps`] (regla del huérfano:
/// ni el trait ni `f64` son de este crate) y, sobre todo, **hace imposible mezclar por accidente**
/// un número de este camino con un `Decimal` del camino exacto.
///
/// # Políticas declaradas
///
/// | operación | política | por qué |
/// |---|---|---|
/// | [`MoneyOps::from_decimal`] | `to_f64().unwrap_or(0.0)` | ~15-16 dígitos significativos; un `Decimal` de 28 dígitos PIERDE los 12 últimos. Es la única pérdida de la frontera de entrada y es la razón de la regla «de aquí no sale un euro». |
/// | [`MoneyOps::to_decimal`] | `from_f64_retain`, **saturando** a `Decimal::MAX`/`MIN` fuera de rango y `ZERO` con `NaN` | publicar un cero por un infinito sería inventarse una cifra pequeña donde hay una enorme; saturar conserva el orden de magnitud y el signo. `NaN` no tiene lectura honesta. |
/// | [`MoneyOps::checked_mul`] / [`MoneyOps::checked_div`] / [`MoneyOps::checked_add`] | `None` ⟺ el resultado **no es finito** | en `Decimal` estos devuelven `None` al desbordar el rango; el equivalente en coma flotante es `inf`/`NaN`. Así el bucle levanta `AssetValueOverflow` en vez de propagar un `inf` que contaminaría la serie entera en silencio. La división por cero cae aquí. |
/// | [`MoneyOps::min`] / [`MoneyOps::max`] | la MISMA forma que los inherentes de `rust_decimal` (`if self < other { other } else { self }`) | el camino determinista depende de ese desempate; escribirlo igual mantiene los dos caminos alineados operando a operando. Con `NaN` toda comparación es falsa y ambos devuelven `self`. |
/// | [`MoneyOps::total_cmp`] | `f64::total_cmp` | `f64` **no** es `Ord` y `drain_order` ordena de verdad: hace falta un orden TOTAL, no uno parcial que se rinda con `NaN` o con `-0.0`. |
/// | [`MoneyOps::gains_equal`] | `\|a − b\| ≤ `[`GAIN_RATIO_EQ_TOLERANCE`] | ver la constante. |
/// | [`MoneyOps::powd_fraction`] | `powf(num/den)` | en `Decimal` la familia `(1+p)^{k/12}` va por `powd`, que enruta los exponentes enteros por potencia exacta. `powf` no distingue esos casos: la diferencia es del orden del épsilon, y es parte de lo que mide la puerta de degeneración. |
/// | [`MoneyOps::is_zero`] | `== 0.0` | cierto también para `-0.0`, igual que en `Decimal`. |
/// | `PartialEq` / `PartialOrd` | los de `f64`, EXACTOS | con `NaN` todas las comparaciones son falsas. La única igualdad con tolerancia del núcleo es `gains_equal`, y está aparte a propósito. |
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct F64Money(pub f64);

impl core::ops::Add for F64Money {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        F64Money(self.0 + rhs.0)
    }
}
impl core::ops::Sub for F64Money {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        F64Money(self.0 - rhs.0)
    }
}
impl core::ops::Mul for F64Money {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        F64Money(self.0 * rhs.0)
    }
}
impl core::ops::Div for F64Money {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        F64Money(self.0 / rhs.0)
    }
}
impl core::ops::Neg for F64Money {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        F64Money(-self.0)
    }
}

impl MoneyOps for F64Money {
    #[inline]
    fn zero() -> Self {
        F64Money(0.0)
    }
    #[inline]
    fn one() -> Self {
        F64Money(1.0)
    }
    #[inline]
    fn max_value() -> Self {
        F64Money(f64::MAX)
    }

    #[inline]
    fn from_decimal(d: Decimal) -> Self {
        // ~15-16 dígitos significativos: un `Decimal` de 28 pierde los 12 últimos. `None` solo
        // llega con valores que `f64` no representa; cero es la lectura neutra y esta frontera
        // solo la cruzan importes de entrada ya acotados por la API.
        F64Money(d.to_f64().unwrap_or(0.0))
    }
    #[inline]
    fn to_decimal(self) -> Decimal {
        // SATURA en vez de rendirse a cero: una cifra que se salió del rango de `Decimal` es
        // enorme, no nula, y publicarla como 0 sería inventarse el número contrario.
        Decimal::from_f64_retain(self.0).unwrap_or(if self.0.is_nan() {
            Decimal::ZERO
        } else if self.0.is_sign_negative() {
            Decimal::MIN
        } else {
            Decimal::MAX
        })
    }
    #[inline]
    fn from_u32(v: u32) -> Self {
        F64Money(f64::from(v))
    }
    #[inline]
    fn from_i64(v: i64) -> Self {
        // Por encima de 2^53 la conversión redondea: es la misma pérdida que `from_decimal` y no
        // afecta a las constantes del núcleo (12, 100, 1200).
        F64Money(v as f64)
    }

    #[inline]
    fn checked_add(self, rhs: Self) -> Option<Self> {
        finite(self.0 + rhs.0)
    }
    #[inline]
    fn checked_mul(self, rhs: Self) -> Option<Self> {
        finite(self.0 * rhs.0)
    }
    #[inline]
    fn checked_div(self, rhs: Self) -> Option<Self> {
        finite(self.0 / rhs.0)
    }

    #[inline]
    fn min(self, other: Self) -> Self {
        // Misma forma que el `min` inherente de `rust_decimal` (devuelve `self` en el empate).
        if self.0 > other.0 {
            other
        } else {
            self
        }
    }
    #[inline]
    fn max(self, other: Self) -> Self {
        // Misma forma que el `max` inherente de `rust_decimal` (devuelve `self` en el empate).
        if self.0 < other.0 {
            other
        } else {
            self
        }
    }
    #[inline]
    fn clamp(self, lo: Self, hi: Self) -> Self {
        // Misma forma que `Ord::clamp`: dentro del intervalo, `self` intacto.
        if self.0 < lo.0 {
            lo
        } else if self.0 > hi.0 {
            hi
        } else {
            self
        }
    }

    #[inline]
    fn is_zero(self) -> bool {
        self.0 == 0.0
    }
    #[inline]
    fn is_sign_negative(self) -> bool {
        self.0.is_sign_negative()
    }

    #[inline]
    fn total_cmp(&self, other: &Self) -> core::cmp::Ordering {
        // `f64` no es `Ord`; `drain_order` necesita un orden TOTAL de verdad.
        self.0.total_cmp(&other.0)
    }

    #[inline]
    fn powd_fraction(self, num: u32, den: u32) -> Self {
        F64Money(self.0.powf(f64::from(num) / f64::from(den)))
    }

    #[inline]
    fn gains_equal(a: Self, b: Self) -> bool {
        (a.0 - b.0).abs() <= GAIN_RATIO_EQ_TOLERANCE
    }
}

/// `Some` solo si el resultado es finito: es el equivalente en coma flotante de «no desbordó».
#[inline]
fn finite(x: f64) -> Option<F64Money> {
    x.is_finite().then_some(F64Money(x))
}

/// **La proyección determinista, ejecutada en coma flotante.**
///
/// Convierte la entrada UNA vez y corre el MISMO bucle que `project_net_worth_series`. No es una
/// aproximación de otra cosa: es la misma función con otro tipo numérico, y la puerta de
/// degeneración (`tests/degeneration.rs`) mide lo que esa diferencia vale en euros.
pub fn simulate_f64(input: &ProjectionInput) -> Result<SimOutput<F64Money>, EngineError> {
    let sim = SimInput::<F64Money>::from(input);
    simulate(&sim)
}

/// **El gancho de Monte Carlo** (WP6): la misma proyección, con los factores de crecimiento
/// mensuales por activo dados desde fuera.
///
/// `per_month_per_asset[k − 1][i]` es el factor del activo `i` en el mes `k` (1-based). Hoy el
/// único llamante le pasa los multiplicadores DETERMINISTAS
/// ([`deterministic_growth_multipliers`]) y el resultado es bit a bit el de [`simulate_f64`] —
/// eso es lo que hace este hueco verificable ANTES de que exista el generador aleatorio. En WP6
/// las filas vendrán del sorteo: un shock de mercado común por mes escalado por la volatilidad de
/// cada activo (D11).
///
/// Una fila con un número de elementos distinto del número de activos **se ignora** y ese mes usa
/// el multiplicador determinista: el motor es una función pura y no panica por una entrada mal
/// dimensionada.
pub fn simulate_f64_with_multipliers(
    input: &ProjectionInput,
    per_month_per_asset: &[Vec<f64>],
) -> Result<SimOutput<F64Money>, EngineError> {
    let mut sim = SimInput::<F64Money>::from(input);
    sim.growth_overrides = Some(
        per_month_per_asset
            .iter()
            .map(|row| row.iter().copied().map(F64Money).collect())
            .collect(),
    );
    simulate(&sim)
}

/// Los multiplicadores de crecimiento que el camino determinista usaría, mes a mes y activo a
/// activo — el «caso de volatilidad cero» del que WP6 partirá.
///
/// Se derivan con el MISMO helper del motor (`monthly_growth_multiplier`), no con una fórmula
/// reescrita aquí: una segunda copia de la raíz doceava haría que «volatilidad cero» dejara de
/// significar «el camino determinista» sin que nada fallara.
pub fn deterministic_growth_multipliers(input: &ProjectionInput) -> Vec<Vec<f64>> {
    let sim = SimInput::<F64Money>::from(input);
    let row: Vec<f64> = sim
        .assets
        .iter()
        .map(|a| monthly_growth_multiplier(a.expected_annual_return_percent).0)
        .collect();
    vec![row; sim.horizon_months as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checked_family_reports_non_finite_results() {
        let big = F64Money(f64::MAX);
        assert_eq!(MoneyOps::checked_mul(big, F64Money(10.0)), None);
        assert_eq!(MoneyOps::checked_add(big, big), None);
        assert_eq!(MoneyOps::checked_div(F64Money(1.0), F64Money(0.0)), None);
        assert_eq!(MoneyOps::checked_div(F64Money(0.0), F64Money(0.0)), None);
        assert_eq!(
            MoneyOps::checked_mul(F64Money(2.0), F64Money(3.0)),
            Some(F64Money(6.0))
        );
    }

    #[test]
    fn gains_equal_uses_the_declared_tolerance_and_partial_eq_does_not() {
        let a = F64Money(0.25);
        let b = F64Money(0.25 + 1e-15);
        assert!(a != b, "PartialEq sigue siendo exacto");
        assert!(MoneyOps::gains_equal(a, b), "gains_equal tolera el último bit");
        let far = F64Money(0.25 + 1e-9);
        assert!(
            !MoneyOps::gains_equal(a, far),
            "una diferencia REAL de g no se tolera"
        );
    }

    #[test]
    fn to_decimal_saturates_instead_of_inventing_a_zero() {
        assert_eq!(MoneyOps::to_decimal(F64Money(f64::INFINITY)), Decimal::MAX);
        assert_eq!(
            MoneyOps::to_decimal(F64Money(f64::NEG_INFINITY)),
            Decimal::MIN
        );
        assert_eq!(MoneyOps::to_decimal(F64Money(f64::NAN)), Decimal::ZERO);
        assert_eq!(
            MoneyOps::to_decimal(F64Money(1234.5)),
            "1234.5".parse::<Decimal>().unwrap()
        );
    }

    #[test]
    fn min_and_max_keep_self_on_a_tie_like_the_decimal_path() {
        let a = F64Money(1.0);
        let b = F64Money(1.0);
        // No se puede distinguir por valor; lo que se fija es la FORMA (misma que el inherente de
        // `rust_decimal`), y se comprueba con el orden estricto.
        assert_eq!(MoneyOps::max(F64Money(2.0), a).0, 2.0);
        assert_eq!(MoneyOps::max(a, F64Money(2.0)).0, 2.0);
        assert_eq!(MoneyOps::min(F64Money(2.0), b).0, 1.0);
        assert_eq!(MoneyOps::clamp(F64Money(0.5), a, F64Money(2.0)).0, 1.0);
        assert_eq!(MoneyOps::clamp(F64Money(1.5), a, F64Money(2.0)).0, 1.5);
    }

    #[test]
    fn total_cmp_is_a_total_order_where_partial_ord_gives_up() {
        use core::cmp::Ordering;
        assert_eq!(
            MoneyOps::total_cmp(&F64Money(f64::NAN), &F64Money(1.0)),
            Ordering::Greater
        );
        assert_eq!(
            MoneyOps::total_cmp(&F64Money(-0.0), &F64Money(0.0)),
            Ordering::Less
        );
        assert!(F64Money(f64::NAN).partial_cmp(&F64Money(1.0)).is_none());
    }
}
