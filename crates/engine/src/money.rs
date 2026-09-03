//! **El tipo numérico del núcleo de simulación** (WP5.5 de 5.0.0, §B.4 del plan de #207).
//!
//! El bucle de proyección es una sola pieza de aritmética encadenada sobre cientos de meses.
//! Monte Carlo (WP6) necesita ejecutar ESE MISMO bucle miles de veces, y hacerlo en `Decimal`
//! cuesta ~12 ms por camino: 500 caminos serían seis segundos por request. La salida no es
//! reescribir el bucle en coma flotante —dos bucles divergen en silencio al primer cambio, y esta
//! casa ya tiene fichada esa familia de fallos— sino **parametrizar el bucle por su tipo
//! numérico**: una sola implementación, dos instanciaciones.
//!
//! [`MoneyOps`] es ese parámetro. Declara las operaciones que el núcleo ejecuta, y la
//! instanciación `Decimal` delega cada una en lo que 4.15.0 ya hacía — mismos operandos, mismo
//! orden, mismos desempates. Por eso el pin dorado no puede moverse: no es que el refactor «no
//! cambie el resultado», es que ejecuta **la misma secuencia de llamadas**.
//!
//! **Tres métodos son FRONTERA y no los llama el bucle** (`grep -rn "to_decimal\|from_i64\|
//! is_sign_negative" crates/engine/src/sim_core.rs` sale vacío, y así debe seguir mientras esto
//! sea cierto): [`MoneyOps::to_decimal`] es la salida hacia el dominio —la usará WP6 para
//! publicar—, [`MoneyOps::from_i64`] cubre las constantes que no caben en `u32`, y
//! [`MoneyOps::is_sign_negative`] existe porque el contrato del tipo debe declarar su signo. Se
//! dicen aquí en vez de dejar que alguien deduzca que todo el trait es «lo que el bucle usa».
//!
//! # Lo que este módulo NO hace
//!
//! No introduce coma flotante en `crates/engine`. El freezer de `crates/engine/src/lib.rs`
//! (`crates_engine_src_has_no_f64_outside_comments`) sigue intacto y sin excepciones: la única
//! implementación de este trait que vive aquí es la de `Decimal`. El mundo de coma flotante vive
//! en un crate APARTE (`crates/engine-stochastic`), que puede implementar el trait sobre su
//! propio newtype gracias a que el trait es público (regla del huérfano).
//!
//! # Por qué cada método está aquí (y por qué no basta con los operadores)
//!
//! - **`min` / `max` / `clamp` no son azúcar de `<`**, y aquí hay una trampa medida. En `Decimal`
//!   la ESCALA sobrevive a las operaciones y decide el `Display`, que es lo que el pin dorado
//!   hashea. `rust_decimal` tiene métodos **inherentes** `min`/`max` (`if self < other { other }
//!   else { self }`), y son los que el motor de 4.15.0 llamaba: con `a == b` devuelven **`self`**.
//!   `Ord::max`, en cambio, devuelve **`other`** en el empate — así que `x.max(ZERO)` con
//!   `x = 0.000000000000000000` daría `"0"` por `Ord` y `"0.000000000000000000"` por el inherente.
//!   Mismo valor, otro hash. Por eso [`MoneyOps::max`] delega en el INHERENTE, no en `Ord`.
//!   `clamp` sí es `Ord::clamp` (`Decimal` no tiene inherente) y devuelve `self` intacto dentro
//!   del intervalo — por eso tampoco se escribe como `self.max(lo).min(hi)`.
//! - **`total_cmp` existe porque `drain_order` ordena** (`sort_by`) y el orden de drenaje decide
//!   qué activo se vende y con qué base gravable. En `Decimal` es `Ord::cmp`; un tipo sin orden
//!   total debe declarar el suyo en vez de que el núcleo se invente un desempate.
//! - **`gains_equal` existe porque la selección uniforme/mixta de `g` se decide con `==`**
//!   (`execute_month_sale`), y esa igualdad es una POLÍTICA, no una operación: en `Decimal` es la
//!   igualdad exacta; un tipo aproximado necesita tolerancia, y esa tolerancia tiene que estar
//!   declarada en el trait, no escondida en un `PartialEq` que nadie mira.
//! - **`powd_fraction` existe porque toda la familia `(1 + p)^{k/12}` del motor** —factor mensual
//!   de crecimiento, factor de inflación, descuento del puente— se calcula con `powd`, que enruta
//!   los exponentes enteros por `checked_powu` (potencia exacta). Pasar el exponente como
//!   `(num, den)` en vez de como un `Self` ya dividido es lo que permite que la instanciación
//!   `Decimal` reproduzca la MISMA llamada de 4.15.0.
//! - **`checked_mul` / `checked_div` / `checked_add` existen porque el motor es una función pura
//!   que no puede panicar**: los desbordes de `Decimal` (#208, #209, `AssetValueOverflow`) tienen
//!   cada uno su degradación declarada, y esas degradaciones son parte del contrato.
//! - **`max_value` existe por un solo sitio**: el techo de la retirada de Guyton-Klinger satura
//!   en vez de panicar («sin límite práctico»).

use core::cmp::Ordering;
use core::fmt::Debug;
use core::ops::{Add, Div, Mul, Neg, Sub};

use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;

/// El contrato numérico del núcleo de simulación.
///
/// **Toda implementación debe declarar sus políticas** —qué hace su `total_cmp` con los valores
/// no ordenables, cuándo devuelven `None` sus `checked_*`, con qué criterio compara
/// `gains_equal`, cuánta precisión pierde `from_decimal`— en el doc-comment de la implementación.
/// El núcleo no toma ninguna de esas decisiones: las ejecuta.
pub trait MoneyOps:
    Copy
    + PartialOrd
    + PartialEq
    + Sized
    + Debug
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    /// El cero del tipo. En `Decimal`, `Decimal::ZERO` (escala 0) — la escala importa: sumar un
    /// cero de escala 18 a un acumulador cambia su `Display` sin cambiar su valor.
    fn zero() -> Self;
    /// El uno del tipo.
    fn one() -> Self;
    /// El mayor valor representable. Único uso: saturar el techo de retirada de los
    /// guardarraíles.
    fn max_value() -> Self;

    /// Conversión desde el tipo canónico del dominio. En `Decimal` es la identidad; en un tipo
    /// aproximado, el punto donde la precisión se pierde — y donde debe declararse cuánta.
    fn from_decimal(d: Decimal) -> Self;
    /// Conversión al tipo canónico del dominio, para publicar. Exacta en `Decimal`. **El núcleo
    /// no la llama**: es la FRONTERA de salida, la que WP6 usará para convertir un percentil en
    /// una cifra publicable.
    fn to_decimal(self) -> Decimal;
    /// Constante entera pequeña (12, 100, 1200…). Debe coincidir EXACTAMENTE con lo que el
    /// código de 4.15.0 escribía como `Decimal::from(n)`.
    fn from_u32(v: u32) -> Self;
    /// Constante entera grande. **Hoy ningún sitio del núcleo la llama** (las constantes del
    /// bucle son 12, 100 y 1200): está en el contrato para que un tipo no tenga que inventarse la
    /// conversión el día que haga falta.
    fn from_i64(v: i64) -> Self;

    /// Suma que devuelve `None` en vez de desbordar.
    fn checked_add(self, rhs: Self) -> Option<Self>;
    /// Producto que devuelve `None` en vez de desbordar. El motor lo usa donde el desborde tiene
    /// una degradación DECLARADA (saturar el payoff de un pasivo, reordenar `b·v/v'`, elevar
    /// `AssetValueOverflow`).
    fn checked_mul(self, rhs: Self) -> Option<Self>;
    /// Cociente que devuelve `None` en vez de panicar (divisor cero, cociente no representable).
    fn checked_div(self, rhs: Self) -> Option<Self>;

    /// Mínimo con el desempate del tipo. En `Decimal`, el método INHERENTE de `rust_decimal`
    /// (`if self > other { other } else { self }`): devuelve `self` cuando son iguales.
    fn min(self, other: Self) -> Self;
    /// Máximo con el desempate del tipo. En `Decimal`, el método INHERENTE de `rust_decimal`
    /// (`if self < other { other } else { self }`): devuelve **`self`** cuando son iguales —
    /// **NO** `Ord::max`, que devuelve `other` y cambiaría la escala (y con ella el `Display` que
    /// el pin dorado hashea).
    fn max(self, other: Self) -> Self;
    /// Acotado a `[lo, hi]`. **No** es `max(lo).min(hi)`: en `Decimal`, `Ord::clamp` devuelve
    /// `self` TAL CUAL cuando ya está dentro del intervalo, y esa identidad conserva la escala.
    fn clamp(self, lo: Self, hi: Self) -> Self;

    /// ¿Es cero? (independiente de la escala en `Decimal`.)
    fn is_zero(self) -> bool;
    /// ¿Lleva signo negativo? **Hoy ningún sitio del núcleo la llama** (el bucle compara con
    /// `zero()`): está en el contrato porque un tipo que no sepa decir su signo no puede
    /// publicar un déficit.
    fn is_sign_negative(self) -> bool;

    /// Orden TOTAL, el que consume `drain_order`. En `Decimal` es `Ord::cmp`.
    fn total_cmp(&self, other: &Self) -> Ordering;

    /// `self^(num/den)` — la familia `(1 + p)^{k/12}` del motor. La implementación `Decimal`
    /// DEBE construir el exponente como `from_u32(num) / from_u32(den)` y llamar a `powd`, que es
    /// lo que 4.15.0 hacía: cualquier otra ruta (producto acumulado, `exp`/`ln`) cambia dígitos.
    fn powd_fraction(self, num: u32, den: u32) -> Self;

    /// ¿Son «la misma» fracción de plusvalía gravable? Decide el cortocircuito uniforme/mixto de
    /// la venta mensual. Política del tipo, no del núcleo.
    fn gains_equal(a: Self, b: Self) -> bool;

    /// Suma de un iterador, con el MISMO plegado que `Iterator::sum` para `Decimal`
    /// (`fold(zero, +)`). Escrita aquí para no depender de una impl de `Sum` que el tipo puede no
    /// tener.
    fn sum_of(values: impl Iterator<Item = Self>) -> Self {
        values.fold(Self::zero(), |acc, v| acc + v)
    }
}

/// La instanciación CANÓNICA: el dinero del dominio.
///
/// Cada método delega en lo que el motor de 4.15.0 ya escribía a mano, sin reinterpretar nada:
///
/// | método | qué ejecuta |
/// |---|---|
/// | `min`/`max` | los INHERENTES de `rust_decimal` (los que el bucle ya llamaba), no los de `Ord` |
/// | `clamp` | `Ord::clamp` (`Decimal` no tiene inherente): dentro del intervalo, `self` intacto |
/// | `total_cmp` | `Ord::cmp` (`Decimal` es totalmente ordenable) |
/// | `gains_equal` | `==` exacto |
/// | `powd_fraction(n, d)` | `self.powd(Decimal::from(n) / Decimal::from(d))` |
/// | `checked_*` | los `checked_*` inherentes de `rust_decimal` (desborde REAL) |
/// | `from_decimal`/`to_decimal` | la identidad — cero pérdida |
impl MoneyOps for Decimal {
    #[inline]
    fn zero() -> Self {
        Decimal::ZERO
    }
    #[inline]
    fn one() -> Self {
        Decimal::ONE
    }
    #[inline]
    fn max_value() -> Self {
        Decimal::MAX
    }
    #[inline]
    fn from_decimal(d: Decimal) -> Self {
        d
    }
    #[inline]
    fn to_decimal(self) -> Decimal {
        self
    }
    #[inline]
    fn from_u32(v: u32) -> Self {
        Decimal::from(v)
    }
    #[inline]
    fn from_i64(v: i64) -> Self {
        Decimal::from(v)
    }
    #[inline]
    fn checked_add(self, rhs: Self) -> Option<Self> {
        Decimal::checked_add(self, rhs)
    }
    #[inline]
    fn checked_mul(self, rhs: Self) -> Option<Self> {
        Decimal::checked_mul(self, rhs)
    }
    #[inline]
    fn checked_div(self, rhs: Self) -> Option<Self> {
        Decimal::checked_div(self, rhs)
    }
    #[inline]
    fn min(self, other: Self) -> Self {
        Decimal::min(self, other)
    }
    #[inline]
    fn max(self, other: Self) -> Self {
        Decimal::max(self, other)
    }
    #[inline]
    fn clamp(self, lo: Self, hi: Self) -> Self {
        Ord::clamp(self, lo, hi)
    }
    #[inline]
    fn is_zero(self) -> bool {
        Decimal::is_zero(&self)
    }
    #[inline]
    fn is_sign_negative(self) -> bool {
        Decimal::is_sign_negative(&self)
    }
    #[inline]
    fn total_cmp(&self, other: &Self) -> Ordering {
        Ord::cmp(self, other)
    }
    #[inline]
    fn powd_fraction(self, num: u32, den: u32) -> Self {
        self.powd(Decimal::from(num) / Decimal::from(den))
    }
    #[inline]
    fn gains_equal(a: Self, b: Self) -> bool {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **La trampa que casi mueve el pin dorado.** `x.max(ZERO)` en el motor llama al `max`
    /// INHERENTE de `rust_decimal`, que devuelve `self` en el empate; `Ord::max` devuelve
    /// `other`. Con `x = 0.000000000000000000` los dos valen cero y tienen `Display` distinto —
    /// y el pin hashea el `Display`. Este test fija cuál de los dos implementa el trait.
    #[test]
    fn max_keeps_self_on_a_tie_like_the_inherent_decimal_max_not_like_ord() {
        let zero_scaled = Decimal::new(0, 18); // 0.000000000000000000
        assert_eq!(zero_scaled.to_string(), "0.000000000000000000");
        assert_eq!(
            MoneyOps::max(zero_scaled, Decimal::ZERO).to_string(),
            "0.000000000000000000",
            "el inherente devuelve self en el empate"
        );
        assert_eq!(
            Ord::max(zero_scaled, Decimal::ZERO).to_string(),
            "0",
            "…y Ord devuelve other: por eso el trait NO puede delegar en Ord"
        );
        assert_eq!(
            MoneyOps::max(zero_scaled, Decimal::ZERO).to_string(),
            Decimal::max(zero_scaled, Decimal::ZERO).to_string(),
            "el trait ES el inherente"
        );
        assert_eq!(
            MoneyOps::min(zero_scaled, Decimal::ZERO).to_string(),
            Decimal::min(zero_scaled, Decimal::ZERO).to_string()
        );
    }

    /// `clamp` NO es `max(lo).min(hi)`: dentro del intervalo devuelve `self` tal cual.
    #[test]
    fn clamp_inside_the_interval_returns_self_untouched() {
        let x = Decimal::new(5, 4); // 0.0005
        let clamped = MoneyOps::clamp(x, Decimal::ZERO, Decimal::ONE);
        assert_eq!(clamped.to_string(), "0.0005");
        let zero_scaled = Decimal::new(0, 18);
        assert_eq!(
            MoneyOps::clamp(zero_scaled, Decimal::ZERO, Decimal::ONE).to_string(),
            "0.000000000000000000",
            "un cero con escala está DENTRO del intervalo y sale intacto"
        );
    }

    /// `powd_fraction` reproduce literalmente las dos llamadas del motor de 4.15.0.
    #[test]
    fn powd_fraction_is_the_call_the_engine_already_made() {
        let annual = Decimal::ONE + Decimal::from(7u32) / Decimal::from(100u32);
        assert_eq!(
            MoneyOps::powd_fraction(annual, 1, 12),
            annual.powd(Decimal::ONE / Decimal::from(12))
        );
        let base = Decimal::ONE + Decimal::from(2u32) / Decimal::from(100u32);
        for m in [1u32, 7, 12, 24, 137, 840] {
            assert_eq!(
                MoneyOps::powd_fraction(base, m, 12),
                base.powd(Decimal::from(m) / Decimal::from(12u32)),
                "mes {m}"
            );
        }
    }

    /// El plegado de `sum_of` es el de `Iterator::sum` para `Decimal` — mismo valor y misma
    /// escala, que es lo que el pin hashea.
    #[test]
    fn sum_of_matches_the_iterator_sum_of_decimal() {
        let vals = [
            Decimal::new(1234, 2),
            Decimal::new(0, 18),
            Decimal::new(-5, 0),
            Decimal::ZERO,
        ];
        let folded = <Decimal as MoneyOps>::sum_of(vals.iter().copied());
        let summed: Decimal = vals.iter().copied().sum();
        assert_eq!(folded, summed);
        assert_eq!(folded.to_string(), summed.to_string());
        assert_eq!(
            <Decimal as MoneyOps>::sum_of(std::iter::empty()).to_string(),
            "0"
        );
    }
}
