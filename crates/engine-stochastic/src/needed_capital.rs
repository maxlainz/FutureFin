//! **El CAPITAL NECESARIO** (WP E7 de 5.0.0; decisión M9 del owner y corrección C4 del panel
//! adversarial de 2026-09-06).
//!
//! `solve_mc` responde «¿cuándo me puedo jubilar?» moviendo la FECHA. Este módulo responde la
//! pregunta simétrica —**«¿cuánto me falta?»**— moviendo el CAPITAL: se fija el mes de jubilación
//! y se bisecciona sobre un factor `λ` que escala el patrimonio LÍQUIDO de partida hasta que el
//! plan cumple el umbral de éxito.
//!
//! ```text
//!   λ*        = mín{λ : éxito(escalar_líquido(λ), k) ≥ umbral}
//!   needed(k) = líquido(k−1) DEL HOGAR ESCALADO POR λ* que se jubila en k
//! ```
//!
//! # Por qué se escala, y por qué solo el líquido
//!
//! El capital necesario NO se descuenta con una fórmula (`gasto/SWR`, «25× tu gasto»): esa es la
//! cifra clásica y sobrevive como lectura informativa (`fire_number_classic_today`,
//! `crates/engine/src/target.rs`), no como el número del plan. Aquí la pregunta se contesta
//! **ejecutando el motor entero** —cascada, topes, deuda, «Próximos», fiscalidad del drenaje,
//! pensión con fecha y puerta de tasa inicial dentro— sobre un hogar idéntico al del usuario
//! salvo por el tamaño de su cartera. Es la misma doctrina de `crates/engine/src/solve.rs` y de
//! `solve_mc`: **bisección sobre el motor entero, extremo VERIFICADO, presupuesto de iteraciones**.
//!
//! Se escalan **solo los activos con `is_liquid == true`** (#143). La vivienda no se toca: no es
//! el stock que el drenaje puede vender, no entra en `liquid_worth` y multiplicarla movería el
//! patrimonio publicado sin mover ni un euro de la capacidad de jubilarse.
//!
//! Y de cada activo líquido se escalan **el valor Y la base de coste** (`purchase_price`). Escalar
//! el valor dejando la base quieta fabricaría una **plusvalía fantasma**: la `g_i = 1 − b_i/v_i`
//! que gobierna el gross-up del drenaje (§2.4 de `.claude/financial-contracts.md`) subiría con `λ`
//! y el hogar pagaría un impuesto que no debe — el capital necesario saldría más alto por un
//! artefacto del método. Moviendo las dos, `g_i` es **invariante exacta** bajo el escalado
//! (`(b·λ)/(v·λ) = b/v`) y el neto de una liquidación escala exactamente por `λ`. Regresión:
//! `scaling_moves_the_basis_with_the_value_so_no_phantom_gain_appears`.
//!
//! # El caveat que este método NO resuelve: la cascada no es homogénea
//!
//! Escalar la cartera supone que **el reparto se mantiene proporcional**, y eso es cierto solo
//! mientras ninguna regla toque su tope. Con un `AllocationCap::Amount` (un «hasta 20.000 € en el
//! fondo de bonos»), un `λ` mayor llena el tope **antes** y desvía el resto de las aportaciones al
//! siguiente destino de la cascada, que tiene otra rentabilidad y otra fiscalidad. Lo mismo con
//! `MonthsExpense`/`IncomeMultiple`, cuyo techo no escala con `λ` en absoluto porque se define
//! sobre el gasto o el ingreso, que no se tocan.
//!
//! **No se corrige aquí y se dice en voz alta**: el número publicado es «el líquido que, con TU
//! mezcla de activos y TUS reglas, hace que el plan cumpla el umbral», y con topes por importe esa
//! mezcla deja de ser la de hoy a medida que `λ` crece. La alternativa —reescalar los topes— sería
//! inventarse una regla de asignación que el usuario no configuró.
//!
//! # Qué es EXACTAMENTE el importe publicado (y por qué no es `λ*·L_det(k−1)`)
//!
//! El importe publicado es el líquido de cierre del mes `k−1` **de la trayectoria del hogar
//! ESCALADO** —`project_net_worth_series(retiring_at(scale_liquid_assets(input, λ*), k))`—, no el
//! producto `λ*·L_det(k−1)` sobre la trayectoria sin escalar. Es una diferencia con consecuencias:
//!
//! - **Es exacto para todo `k`.** El hogar escalado vive `L_escalado(k−1)`, y `λ·L_det(k−1)` solo
//!   coincide con eso cuando los flujos que NO escalan (ahorro, gasto, deuda, «Próximos») no han
//!   tenido tiempo de intervenir — es decir, en `k = 1` y en ningún otro sitio. El error del
//!   producto crece con el horizonte.
//! - **No se rompe en los nodos tardíos.** Si la trayectoria SIN escalar se queda a cero antes de
//!   `k` —en P9 pasa hacia el mes 800, porque el ingreso es plano y el gasto se indexa (#139)—, el
//!   producto valdría `λ·0 = 0` y el nodo saldría como una ausencia, aunque el hogar escalado sí
//!   tiene cartera ahí. Con la trayectoria escalada el nodo publica su cifra. Regresión:
//!   `the_curve_uses_the_scaled_liquid_so_late_nodes_are_not_zero` (nodo del mes 840 de P9).
//!
//! Esa trayectoria es la **determinista**, es decir la **MEDIANA** de los caminos del hogar
//! escalado (decisión M8: la rentabilidad declarada es una CAGR y el sorteo centra la mediana en la
//! línea determinista). Es el mismo escenario que el sorteo evaluó —`retiring_at` incluido—, así
//! que la cifra y la medición que la acompaña hablan del mismo hogar.
//!
//! El precio es **una proyección `Decimal` de más por nodo** (~12,6 ms en P9 a 840 meses), que
//! frente a los 10–19 sorteos de 500 caminos de ese mismo nodo no se nota.
//!
//! # Redondeo: **hacia ARRIBA**, a cientos
//!
//! D4 enmendado (decisión de arquitectura, 2026-09-06) deja que este crate publique estimaciones
//! en euros **rotuladas y redondeadas**, porque su error real es el de MUESTREO, no el del tipo
//! numérico. El redondeo es **hacia arriba** y no al más cercano: un capital necesario redondeado
//! a la baja quedaría por debajo del umbral que promete, que es exactamente el error que no se
//! puede cometer aquí. Un capital que ya es múltiplo de 100 no se mueve.
//!
//! # De aquí SÍ sale un euro — y por eso viaja rotulado
//!
//! Es la única excepción a la regla del crate, y está acotada: [`NeededCapital`] publica dos
//! importes (`amount_nominal` en euros del mes `k−1` y `amount_today` deflactado con el MISMO
//! factor del motor, `inflation_factor_at_month_index`), los dos redondeados a cientos y los dos
//! acompañados de la medición que los respalda ([`NeededCapital::success_at_lambda`], con su `N`,
//! sus fallos y su cota de Wilson). La contabilidad del hogar sigue saliendo del camino `Decimal`.
//!
//! # Nunca un 0 €
//!
//! Un hogar sin activos líquidos no tiene «capital necesario 0 €»: tiene un capital necesario que
//! **este método no puede medir**, porque escalar cero es cero y `λ` no mueve nada. Se publica
//! [`ABSENT_NO_LIQUID_ASSETS`] y ningún importe — y desde la corrección de la fórmula esa ausencia
//! significa lo que dice: el hogar **de verdad** no tiene líquido, no que la trayectoria sin
//! escalar se hubiera agotado. Misma disciplina que `month: None` en
//! `solve_mc`: la ausencia se nombra, no se rellena con el valor que más se le parece.
//!
//! # Y nunca el SUELO DEL MÉTODO disfrazado de necesidad
//!
//! La misma disciplina, en el otro extremo. Cuando `λ` deja de morder —la pensión ya cubre el
//! gasto, un ingreso persiste— el éxito vale 1 para cualquier `λ`, los [`MAX_LAMBDA_HALVINGS`]
//! halvings cumplen todos y **no hay frontera que biseccionar**. La respuesta es
//! [`ABSENT_ALREADY_COVERED`]: «no hace falta capital adicional hoy», sin importe.
//!
//! Hasta esa corrección el bracket devolvía en ese caso el último halving como si fuera `λ*`, y la
//! curva publicaba el líquido del hogar escalado por él. Con `λ ≈ 0` ese líquido no es una
//! necesidad: es lo que el hogar acumula de su NÓMINA hasta esa edad. Y el warm start lo empeoraba
//! nodo a nodo —cada uno heredaba el `λ` del anterior y volvía a dividirlo por `2^8`—, así que en
//! la demo sintética los cinco nodos posteriores a la fecha salían con `λ` = 0,0374 · 0,0001 · 0 ·
//! 0 · 0 y publicaban 485.800 → 771.200 → 1.259.800 → 1.944.000 → 2.902.400 €: una curva CRECIENTE
//! después de la fecha, que se lee como «a los 86 necesitas 2,9 M€». El suelo del warm start
//! ([`WARM_LAMBDA_FLOOR`]) corta la composición; nombrar el caso corta la publicación.
//!
//! # Coste
//!
//! Cada evaluación de `λ` es un SORTEO completo, igual que en `solve_mc`, y además reconstruye la
//! maquinaria del sorteo: al cambiar los valores de los activos cambia la ENTRADA, no solo el
//! trigger, así que el atajo de `solve_mc` (un `PathEngine` sostenido que solo reescribe
//! `retirement_trigger`) no aplica. El sobrecoste es la conversión de la entrada más el buffer de
//! factores (`meses × activos`) por sorteo — despreciable frente a los `paths` caminos que vienen
//! detrás.
//!
//! El presupuesto de una cifra es, por tanto, un número conocido de sorteos:
//!
//! ```text
//!   hoy (k = 1):  1 + ≤ 12 (bracket) + ≤ 12 (bisección)  con `search`
//!                 + 1 + ≤ 6 (confirmación)               con `confirm`
//!   curva:        por nodo, 1 + bracket + ≤ 8 (bisección) con `search`, SIN confirmación
//!   + por cifra publicada: UNA proyección `Decimal` del hogar escalado (~12,6 ms en P9)
//! ```
//!
//! Medido en `tests/timing_mc.rs::the_needed_capital_solve_costs_what_the_plan_says`.

use futurefin_engine::{inflation_factor_at_month_index, project_net_worth_series, ProjectionInput};
use rust_decimal::Decimal;

use crate::mc::PathEngine;
use crate::solve_mc::{retiring_at, success_at_month, SuccessAt};
use crate::{McConfig, McError};

// =================================================================================================
// Presupuestos y constantes
// =================================================================================================

/// Duplicaciones máximas del bracket cuando `λ = 1` (o el warm start) **no** cumple. `2^12 = 4.096`
/// veces la cartera actual: si ni con eso se cumple el umbral, el problema no es el tamaño de la
/// cartera y se publica [`ABSENT_THRESHOLD_UNREACHABLE`].
pub const MAX_LAMBDA_DOUBLINGS: u32 = 12;

/// Halvings máximos del bracket cuando el punto de partida YA cumple (el hogar tiene de sobra).
/// `2^-8 = 1/256`, es decir `λ_min = start/256`.
///
/// **Agotarlos sin encontrar un extremo malo NO devuelve un `λ`**, y esta línea decía lo contrario:
/// «se devuelve el último `λ` verificado bueno, con la minimalidad sin establecer». No es un
/// problema de minimalidad, es que `λ*` **no existe** dentro de la rejilla explorada — con una
/// cartera 256 veces menor el plan sigue cumpliendo, así que la necesidad está por debajo de lo
/// que este método sabe medir y publicar `λ_min·L` sería publicar el SUELO DEL MÉTODO como si
/// fuera una necesidad. Ese caso sale por [`Bracket::AlreadyCovered`] y se publica como
/// [`ABSENT_ALREADY_COVERED`].
pub const MAX_LAMBDA_HALVINGS: u32 = 8;

/// Pasos máximos de la bisección sobre `λ` en el solve FRÍO (capital necesario hoy). Sobre un
/// bracket de razón 2 dejan una precisión relativa de `2^-12 ≈ 2,4e-4` — por debajo de los 100 €
/// del redondeo en cualquier cartera de menos de 400.000 €, y muy por debajo del error de MUESTREO
/// del propio umbral (un camino de 500 vale 0,2 pp).
pub const MAX_LAMBDA_BISECTION_DRAWS: u32 = 12;

/// Pasos máximos de la bisección sobre `λ` en un nodo de la curva CON warm start. Menos que el
/// solve frío porque el bracket llega ya estrecho: el nodo anterior dejó su `λ*` y la curva es
/// suave entre nodos separados 60 meses.
pub const WARM_LAMBDA_BISECTION_DRAWS: u32 = 8;

/// **Suelo del warm start: el `λ` del hogar REAL.** Ningún nodo hereda del anterior un punto de
/// partida por debajo de `1`.
///
/// Sin este suelo, el warm start compone el suelo de los halvings: un nodo que se queda sin
/// extremo malo deja `start/256`, el siguiente arranca ahí y vuelve a dividir por `2^8`, y en tres
/// nodos el punto de partida es `1e-7`. Con la cartera de partida efectivamente a cero, el sorteo
/// no puede distinguir un hogar de otro y **todos** los nodos posteriores «cumplen» — que es
/// exactamente cómo la curva de la demo publicaba la asíntota `λ → 0` (el líquido que el hogar
/// acumula de su nómina) rotulada como capital NECESARIO. El `λ` del hogar real es el primer
/// sondeo natural y no cuesta nada: si ya cumple, el bracket baja desde ahí.
pub const WARM_LAMBDA_FLOOR: f64 = 1.0;

/// Avances de la fase de confirmación cuando el presupuesto grande desmiente a la búsqueda.
pub const MAX_CAPITAL_CONFIRMATION_ADVANCES: u32 = 6;

/// Cuánto avanza `λ` en cada paso de la confirmación: **+2 %**. La confirmación no es otra muestra
/// sino la MISMA ampliada (números aleatorios comunes, ver `solve_mc`), así que cuando desmiente a
/// la búsqueda lo hace por poco: subir de dos en dos por ciento cubre un 12,6 % acumulado en seis
/// pasos sin saltarse el mínimo.
pub const CAPITAL_CONFIRMATION_STEP: f64 = 0.02;

/// La unidad de redondeo de todo importe publicado por este módulo: **cientos de euros, hacia
/// arriba** (D4 enmendado).
pub const CAPITAL_ROUNDING_EUR: i64 = 100;

/// Decimales a los que se recorta `λ` antes de multiplicar. Diez sobran para el `2^-12` de
/// resolución que la bisección alcanza y mantienen la mantisa del producto lejos del techo de
/// `Decimal`.
const LAMBDA_SCALE: u32 = 10;

/// **No hay nada que escalar**: el hogar no tiene activos líquidos (o el hogar ESCALADO llega al
/// cierre de `k−1` con el líquido a cero). Escalar cero es cero, así que `λ` no mueve nada y el
/// método no puede medir. **Nunca se publica 0 €** en su lugar.
pub const ABSENT_NO_LIQUID_ASSETS: &str = "no_liquid_assets";

/// **Ni multiplicando la cartera por `2^`[`MAX_LAMBDA_DOUBLINGS`] se cumple el umbral.** El plan no
/// falla por falta de capital: falla por otra cosa (una regla que no llega a la necesidad, un
/// horizonte imposible, un umbral inalcanzable con los caminos sorteados — ver
/// `SuccessAt::meets`).
pub const ABSENT_THRESHOLD_UNREACHABLE: &str = "threshold_unreachable";

/// El mes pedido cae **fuera del horizonte** de la entrada. Guarda defensiva: el llamante decide
/// la rejilla y el motor es una función pura que no debe indexar fuera de su serie.
pub const ABSENT_MONTH_BEYOND_HORIZON: &str = "month_beyond_horizon";

/// **Ya cubierto**: tu acumulación sola basta al umbral. La necesidad está **por debajo de
/// `λ_min × tu líquido de hoy`** (`λ_min = 1/2^`[`MAX_LAMBDA_HALVINGS`]) y por tanto por debajo de
/// lo que este método puede medir — no hace falta capital adicional HOY para jubilarse en ese mes.
///
/// **No es un importe, y por eso no se publica ninguno.** El caso aparece cuando la necesidad
/// ordinaria del mes de jubilación es cero o casi —una pensión que ya cubre el gasto, un ingreso
/// que persiste— y entonces ni la puerta de tasa inicial ni el drenaje muerden por pequeña que sea
/// la cartera: el éxito es 1 para CUALQUIER `λ`, y no hay frontera que biseccionar.
///
/// **Qué se publicaba antes de nombrarlo, y por qué era falso.** El bracket devolvía como `λ*` el
/// último halving verificado —el SUELO del método— y la curva publicaba
/// `liquid_worth[k−1]` del hogar escalado por él: con `λ ≈ 0` eso no es una necesidad, es lo que el
/// hogar acumula de su nómina hasta esa edad. En la demo sintética, los cinco nodos posteriores a
/// la fecha publicaban 485.800 → 771.200 → 1.259.800 → 1.944.000 → 2.902.400 €, una curva
/// CRECIENTE «porque sigues ahorrando» que se leía como «a los 86 necesitas 2,9 M€».
pub const ABSENT_ALREADY_COVERED: &str = "already_covered";

// =================================================================================================
// El escenario: «la misma cartera, `λ` veces más grande»
// =================================================================================================

/// **Escala el patrimonio LÍQUIDO por `λ`**, valor y base de coste, y deja todo lo demás intacto.
///
/// Qué se toca y qué no, escrito una sola vez:
///
/// | campo | escalado |
/// |---|---|
/// | `SimAsset::value` de un activo con `is_liquid == true` | **sí** |
/// | `SimAsset::purchase_price` de ese mismo activo (si está declarada) | **sí** — o aparece una plusvalía fantasma |
/// | cualquier campo de un activo `is_liquid == false` (la vivienda, #143) | no |
/// | ingresos, gastos, deuda, «Próximos», reglas de la cascada y sus topes | no |
///
/// Un activo líquido **sin** `purchase_price` sigue sin ella: su fiscalidad la gobierna el escalar
/// `taxable_gain_ratio`, que no depende del tamaño.
///
/// `λ` se convierte a [`Decimal`] **una vez** y toda la aritmética es exacta desde ahí: la entrada
/// es una estructura `Decimal` y multiplicar en `f64` reintroduciría por la puerta de atrás el
/// tipo que este crate tiene acotado. `λ = 1` devuelve una copia sin tocar. Un `λ` no finito o ≤ 0
/// se trata como `1` (la bisección nunca genera uno: sus extremos salen de duplicar y promediar
/// valores finitos positivos).
pub fn scale_liquid_assets(input: &ProjectionInput, lambda: f64) -> ProjectionInput {
    let mut scaled = input.clone();
    let factor = lambda_to_decimal(lambda);
    if factor == Decimal::ONE {
        return scaled;
    }
    for asset in scaled.assets.iter_mut() {
        if !asset.is_liquid {
            continue;
        }
        asset.value = asset.value.saturating_mul(factor);
        if let Some(basis) = asset.purchase_price {
            asset.purchase_price = Some(basis.saturating_mul(factor));
        }
    }
    scaled
}

/// La conversión `f64 → Decimal` de `λ`, en su ÚNICO sitio.
fn lambda_to_decimal(lambda: f64) -> Decimal {
    if !lambda.is_finite() || lambda <= 0.0 {
        return Decimal::ONE;
    }
    Decimal::from_f64_retain(lambda)
        .map(|d| d.round_dp(LAMBDA_SCALE))
        .unwrap_or(Decimal::ONE)
}

/// Redondeo **hacia arriba** a [`CAPITAL_ROUNDING_EUR`]. Un múltiplo exacto no se mueve.
fn round_up_to_hundreds(amount: Decimal) -> Decimal {
    let unit = Decimal::from(CAPITAL_ROUNDING_EUR);
    amount
        .checked_div(unit)
        .map(|q| q.ceil())
        .and_then(|q| q.checked_mul(unit))
        .unwrap_or(amount)
}

// =================================================================================================
// El resultado
// =================================================================================================

/// **El capital necesario en un mes**, o la razón de que no lo haya.
///
/// Los dos importes vienen **redondeados a cientos hacia arriba** y son `None` a la vez que
/// [`Self::lambda`]: un capital sin `λ` sería un número sin procedencia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NeededCapital {
    /// El mes del BUCLE (1-based) en el que se forzó la jubilación. `1` = «capital necesario hoy».
    pub month: u32,
    /// El factor VERIFICADO: la cartera líquida de hoy multiplicada por él cumple el umbral
    /// jubilándose en [`Self::month`]. `< 1` significa «ya tienes más del que necesitas».
    ///
    /// **`None` ⟺ hay [`Self::absent_reason`]**, y nunca un `0`, que se leería como «no necesitas
    /// nada». Con [`ABSENT_ALREADY_COVERED`] es `None` **aunque haya habido sondeos que cumplen**:
    /// lo que esos sondeos verificaron es que `λ` no muerde, no que valga uno concreto.
    pub lambda: Option<f64>,
    /// **El líquido de cierre del mes `month−1` de la trayectoria del hogar ESCALADO por `λ*`**,
    /// en euros de ese mes (nominales) y redondeado a cientos hacia arriba. No es
    /// `λ*·L_det(month−1)` sobre la trayectoria sin escalar: ese producto solo coincide en
    /// `month = 1` (ver el doc del módulo).
    pub amount_nominal: Option<Decimal>,
    /// El mismo importe en euros de HOY: `amount_nominal / (1 + π/100)^((month−1)/12)`, con el
    /// factor del motor (`inflation_factor_at_month_index`) y redondeado a cientos hacia arriba
    /// **después** de deflactar. Para `month = 1` coincide con [`Self::amount_nominal`] (el factor
    /// en el índice 0 es 1 exacto).
    ///
    /// `None` con `amount_nominal` presente solo si el factor de inflación es cero o negativo —
    /// una entrada que la API no puede producir (rango validado `[−2, 50]`) y que el motor, como
    /// función pura, admite en su firma.
    pub amount_today: Option<Decimal>,
    /// Por qué no hay cifra: [`ABSENT_NO_LIQUID_ASSETS`], [`ABSENT_THRESHOLD_UNREACHABLE`],
    /// [`ABSENT_MONTH_BEYOND_HORIZON`] o [`ABSENT_ALREADY_COVERED`]. `None` ⟺ hay cifra.
    ///
    /// **Las cuatro son ausencias, pero no dicen lo mismo**: las tres primeras son «este método no
    /// puede medirlo»; la cuarta es una RESPUESTA —«no hace falta capital adicional hoy»— y quien
    /// la enseñe debería decirlo así, no con un guion mudo.
    pub absent_reason: Option<&'static str>,
    /// La medición que respalda el importe: el sorteo del `λ` publicado, con su `N`, sus fallos por
    /// motivo y su cota de Wilson. En `needed_liquid_at_month` es el de CONFIRMACIÓN; en
    /// `needed_capital_curve`, el de búsqueda.
    ///
    /// **Es la única ausencia que la trae**: con [`ABSENT_ALREADY_COVERED`] viaja la medición del
    /// `λ` MÍNIMO que se llegó a sondear y aun así cumplió, porque «ya cubierto» es una afirmación
    /// medida y sin su `N` no sería un resultado. Las otras tres la dejan en `None`.
    pub success_at_lambda: Option<SuccessAt>,
    /// `true` ⟺ **la confirmación no cerró**: ni el `λ` que la búsqueda verificó ni los
    /// [`MAX_CAPITAL_CONFIRMATION_ADVANCES`] avances del +2 % cumplieron el umbral con el
    /// presupuesto grande, y lo que se publica es el que más cerca quedó. El importe se publica
    /// igual —con este flag y con [`Self::success_at_lambda`] al lado—, porque «no lo sé» no es lo
    /// mismo que «no existe».
    ///
    /// Siempre `false` en la curva, que no confirma nada (y lo dice su doc).
    pub capital_is_approximate: bool,
    /// Sorteos de BÚSQUEDA ejecutados (bracket + bisección).
    pub draws_search: u32,
    /// Sorteos de CONFIRMACIÓN ejecutados. `0` en la curva.
    pub draws_confirm: u32,
}

impl NeededCapital {
    /// **«Ya cubierto»**: la necesidad está por debajo de `λ_min × líquido de hoy` y no hay `λ*`
    /// que biseccionar. Se publica SIN importe y SIN `λ` —un `λ_min·L` sería el suelo del método
    /// disfrazado de necesidad— pero CON la medición del sondeo más pequeño que cumplió, que es lo
    /// que sostiene la afirmación.
    ///
    /// `draws_confirm` es 0 a propósito: no hay `λ` que confirmar. Lo que la búsqueda estableció es
    /// que la frontera cae por debajo de la rejilla, y eso no se afina con más caminos sobre un
    /// punto que ya cumple.
    fn already_covered(month: u32, stats: SuccessAt, draws_search: u32) -> Self {
        NeededCapital {
            month,
            lambda: None,
            amount_nominal: None,
            amount_today: None,
            absent_reason: Some(ABSENT_ALREADY_COVERED),
            success_at_lambda: Some(stats),
            capital_is_approximate: false,
            draws_search,
            draws_confirm: 0,
        }
    }

    fn absent(month: u32, reason: &'static str, draws_search: u32, draws_confirm: u32) -> Self {
        NeededCapital {
            month,
            lambda: None,
            amount_nominal: None,
            amount_today: None,
            absent_reason: Some(reason),
            success_at_lambda: None,
            capital_is_approximate: false,
            draws_search,
            draws_confirm,
        }
    }
}

// =================================================================================================
// La bisección sobre λ
// =================================================================================================

/// El sorteo de UN presupuesto, con su contador. A diferencia de `solve_mc::Draws` **no** sostiene
/// un `PathEngine`: cada `λ` es una entrada distinta.
struct LambdaDraws<'a> {
    input: &'a ProjectionInput,
    volatilities: &'a [Option<f64>],
    mc: &'a McConfig,
    threshold_pct: u32,
    month: u32,
    draws: u32,
}

impl LambdaDraws<'_> {
    fn at(&mut self, lambda: f64) -> Result<SuccessAt, McError> {
        let scenario = scale_liquid_assets(self.input, lambda);
        let stats = success_at_month(&scenario, self.volatilities, self.mc, self.month)?;
        self.draws += 1;
        Ok(stats)
    }

    fn meets(&self, stats: &SuccessAt) -> bool {
        stats.meets(self.threshold_pct)
    }
}

/// **El desenlace del bracket sobre `λ`: TRES, no dos.**
///
/// La forma anterior —`Option<(f64, SuccessAt)>`— solo sabía distinguir «hay `λ`» de «no lo hay ni
/// multiplicando la cartera», y metía en la primera un caso que **no es un `λ`**: el de los
/// halvings agotados sin extremo malo. De ahí salía el bug de la curva
/// ([`ABSENT_ALREADY_COVERED`]).
enum Bracket {
    /// **`λ` verificado que cumple, con un extremo MALO conocido por debajo**: la frontera está
    /// acotada por los dos lados y el valor devuelto se ejecutó y cumplió.
    Found(f64, SuccessAt),
    /// **Los [`MAX_LAMBDA_HALVINGS`] halvings se agotaron y TODOS cumplieron.** No hay extremo malo
    /// que acote la frontera por abajo: la necesidad cae por debajo de `λ_min = start/2^8` y `λ*`
    /// no existe dentro de la rejilla explorada.
    ///
    /// El `lambda_probe` es el sondeo más pequeño que se verificó —**no un `λ*`**— y viaja solo
    /// para el warm start y para la medición que acompaña a la ausencia.
    AlreadyCovered {
        lambda_probe: f64,
        stats: SuccessAt,
    },
    /// Ni `2^`[`MAX_LAMBDA_DOUBLINGS`]`·start` cumple: el plan no falla por falta de capital.
    Unreachable,
}

/// **Bracket + bisección sobre `λ`, escrito UNA vez.**
///
/// 1. Se sondea `start`. Si cumple, se **baja** halvando hasta encontrar un `λ` que falle
///    ([`MAX_LAMBDA_HALVINGS`] intentos); si no cumple, se **sube** duplicando hasta encontrar uno
///    que cumpla ([`MAX_LAMBDA_DOUBLINGS`]).
/// 2. Con el bracket «`lo` falla, `hi` cumple», se bisecciona `bisection_budget` veces moviendo
///    `hi` solo a puntos que acaban de comprobarse BUENOS.
///
/// **[`Bracket::Found`] siempre se ejecutó y cumplió.** Agotar el presupuesto de bisección no
/// invalida nada: deja el intervalo más ancho de lo que podría estar y se pierde MINIMALIDAD, no
/// validez.
///
/// # Quedarse sin extremo malo NO es un `λ`
///
/// Si los ocho halvings cumplen, la respuesta honesta no es «`λ* = start/256`»: es que con la
/// cartera dividida por 256 el plan **sigue** cumpliendo, o sea que la necesidad está por debajo de
/// lo que este método sabe medir. Devolverlo como `λ` publicaba el SUELO del método como si fuera
/// una necesidad, y en la curva ese suelo se componía nodo a nodo hasta `λ ≈ 0` —donde el hogar
/// escalado es literalmente un hogar sin cartera— convirtiendo la curva en el ahorro acumulado de
/// la nómina. Sale por [`Bracket::AlreadyCovered`].
///
/// # La monotonía tampoco se supone aquí
///
/// «Más capital ⇒ más éxito» es cierto casi siempre (el numerador de la puerta de tasa inicial no
/// escala y el denominador sí; la cartera aguanta más meses de drenaje), pero no es un teorema:
/// con topes por importe en la cascada un `λ` mayor redirige aportaciones a otro activo, y la
/// medición es muestral. Por eso la garantía es la misma que en `solve_mc`: **un `λ` VERIFICADO
/// que cumple**, no el mínimo demostrable.
fn bracket_and_bisect(
    draws: &mut LambdaDraws<'_>,
    start: f64,
    bisection_budget: u32,
) -> Result<Bracket, McError> {
    let start = if start.is_finite() && start > 0.0 {
        start
    } else {
        1.0
    };
    let first = draws.at(start)?;

    let (mut lo, mut hi, mut hi_stats) = if draws.meets(&first) {
        // ---- hacia ABAJO: el punto de partida ya cumple ---------------------------------------
        let mut hi = start;
        let mut hi_stats = first;
        let mut lo_fails: Option<f64> = None;
        for _ in 0..MAX_LAMBDA_HALVINGS {
            let candidate = hi / 2.0;
            let stats = draws.at(candidate)?;
            if draws.meets(&stats) {
                hi = candidate;
                hi_stats = stats;
            } else {
                lo_fails = Some(candidate);
                break;
            }
        }
        match lo_fails {
            Some(lo) => (lo, hi, hi_stats),
            // Sin extremo malo no hay frontera: NO se devuelve el bueno más pequeño como si fuera
            // `λ*` (era el bug), se nombra el caso.
            None => {
                return Ok(Bracket::AlreadyCovered {
                    lambda_probe: hi,
                    stats: hi_stats,
                })
            }
        }
    } else {
        // ---- hacia ARRIBA: el punto de partida no llega ----------------------------------------
        let mut lo = start;
        let mut found: Option<(f64, SuccessAt)> = None;
        let mut candidate = start;
        for _ in 0..MAX_LAMBDA_DOUBLINGS {
            candidate *= 2.0;
            let stats = draws.at(candidate)?;
            if draws.meets(&stats) {
                found = Some((candidate, stats));
                break;
            }
            lo = candidate;
        }
        let Some((hi, hi_stats)) = found else {
            return Ok(Bracket::Unreachable);
        };
        (lo, hi, hi_stats)
    };

    let mut budget = bisection_budget;
    while budget > 0 {
        let mid = lo + (hi - lo) / 2.0;
        // Bracket agotado en la rejilla de `f64`: seguir sorteando no estrecharía nada.
        if !(mid > lo && mid < hi) {
            break;
        }
        let stats = draws.at(mid)?;
        if draws.meets(&stats) {
            hi = mid;
            hi_stats = stats;
        } else {
            lo = mid;
        }
        budget -= 1;
    }
    Ok(Bracket::Found(hi, hi_stats))
}

// =================================================================================================
// El capital necesario de un mes
// =================================================================================================

/// **El capital necesario para jubilarse en el mes `k`**: el líquido de cierre del mes `k−1` del
/// hogar escalado por `λ*`, con `λ*` biseccionado con `search` y **verificado con `confirm`**.
///
/// # Las fases
///
/// | fase | qué hace | presupuesto |
/// |---|---|---|
/// | **A** | bracket sobre `λ` desde 1 (duplicando o halvando) | ≤ [`MAX_LAMBDA_DOUBLINGS`] / ≤ [`MAX_LAMBDA_HALVINGS`] |
/// | **B** | bisección sobre `λ`, extremo alto siempre VERIFICADO | ≤ [`MAX_LAMBDA_BISECTION_DRAWS`] |
/// | **C** | confirmación del `λ` devuelto con `confirm`; si no cumple, sube +2 % hasta [`MAX_CAPITAL_CONFIRMATION_ADVANCES`] veces y, si aun así no cierra, marca [`NeededCapital::capital_is_approximate`] | 1 + ≤ 6 |
/// | **D** | el IMPORTE: una proyección `Decimal` del hogar escalado por el `λ` publicado, y se lee su `liquid_worth[k−1]` | 1 proyección |
///
/// Las dos configuraciones se validan **antes de sortear nada** y también antes de cualquier
/// ausencia: un `McConfig` inválido es un error aunque no haya nada que medir.
///
/// # Ausencias
///
/// - [`ABSENT_MONTH_BEYOND_HORIZON`] si `k` cae fuera del horizonte (`k` se sube a 1 si viene 0:
///   el mes 0 no existe en el bucle).
/// - [`ABSENT_NO_LIQUID_ASSETS`] si el hogar no tiene activos líquidos (se decide sin sortear) o
///   si el hogar ESCALADO llega a `k−1` con el líquido a cero — **nunca un 0 €**.
/// - [`ABSENT_THRESHOLD_UNREACHABLE`] si ni `2^12` veces la cartera cumple el umbral.
/// - [`ABSENT_ALREADY_COVERED`] si ni `1/2^8` veces la cartera lo incumple: **no hace falta capital
///   adicional hoy**. Se salta la fase C entera —no hay `λ` que confirmar— y se publica la medición
///   del sondeo más pequeño que cumplió.
///
/// # Lo que NO promete
///
/// La **minimalidad** de `λ*` (ver [`bracket_and_bisect`]) ni la homogeneidad de la cascada bajo el
/// escalado (ver el doc del módulo: un tope por importe se llena antes con un `λ` mayor). Lo que sí
/// promete es que el `λ` publicado se **ejecutó** con el presupuesto de confirmación, que su
/// medición viaja al lado y que el importe es el líquido REAL de esa trayectoria en `k−1`, no una
/// extrapolación.
pub fn needed_liquid_at_month(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    search: &McConfig,
    confirm: &McConfig,
    threshold_pct: u32,
    k: u32,
) -> Result<NeededCapital, McError> {
    // El mes 0 no existe en el bucle; el contrato de los índices es 1-based y aquí se respeta.
    let month = k.max(1);
    validate_config(input, volatilities, search)?;
    validate_config(input, volatilities, confirm)?;

    if month > input.horizon_months {
        return Ok(NeededCapital::absent(
            month,
            ABSENT_MONTH_BEYOND_HORIZON,
            0,
            0,
        ));
    }
    // La única ausencia que se puede decidir SIN sortear: sin líquido de partida, `λ` no mueve
    // nada y la bisección no tendría sentido.
    if liquid_today(input).is_zero() {
        return Ok(NeededCapital::absent(month, ABSENT_NO_LIQUID_ASSETS, 0, 0));
    }

    // ---- (A) + (B) búsqueda -------------------------------------------------------------------
    let mut searching = LambdaDraws {
        input,
        volatilities,
        mc: search,
        threshold_pct,
        month,
        draws: 0,
    };
    let lambda = match bracket_and_bisect(&mut searching, 1.0, MAX_LAMBDA_BISECTION_DRAWS)? {
        Bracket::Found(lambda, _) => lambda,
        // **No hace falta capital adicional hoy** para jubilarse en `month`, y eso no es un
        // importe: publicar `λ_min·L` diría «necesitas esto» donde la verdad es «necesitas menos
        // de lo que sé medir». Sin confirmación: no hay `λ` que confirmar.
        Bracket::AlreadyCovered { stats, .. } => {
            return Ok(NeededCapital::already_covered(
                month,
                stats,
                searching.draws,
            ))
        }
        Bracket::Unreachable => {
            return Ok(NeededCapital::absent(
                month,
                ABSENT_THRESHOLD_UNREACHABLE,
                searching.draws,
                0,
            ))
        }
    };
    let draws_search = searching.draws;

    // ---- (C) confirmación ---------------------------------------------------------------------
    let mut confirming = LambdaDraws {
        input,
        volatilities,
        mc: confirm,
        threshold_pct,
        month,
        draws: 0,
    };
    let mut lambda = lambda;
    let mut stats = confirming.at(lambda)?;
    let mut approximate = false;
    if !confirming.meets(&stats) {
        let mut probes = vec![(lambda, stats)];
        let mut closed = false;
        let mut candidate = lambda;
        for _ in 0..MAX_CAPITAL_CONFIRMATION_ADVANCES {
            candidate *= 1.0 + CAPITAL_CONFIRMATION_STEP;
            let probe = confirming.at(candidate)?;
            probes.push((candidate, probe));
            if confirming.meets(&probe) {
                lambda = candidate;
                stats = probe;
                closed = true;
                break;
            }
        }
        if !closed {
            // **El que más cerca quedó**, no el último probado: todas las sondas se midieron con el
            // mismo presupuesto y la más cercana al umbral es la lectura útil.
            let best = probes
                .into_iter()
                .reduce(|a, b| if b.1.wilson_low > a.1.wilson_low { b } else { a })
                .expect("siempre está al menos la sonda del λ de búsqueda");
            lambda = best.0;
            stats = best.1;
            approximate = true;
        }
    }

    // ---- El importe, sobre la trayectoria del hogar ESCALADO por el λ publicado ---------------
    let Some(raw_nominal) = scaled_liquid_before(input, lambda, month)? else {
        return Ok(NeededCapital::absent(
            month,
            ABSENT_NO_LIQUID_ASSETS,
            draws_search,
            confirming.draws,
        ));
    };

    Ok(publish(
        input,
        month,
        lambda,
        stats,
        raw_nominal,
        approximate,
        draws_search,
        confirming.draws,
    ))
}

/// **El capital necesario HOY** = el capital necesario para jubilarse en el mes 1 (decisión M9).
///
/// Envoltorio literal de [`needed_liquid_at_month`] con `k = 1`, no una segunda definición: la
/// cifra que la app enseña en Jubilación, Resumen y Proyección sale de la MISMA bisección que la
/// curva. En `k = 1` el importe coincide además con `λ*·L(0)` —el mes 0 es el estado inicial y
/// ningún flujo ha intervenido todavía—, que es la única `k` donde ese producto y la trayectoria
/// escalada dan lo mismo.
pub fn needed_capital_today(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    search: &McConfig,
    confirm: &McConfig,
    threshold_pct: u32,
) -> Result<NeededCapital, McError> {
    needed_liquid_at_month(input, volatilities, search, confirm, threshold_pct, 1)
}

// =================================================================================================
// La curva por edad
// =================================================================================================

/// **La curva de capital necesario** sobre la rejilla que el llamante pasa, en su MISMO orden y con
/// sus repeticiones (C4: la curva REAL por edad, sin escalados por mediana ni por cuantil — la
/// fórmula `needed·mediana/q` era algebraicamente incapaz de cruzar la línea en la fecha).
///
/// La rejilla la decide quien dibuja (la API pasa cada 60 meses ∪ `{k*}`): este crate no sabe de
/// fechas de nacimiento y no se inventa un muestreo. Rejilla vacía ⇒ vector vacío, **después** de
/// validar la configuración.
///
/// # Warm start
///
/// El primer nodo arranca en frío (`λ = 1`, [`MAX_LAMBDA_BISECTION_DRAWS`] pasos). Cada nodo
/// siguiente **empieza el bracket en el `λ*` del nodo anterior, nunca por debajo de
/// [`WARM_LAMBDA_FLOOR`]**, y bisecciona [`WARM_LAMBDA_BISECTION_DRAWS`] pasos: entre dos nodos
/// separados 60 meses la curva se mueve poco, así que el bracket se cierra en uno o dos sondeos en
/// vez de en cuatro. Un nodo sin `λ` (ausente) no envenena el warm start: se conserva el último
/// `λ*` que sí se verificó.
///
/// **El suelo no es cosmético.** Sin él, un nodo [`Bracket::AlreadyCovered`] deja `start/256` y el
/// siguiente vuelve a dividir por `2^8`: en tres nodos el punto de partida es `1e-7` y el hogar
/// escalado deja de tener cartera, con lo que todos los nodos posteriores «cumplen» por
/// construcción. Es el mecanismo exacto del artefacto que la curva de la demo publicaba (ver el
/// doc del módulo).
///
/// El warm start **no cambia lo que se garantiza**: cada `λ` devuelto se ejecutó y cumplió. Lo que
/// cambia es el coste, y que dos rejillas distintas pueden dar `λ` que difieren en el último
/// dígito de la bisección — la curva es informativa y se calcula en segundo plano.
///
/// # El importe de cada nodo
///
/// Cada nodo paga **una proyección `Decimal` de más** (`scaled_liquid_before`) para leer el líquido
/// REAL del hogar escalado en `k−1`, en vez de multiplicar `λ*` por el líquido de la trayectoria sin
/// escalar. Es lo que hace que los nodos tardíos publiquen cifra: en un hogar cuyo camino actual se
/// agota antes del horizonte (P9 hacia el mes 800), el producto valdría 0 € y el nodo saldría como
/// ausencia aunque el hogar escalado sí tenga cartera ahí.
///
/// # Nodos sin cifra
///
/// Un nodo puede salir sin importe por las CUATRO razones de [`NeededCapital::absent_reason`], y la
/// cuarta ([`ABSENT_ALREADY_COVERED`]) es la que más se ve: a partir de la fecha en que la pensión
/// cubre el gasto, `λ` deja de morder y no hay frontera. Quien dibuje la curva **parte la línea**
/// ahí; unir los dos nodos vecinos por encima del hueco sería una interpolación disfrazada de
/// medición, y rellenarlo con el líquido del hogar escalado a cero fue el bug.
///
/// # Sin confirmación
///
/// **Un solo presupuesto** (`mc`, los 500 caminos de búsqueda) y ninguna fase de confirmación:
/// [`NeededCapital::draws_confirm`] es 0 y [`NeededCapital::capital_is_approximate`] siempre
/// `false` en todos los nodos. La cifra que se verifica con el presupuesto grande es la de HOY
/// ([`needed_capital_today`]); la curva dibuja la forma, no publica un compromiso.
pub fn needed_capital_curve(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
    threshold_pct: u32,
    grid: &[u32],
) -> Result<Vec<NeededCapital>, McError> {
    validate_config(input, volatilities, mc)?;

    let mut curve = Vec::with_capacity(grid.len());
    let mut warm: Option<f64> = None;
    for &k in grid {
        let month = k.max(1);
        if month > input.horizon_months {
            curve.push(NeededCapital::absent(
                month,
                ABSENT_MONTH_BEYOND_HORIZON,
                0,
                0,
            ));
            continue;
        }
        if liquid_today(input).is_zero() {
            curve.push(NeededCapital::absent(month, ABSENT_NO_LIQUID_ASSETS, 0, 0));
            continue;
        }
        let (start, budget) = match warm {
            Some(previous) => (previous, WARM_LAMBDA_BISECTION_DRAWS),
            None => (1.0, MAX_LAMBDA_BISECTION_DRAWS),
        };
        let mut searching = LambdaDraws {
            input,
            volatilities,
            mc,
            threshold_pct,
            month,
            draws: 0,
        };
        match bracket_and_bisect(&mut searching, start, budget)? {
            Bracket::Found(lambda, stats) => {
                // El `λ` verificado alimenta el warm start aunque el importe salga ausente: lo que
                // el nodo siguiente hereda es la POSICIÓN de la frontera, no la cifra. Nunca por
                // debajo de [`WARM_LAMBDA_FLOOR`].
                warm = Some(lambda.max(WARM_LAMBDA_FLOOR));
                match scaled_liquid_before(input, lambda, month)? {
                    Some(raw_nominal) => curve.push(publish(
                        input,
                        month,
                        lambda,
                        stats,
                        raw_nominal,
                        false,
                        searching.draws,
                        0,
                    )),
                    None => curve.push(NeededCapital::absent(
                        month,
                        ABSENT_NO_LIQUID_ASSETS,
                        searching.draws,
                        0,
                    )),
                }
            }
            // **Nodo sin frontera**: la necesidad cae por debajo de `λ_min × líquido de hoy`. Se
            // publica como ausencia —no como el líquido de un hogar escalado a casi cero, que es
            // el ahorro de su nómina y no una necesidad— y el warm start vuelve al suelo en vez de
            // heredar el `λ` diminuto que compondría el mismo artefacto en el nodo siguiente.
            Bracket::AlreadyCovered {
                lambda_probe,
                stats,
            } => {
                warm = Some(lambda_probe.max(WARM_LAMBDA_FLOOR));
                curve.push(NeededCapital::already_covered(
                    month,
                    stats,
                    searching.draws,
                ));
            }
            Bracket::Unreachable => curve.push(NeededCapital::absent(
                month,
                ABSENT_THRESHOLD_UNREACHABLE,
                searching.draws,
                0,
            )),
        }
    }
    Ok(curve)
}

// =================================================================================================
// Piezas compartidas
// =================================================================================================

/// Valida un `McConfig` (y el alineamiento de las volatilidades) **sin sortear nada**, con la misma
/// puerta que usa el sorteo de verdad: construir el `PathEngine` y tirarlo. Evita duplicar aquí las
/// cotas de `paths`, de los percentiles y del vector de volatilidades — la trampa de los defaults
/// duplicados que CLAUDE.md nombra.
fn validate_config(
    input: &ProjectionInput,
    volatilities: &[Option<f64>],
    mc: &McConfig,
) -> Result<(), McError> {
    PathEngine::new(&retiring_at(input, 1), volatilities, mc).map(|_| ())
}

/// Σ de los valores de los activos con `is_liquid` en el estado inicial: lo ÚNICO que `λ` mueve.
fn liquid_today(input: &ProjectionInput) -> Decimal {
    input
        .assets
        .iter()
        .filter(|a| a.is_liquid)
        .map(|a| a.value)
        .sum()
}

/// **El importe, medido donde de verdad está**: el líquido de cierre del mes `month−1` de la
/// trayectoria DETERMINISTA del hogar escalado por `λ` que se jubila en `month` — exactamente el
/// escenario que el sorteo acaba de evaluar (`retiring_at` incluido), en su camino central.
///
/// No es `λ·L_det(month−1)`: ese producto solo coincide en `month = 1`, porque los flujos (ahorro,
/// gasto, deuda, «Próximos») no escalan con `λ`. Cuesta **una** proyección `Decimal` por cifra
/// publicada.
///
/// `Ok(None)` = el hogar escalado llega a `month−1` sin líquido, y el importe sería 0 € — que no se
/// publica nunca.
fn scaled_liquid_before(
    input: &ProjectionInput,
    lambda: f64,
    month: u32,
) -> Result<Option<Decimal>, McError> {
    let scenario = retiring_at(&scale_liquid_assets(input, lambda), month);
    let output = project_net_worth_series(&scenario)?;
    let liquid = output
        .liquid_worth
        .get((month - 1) as usize)
        .copied()
        .unwrap_or(Decimal::ZERO);
    if liquid <= Decimal::ZERO {
        return Ok(None);
    }
    Ok(Some(liquid))
}

/// Convierte `(λ*, líquido escalado en k−1)` en los dos importes publicados. El importe CRUDO se
/// redondea hacia arriba **dos veces por separado** —el nominal y el deflactado—, porque son dos
/// magnitudes distintas y cada una tiene que quedar por encima de lo que promete.
#[allow(clippy::too_many_arguments)]
fn publish(
    input: &ProjectionInput,
    month: u32,
    lambda: f64,
    stats: SuccessAt,
    raw_nominal: Decimal,
    approximate: bool,
    draws_search: u32,
    draws_confirm: u32,
) -> NeededCapital {
    // El MISMO factor del motor (`(1 + π/100)^((k−1)/12)`), no una copia: la trampa de la fórmula
    // duplicada que #139 cerró para el gasto.
    let factor = inflation_factor_at_month_index(input.annual_inflation_percent, month - 1);
    let raw_today = if factor > Decimal::ZERO {
        raw_nominal.checked_div(factor)
    } else {
        None
    };
    NeededCapital {
        month,
        lambda: Some(lambda),
        amount_nominal: Some(round_up_to_hundreds(raw_nominal)),
        amount_today: raw_today.map(round_up_to_hundreds),
        absent_reason: None,
        success_at_lambda: Some(stats),
        capital_is_approximate: approximate,
        draws_search,
        draws_confirm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El redondeo es hacia ARRIBA y un múltiplo exacto no se mueve. Números escritos antes de
    /// correr.
    #[test]
    fn the_rounding_goes_up_and_leaves_exact_hundreds_alone() {
        let up = |n: &str| round_up_to_hundreds(n.parse::<Decimal>().expect("decimal"));
        assert_eq!(up("600000"), Decimal::from(600_000));
        assert_eq!(up("600000.0001"), Decimal::from(600_100));
        assert_eq!(up("600016.30"), Decimal::from(600_100));
        assert_eq!(up("1"), Decimal::from(100));
        assert_eq!(up("0"), Decimal::ZERO);
    }

    /// `λ` se convierte UNA vez y los casos degenerados caen en la identidad, no en un cero que
    /// borraría la cartera.
    #[test]
    fn a_degenerate_lambda_is_the_identity() {
        assert_eq!(lambda_to_decimal(2.0), Decimal::from(2));
        assert_eq!(lambda_to_decimal(0.5), Decimal::new(5, 1));
        assert_eq!(lambda_to_decimal(f64::NAN), Decimal::ONE);
        assert_eq!(lambda_to_decimal(f64::INFINITY), Decimal::ONE);
        assert_eq!(lambda_to_decimal(0.0), Decimal::ONE);
        assert_eq!(lambda_to_decimal(-3.0), Decimal::ONE);
    }
}
