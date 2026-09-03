//! **El núcleo de simulación, parametrizado por su tipo numérico** (WP5.5 de 5.0.0, §B.4 de #207).
//!
//! Aquí vive TODA la aritmética del bucle mensual: el servicio de deuda, las fases, la cascada de
//! asignación, la venta del mes con su fiscalidad, el crecimiento de los activos y las lecturas.
//! Es el mismo código que 4.15.0 → WP3 fueron escribiendo en `projection.rs`, **operando a
//! operando**, con `Decimal` sustituido por un parámetro `M: MoneyOps`.
//!
//! # Qué se movió y qué NO
//!
//! `projection.rs` conserva los tipos públicos, el calendario de amortización, el valor actual de
//! una renta y —lo importante— las funciones públicas, que ahora son envoltorios de una línea:
//! convierten [`ProjectionInput`](crate::ProjectionInput) al tipo del núcleo (una copia campo a
//! campo, cero operaciones) y llaman aquí. `runway.rs`, `history.rs`, `net_return.rs` y
//! `solve.rs` siguen en `Decimal` y consumen esos envoltorios.
//!
//! # Por qué esto no puede cambiar un dígito
//!
//! No es un argumento de equivalencia algebraica: es que la instanciación `M = Decimal` **ejecuta
//! la misma secuencia de llamadas**. `x.max(y)` sigue siendo el `max` inherente de `rust_decimal`,
//! `powd_fraction(k, 12)` sigue siendo la misma llamada a `powd`, `a + b` sigue siendo el mismo
//! `Add`, y las sumas de series siguen plegando desde el mismo cero con la misma escala. Los dos
//! pines dorados (`pins-4.15.json` y `pins-5.0-outputs.json`) hashean el `Display` de cada número
//! de cada serie: si alguna de esas equivalencias fuera falsa, fallarían.
//!
//! # El gancho de Monte Carlo
//!
//! El único añadido de comportamiento es [`SimInput::growth_overrides`]: cuando trae la fila del
//! mes, el paso de crecimiento usa esos factores en vez del multiplicador hoisted. Se resuelve
//! con **una selección de slice por mes**, no con un `if` por activo, y con `None` —el único
//! valor que produce la conversión desde `ProjectionInput`— el slice elegido ES el vector hoisted
//! de siempre.

use crate::money::MoneyOps;
use crate::phases::{EngineWarning, Phase, SpendMode};
use crate::projection::{
    add_months, month_first_calendar, month_window, AllocationKind, AllocationSkipReason,
    EarlyRepaymentEffect, EngineError, RepaymentModel,
};
use crate::sim::{
    AllocationCapG, AllocationRuleG, FireTargetView, FirstMonthAllocationG, PhasePlanG,
    RuleOutcomeG, SimAssetG, SimInput, SimLiability, SimOutput, TaxBracketG,
};
use crate::tax::MixedSegment;

// =============================================================================================
// Factores: crecimiento mensual e inflación
// =============================================================================================

/// Factor de crecimiento **mensual** equivalente a una tasa anual nominal (raíz 12ª del factor
/// anual). Tasas ausentes o exactamente 0 se tratan como crecimiento 0 (factor 1). Las tasas
/// **negativas componen de verdad** (−50 % anual ⇒ factor mensual ≈ 0,9439, ×0,5 a los 12 meses);
/// una tasa ≤ −100 % se clampa a factor 0 (pérdida total: el factor anual 1 + p/100 sería ≤ 0 y
/// no tiene raíz 12ª real). La capa API rechaza inputs ≤ −100 con error tipado; el clamp protege
/// frente a valores absurdos ya persistidos.
///
/// `runway.rs` comparte el envoltorio `Decimal` de esta función: el runway debe usar EXACTAMENTE
/// la misma conversión anual→mensual que la simulación, o divergiría del chart de proyección.
pub fn monthly_multiplier_g<M: MoneyOps>(annual_percent: Option<M>) -> M {
    let Some(p) = annual_percent else {
        return M::one();
    };
    if p.is_zero() {
        return M::one();
    }
    let annual_factor = M::one() + p / M::from_u32(100);
    if annual_factor <= M::zero() {
        return M::zero();
    }
    annual_factor.powd_fraction(1, 12)
}

/// Factor de indexación al IPC en el índice de mes `m`: `(1 + annual_percent/100)^(m/12)`.
///
/// `m = 0` o `annual_percent == 0` ⇒ `ONE` **exacto**, sin pasar por la potencia. La guarda es
/// **`is_zero()`, NO `<= ZERO`** (#146): una inflación negativa DEBE componer — con `i = −2 %` el
/// factor a 10 años es `0,98^10 = 0,81707280688754689024` y en los múltiplos de 12 el exponente
/// normaliza a entero y `powd` va por `checked_powu` (potencia exacta, sin `exp`/`ln`).
///
/// Única implementación del factor: la consumen el objetivo FIRE y, desde #139, la indexación del
/// gasto del bucle — la misma trampa de fórmula duplicada que v1.3.0 cerró para el target.
pub(crate) fn inflation_factor_at_index_g<M: MoneyOps>(annual_percent: M, month_index: u32) -> M {
    if month_index == 0 || annual_percent.is_zero() {
        return M::one();
    }
    (M::one() + annual_percent / M::from_u32(100)).powd_fraction(month_index, 12)
}

// =============================================================================================
// Objetivo FIRE de 4.15.0 (sin pensión con fecha)
// =============================================================================================

/// La BASE del objetivo (sin el término de deuda) en el mes `month_index` — evaluada sobre la
/// necesidad REAL del mes (#170): `gross_up(need(k), tramos, g) / SWR`. La puerta de `k = 0` vive
/// AQUÍ y decide para TODA la serie: sin necesidad positiva HOY no hay objetivo en ningún mes —
/// un `max(0,·)` suelto publicaría `target = 0` y un cruce FIRE inmediato y falso.
pub(crate) fn fire_target_base_at_index_g<M: MoneyOps>(
    ft: FireTargetView<'_, M>,
    month_index: u32,
) -> Option<M> {
    if ft.swr_pct <= M::zero() {
        return None;
    }
    if ft.need.annual_net_at(M::one()) <= M::zero() {
        return None;
    }
    let f = inflation_factor_at_index_g(ft.annual_inflation_percent, month_index);
    let net_annual = ft.need.annual_net_at(f);
    let gross = crate::tax::gross_up_net_annual_fire_g(
        net_annual,
        ft.tax_brackets,
        ft.taxes_enabled,
        ft.taxable_gain_ratio,
    );
    Some(gross / (ft.swr_pct / M::from_u32(100)))
}

/// Término finito de deuda (#142) en el `month_index` indicado: cuotas restantes tras ese mes +
/// cola residual, con la cola del vector como valor de saturación fuera de rango.
///
/// **Implementación única**: la consumen el objetivo clásico y el consciente del plan. Dos copias
/// divergirían en el primer cambio de saturación.
pub(crate) fn debt_term_at_index_g<M: MoneyOps>(debt_payments_remaining: &[M], month_index: u32) -> M {
    debt_payments_remaining
        .get(month_index as usize)
        .or(debt_payments_remaining.last())
        .copied()
        .unwrap_or(M::zero())
}

/// El objetivo FIRE de 4.15.0: base + término de deuda. **No es monótono** (base creciente,
/// término decreciente): cualquier optimización que asuma monotonía queda rota en silencio.
pub(crate) fn fire_target_at_index_g<M: MoneyOps>(
    ft: Option<FireTargetView<'_, M>>,
    month_index: u32,
) -> Option<M> {
    let ft = ft?;
    let base = fire_target_base_at_index_g(ft, month_index)?;
    Some(base + debt_term_at_index_g(ft.debt_payments_remaining, month_index))
}

// =============================================================================================
// Pasivos: la recurrencia del mes
// =============================================================================================

/// ¿Plan de pago vivo? — la mitad reutilizable de [`liability_active_g`].
pub(crate) fn plan_alive_g<M: MoneyOps>(
    monthly_payment: M,
    payment_end: Option<chrono::NaiveDate>,
    m_start: chrono::NaiveDate,
) -> bool {
    monthly_payment > M::zero()
        && match payment_end {
            None => true,
            Some(end) => end >= m_start,
        }
}

/// ¿Tiene el pasivo un plan de pago vivo en el mes que empieza en `m_start`?
///
/// Predicado ÚNICO: `monthly_payment > 0` **y** (`payment_end` ausente o `>= m_start`). Sin plan
/// activo el pasivo no cobra caja, no amortiza y tampoco devenga intereses: es una resta
/// constante al patrimonio, que es justo el contrato que explotan los modos B/C del handler.
pub(crate) fn liability_active_g<M: MoneyOps>(
    liab: &SimLiability<M>,
    m_start: chrono::NaiveDate,
) -> bool {
    plan_alive_g(liab.monthly_payment, liab.payment_end, m_start)
}

/// Un mes de vida de un pasivo: devuelve `(caja que sale, principal de cierre)`.
///
/// Única implementación de la recurrencia — la consumen el bucle de simulación, la resolución del
/// mes 1 y el calendario de amortización. Dos implementaciones divergirían en silencio y el chart
/// contaría una historia distinta que la KPI de aportación.
///
/// Convención común a todos los modelos que devengan: **interés sobre el saldo de apertura y
/// cuota a fin de mes**, `P' = P·(1 + i) − M` — la misma recurrencia que `theo(y)` en
/// `history.rs`, para que la interpolación del pasado y la proyección del futuro sean la misma
/// curva.
///
/// - inactivo → `(0, P)`: ni caja, ni amortización, ni devengo.
/// - `FixedPayments` → `cash = min(M, P)`, `P' = P − cash`. **Bit-idéntico** al modelo pre-4.2.0.
///   Sin TIN por contrato desde la Ola 3 (la validación lo rechaza): es el préstamo al 0 %.
/// - `French` → `payoff = P·(1 + i)`, `cash = min(M, payoff)`, `P' = payoff − cash`. El tope de
///   la cuota es el **payoff**, no el principal: cancelar el préstamo cuesta el saldo *con* el
///   interés del mes.
/// - `InterestOnly` (Ola 3, #144) → `cash = min(M, P·i)`, `P' = P + P·i − cash`. La cuota del
///   mes ES el interés del período; la declarada solo topa por arriba, y por debajo el déficit
///   capitaliza (carencia real). Nunca amortiza: eso es `extra_principal_monthly`.
/// - `Revolving` (Ola 3, #144) → misma recurrencia francesa pero la cuota NO es la declarada:
///   `m = max(min_payment_pct·P/100, min_payment_eur)`, `cash = min(m, payoff)`. Con pct 0 y
///   suelo = cuota declarada degenera bit-idéntico en la francesa (forma del backfill).
///
/// **Saturación, nunca pánico**: si el `checked_mul`/`checked_add` del payoff desborda (TIN
/// absurdo × horizonte largo), se devuelve el principal sin devengar más. La salida sigue siendo
/// finita y la simulación termina.
pub(crate) fn liability_month_g<M: MoneyOps>(
    liab: &SimLiability<M>,
    principal: M,
    monthly_payment: M,
    active: bool,
) -> (M, M) {
    if !active {
        return (M::zero(), principal);
    }
    let i = match liab.apr_percent {
        Some(apr) if apr > M::zero() => apr / M::from_u32(1200),
        _ => M::zero(),
    };
    match liab.repayment_model {
        RepaymentModel::FixedPayments => {
            let cash = monthly_payment.min(principal).max(M::zero());
            (cash, principal - cash)
        }
        RepaymentModel::InterestOnly => {
            // Carencia REAL (#144): la cuota ES el interés del período. La declarada es un TOPE
            // por arriba; por debajo, el déficit CAPITALIZA. Nunca amortiza.
            let interest = principal.checked_mul(i).unwrap_or(M::zero());
            let cash = monthly_payment.min(interest).max(M::zero());
            (cash, principal + interest - cash)
        }
        RepaymentModel::Revolving => {
            // Cuota mínima = max(pct × saldo de APERTURA, suelo €); la declarada NO entra en caja.
            let payoff = M::one()
                .checked_add(i)
                .and_then(|factor| principal.checked_mul(factor))
                .unwrap_or(principal);
            let pct_cuota = liab.min_payment_pct.unwrap_or(M::zero()).max(M::zero())
                / M::from_u32(100)
                * principal;
            let m = pct_cuota.max(liab.min_payment_eur.unwrap_or(M::zero()));
            let cash = m.min(payoff).max(M::zero());
            (cash, payoff - cash)
        }
        RepaymentModel::French => {
            let payoff = M::one()
                .checked_add(i)
                .and_then(|factor| principal.checked_mul(factor))
                .unwrap_or(principal);
            let cash = monthly_payment.min(payoff).max(M::zero());
            (cash, payoff - cash)
        }
    }
}

/// Amortización extra del mes `month` (1-based), ya topada al saldo que quedaría tras la cuota.
///
/// Única implementación, como [`liability_month_g`]: la consumen el bucle de simulación, el
/// calendario de amortización y la resolución del mes 1. Devuelve siempre un importe en
/// `0 ..= closing_after_payment`, así que sumarla al servicio de deuda y restarla del principal no
/// puede producir ni caja fantasma ni principal negativo.
///
/// Sin plan de pago activo devuelve `(0, 0)`: amortizar «extra» un pasivo que no cobra cuota no
/// adelanta nada (no hay devengo que evitar ni cuota que liberar) y además rompería el contrato de
/// los modos B/C del handler, donde el principal es una resta CONSTANTE al patrimonio.
///
/// Devuelve `(extra, fee)` (#151): `fee = extra × early_repayment_fee_pct / 100` es la
/// compensación por reembolso anticipado — sale de la caja del mes como coste puro y NO baja el
/// principal. Sin la comisión, el what-if de amortizar era gratis por construcción.
pub(crate) fn liability_extra_principal_g<M: MoneyOps>(
    liab: &SimLiability<M>,
    month: u32,
    closing_after_payment: M,
    active: bool,
) -> (M, M) {
    if !active {
        return (M::zero(), M::zero());
    }
    let mut wanted = liab.extra_principal_monthly.max(M::zero());
    for (m, amount) in &liab.extra_principal_lump_sums {
        if *m == month {
            wanted = wanted + (*amount).max(M::zero());
        }
    }
    let extra = wanted
        .min(closing_after_payment.max(M::zero()))
        .max(M::zero());
    let fee = extra * liab.early_repayment_fee_pct.unwrap_or(M::zero()).max(M::zero())
        / M::from_u32(100);
    (extra, fee)
}

// =============================================================================================
// Drenaje
// =============================================================================================

/// Orden TOTAL de drenaje: líquidos primero; dentro de cada grupo, menor rentabilidad esperada
/// primero (`None` cuenta como 0); empate por índice. **Implementación ÚNICA** (#178): la
/// consumen `drain_from_assets_g`, la rama de déficit del bucle (que necesita el orden ANTES de
/// vender para montar los tramos de `g`) y el bucle finito del runway — una segunda copia haría
/// divergir en silencio la base gravada y la venta ejecutada.
pub(crate) fn drain_order_g<M: MoneyOps>(liquid: &[bool], rates: &[Option<M>]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..liquid.len()).collect();
    order.sort_by(|&i, &j| {
        let li = liquid[i];
        let lj = liquid[j];
        match (li, lj) {
            (true, false) => core::cmp::Ordering::Less,
            (false, true) => core::cmp::Ordering::Greater,
            _ => rates[i]
                .unwrap_or(M::zero())
                .total_cmp(&rates[j].unwrap_or(M::zero()))
                .then_with(|| i.cmp(&j)),
        }
    });
    order
}

/// **El activo que hace de colchón** (P4, §B.6 del plan de #207): el LÍQUIDO de menor
/// rentabilidad esperada, con empate por índice. `None` si no hay ningún activo líquido — sin
/// activo líquido no hay dónde guardar un colchón, y el llamante no debe instalar ninguno.
///
/// Se deriva del ORDEN DE DRENAJE, no de una segunda comparación escrita a mano: es el primer
/// líquido de [`drain_order_g`] y, por tanto, exactamente el activo que la venta del mes vacía
/// primero. Esa coincidencia es lo que hace que el colchón funcione **sin ninguna regla nueva de
/// retirada**: la retirada ya sale de él sola. Una segunda definición del desempate la rompería
/// en silencio al primer cambio.
pub fn cash_buffer_index<M: MoneyOps>(assets: &[SimAssetG<M>]) -> Option<usize> {
    let liquid: Vec<bool> = assets.iter().map(|a| a.is_liquid).collect();
    let rates: Vec<Option<M>> = assets
        .iter()
        .map(|a| a.expected_annual_return_percent)
        .collect();
    drain_order_g(&liquid, &rates)
        .into_iter()
        .find(|&i| liquid[i])
}

/// Drena `need` de los activos en el orden de [`drain_order_g`] y devuelve el DESCUBIERTO.
///
/// Con `taken: Some(slice)` acumula además `taken[i] += lo drenado del activo i` — el reparto que
/// #120 necesita para bajar la base de coste por activo (mismo patrón económico que el
/// `Option<&mut Vec<RuleOutcome>>` de la cascada: el bucle caliente pasa el slice que ya tiene,
/// sin asignar nada por mes).
///
/// Un valor individual NEGATIVO nunca «financia» el drenaje (take clampado a ≥ 0): la escritura
/// valida `current_value ≥ 0` pero la BD no tiene CHECK, y sin el clamp un negativo colado por
/// restore/edición directa SUBÍA el valor y la necesidad a la vez. El negativo sigue pesando en
/// los totales del caller; simplemente no se vende.
fn drain_from_assets_g<M: MoneyOps>(
    values: &mut [M],
    liquid: &[bool],
    rates: &[Option<M>],
    mut need: M,
    mut taken: Option<&mut [M]>,
) -> M {
    if need <= M::zero() {
        return M::zero();
    }
    let order = drain_order_g(liquid, rates);
    for idx in order {
        if need <= M::zero() {
            break;
        }
        let take = values[idx].max(M::zero()).min(need);
        values[idx] = values[idx] - take;
        need = need - take;
        if let Some(t) = taken.as_deref_mut() {
            t[idx] = t[idx] + take;
        }
    }
    need
}

/// Baja la base de coste de un activo en proporción al VALOR drenado — `b' = b·v_post/v_pre`
/// (#120) — sin panicar cuando el producto intermedio no cabe (issue **#209**).
///
/// El orden natural es multiplicar ANTES de dividir: drenar el activo entero deja la base en 0
/// EXACTO, y ese orden es el que 4.15.0 pineó. El reordenamiento `b·(v_post/v_pre)` SOLO se
/// ejecuta cuando el producto no cabe, así que ninguna entrada que hoy funciona cambia un dígito.
fn shrink_basis_g<M: MoneyOps>(basis: M, v_post: M, v_pre: M) -> M {
    match basis.checked_mul(v_post) {
        Some(product) => product / v_pre,
        None => basis * (v_post / v_pre),
    }
}

/// Lo que UNA venta mensual dejó tras de sí, con **las tres magnitudes de B.1.5 ya separadas**
/// (D22/D24, hallazgo B2 de la revisión adversarial):
///
/// - `net_obtained` — euros que de verdad salieron de los activos y se gastaron.
/// - `undrained` — la parte de la venta INTENTADA que los activos no pudieron fundar, en euros
///   de gasto. Es la ÚNICA que resta patrimonio (deuda implícita del hogar).
/// - `shortfall` / `excess` — la distancia entre lo que la regla permitió y el gasto declarado.
///   **Informativas**: no tocan el balance, no cuentan como fracaso.
#[derive(Debug, Clone, Copy)]
struct MonthSale<M> {
    net_obtained: M,
    /// `None` = **no hubo venta** (mes de superávit sin regla que gastar), y entonces no se
    /// acumula NADA — ni siquiera un cero.
    ///
    /// La distinción no es estética: `Decimal` conserva la ESCALA, y sumar un cero de escala 0 a
    /// un acumulador de escala 18 devuelve el operando, no la suma — el mismo VALOR con otro
    /// `Display`, que es justo lo que el pin dorado hashea.
    undrained: Option<M>,
    shortfall: M,
    excess: M,
    /// La venta intentada igualó o superó TODO lo vendible (definición `>=` de #119).
    depleted_portfolio: bool,
}

impl<M: MoneyOps> MonthSale<M> {
    fn empty() -> Self {
        Self {
            net_obtained: M::zero(),
            undrained: None,
            shortfall: M::zero(),
            excess: M::zero(),
            depleted_portfolio: false,
        }
    }

    /// Reparte el resultado de la venta entre las tres magnitudes.
    ///
    /// `attempted_net` = el neto que la venta intentada pretendía obtener; `target_is_need` = esa
    /// venta ERA la necesidad (sin techo, o con un techo que no ataba).
    fn account(
        &mut self,
        need_net: M,
        attempted_net: M,
        obtained_net: M,
        target_is_need: bool,
        forced_by_rule: bool,
        sold: bool,
    ) {
        self.net_obtained = obtained_net;
        // DESCUBIERTO. Lo que los activos no pudieron fundar de la venta intentada, **acotado por
        // la necesidad real**: bajo `rule_is_spend`, la parte discrecional de una venta que la
        // cartera no cubre no es deuda — nadie se endeuda para gastar de más. Con el objetivo =
        // necesidad se conserva la expresión LITERAL de 4.15.0 (sin `min` ni `max`), que es lo
        // que mantiene el pin dorado bit a bit.
        self.undrained = sold.then(|| {
            if target_is_need {
                need_net - obtained_net
            } else {
                (attempted_net.min(need_net) - obtained_net).max(M::zero())
            }
        });
        // RECORTE DE LA REGLA: la necesidad que el techo dejó fuera. NO crece cuando la cartera
        // se agota — eso es el descubierto.
        self.shortfall = if target_is_need {
            M::zero()
        } else {
            (need_net - attempted_net).max(M::zero())
        };
        // SOBRANTE: solo existe cuando la regla ES el gasto y permitió más de lo necesario.
        self.excess = if forced_by_rule {
            (obtained_net - need_net).max(M::zero())
        } else {
            M::zero()
        };
    }
}

/// **La venta del mes** (5.0.0 WP2): decide el bruto a vender, lo vende sobre los activos en el
/// orden de [`drain_order_g`], asienta la base de coste (#120) y devuelve las tres magnitudes.
///
/// Dos razones para vender, y pueden darse a la vez: **la necesidad** (`need_net > 0`, como en
/// 4.15.0) y **la regla como gasto** (`rule_is_spend`, R7). El techo `allowed_gross` es **BRUTO**
/// (R9): topa la VENTA, no los euros que llegan al bolsillo. Con `fixed_real` es `None` y esta
/// función ejecuta, operando a operando, la rama de déficit de 4.15.0.
#[allow(clippy::too_many_arguments)]
fn execute_month_sale_g<M: MoneyOps>(
    values: &mut [M],
    basis: &mut [M],
    basis_declared: &[bool],
    liquid: &[bool],
    rates: &[Option<M>],
    scalar_gain_ratio: M,
    brackets: &[TaxBracketG<M>],
    taxes_enabled: bool,
    need_net: M,
    allowed_gross: Option<M>,
    spend_mode: SpendMode,
    watch_depletion: bool,
) -> MonthSale<M> {
    let mut out = MonthSale::empty();

    // Objetivo BRUTO forzado por la regla: solo en `rule_is_spend` y solo si hay techo.
    let forced_gross = match spend_mode {
        SpendMode::RuleIsSpend => allowed_gross,
        SpendMode::Ceiling => None,
    };
    match forced_gross {
        // Techo no positivo (cartera vacía bajo `percent_of_balance`): no se vende nada, pero el
        // recorte sigue siendo la necesidad entera.
        Some(a) if a <= M::zero() => {
            out.shortfall = need_net.max(M::zero());
            return out;
        }
        Some(_) => {}
        // Sin venta forzada y sin necesidad no hay nada que hacer: es el mes de superávit de
        // 4.15.0, donde la rama de déficit ni se rozaba.
        None if need_net <= M::zero() => return out,
        None => {}
    }

    // #178: la fracción de plusvalía gravable es POR ACTIVO cuando su base es un DATO —
    // `purchase_price` declarado (aunque sea 0) O base alimentada por la propia cascada:
    // `g_i = 1 − b_i/v_i`, clampada a [0,1]. Sin dato, el ESCALAR configurado.
    //
    // `checked_div`, no `/` (issue #208): `*v > ZERO` no basta como guarda. El crecimiento NO
    // toca `basis`, así que una rentabilidad muy negativa deja el valor pegado al mínimo
    // representable con la base entera, y `b/v` se sale del rango — `/` panicaba.
    let gains: Vec<M> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if basis_declared[i] && *v > M::zero() {
                match basis[i].checked_div(*v) {
                    Some(ratio) => (M::one() - ratio).clamp(M::zero(), M::one()),
                    None => M::zero(),
                }
            } else {
                scalar_gain_ratio
            }
        })
        .collect();
    // Cortocircuito de `g` uniforme (sobre lo VENDIBLE): camino LITERAL de 4.11.0, operando a
    // operando — el paseo mixto es algebraicamente igual pero no bit a bit (trocear un tramo
    // lineal añade divisiones que se redondean).
    //
    // **La igualdad la decide el TIPO** (`MoneyOps::gains_equal`): exacta en `Decimal`, con la
    // tolerancia que el tipo declare en aritmética aproximada. Es una política, no una operación.
    let mut uniform_g: Option<M> = None;
    let mut is_uniform = true;
    for i in 0..values.len() {
        if values[i] > M::zero() {
            match uniform_g {
                None => uniform_g = Some(gains[i]),
                Some(u) if M::gains_equal(u, gains[i]) => {}
                Some(_) => {
                    is_uniform = false;
                    break;
                }
            }
        }
    }
    let effective_uniform = if is_uniform {
        Some(uniform_g.unwrap_or(scalar_gain_ratio))
    } else {
        None
    };

    if let Some(g_scalar) = effective_uniform {
        // #140 fase 1: lo que falta se cubre VENDIENDO, y la venta tributa — el bruto a drenar es
        // gross_up_monthly(neto). Con `taxes_enabled = false` es la identidad.
        //
        // 5.0.0 WP2: sobre ese bruto se aplica el techo de la regla. `target_is_need` distingue
        // «la venta ERA la necesidad» (el camino de 4.15.0) de «la venta la fijó la regla».
        let (target_gross, target_is_need) = match forced_gross {
            Some(a) => (a, false),
            None => {
                let need_gross =
                    crate::tax::gross_up_monthly_g(need_net, brackets, taxes_enabled, g_scalar);
                match allowed_gross {
                    Some(a) if a < need_gross => (a, false),
                    _ => (need_gross, true),
                }
            }
        };
        // Mes de agotamiento (#119): el primer mes cuya VENTA BRUTA intentada iguala o supera
        // todo lo vendible. En el caso exacto la cartera se VACÍA este mes y el descubierto
        // empieza el siguiente: por eso `>=`.
        if target_gross > M::zero() && watch_depletion {
            let drainable: M = M::sum_of(values.iter().map(|v| (*v).max(M::zero())));
            if target_gross >= drainable {
                out.depleted_portfolio = true;
            }
        }
        let mut drawn_net = M::zero();
        if target_gross > M::zero() {
            let mut taken = vec![M::zero(); values.len()];
            let und_gross =
                drain_from_assets_g(values, liquid, rates, target_gross, Some(&mut taken));
            // El descubierto se acumula NETO (#140 D-4): mide euros de GASTO que faltaron, no
            // ventas que no ocurrieron.
            let drawn_gross = target_gross - und_gross;
            drawn_net =
                crate::tax::after_tax_monthly_g(drawn_gross, brackets, taxes_enabled, g_scalar);
            // #120: la base baja en proporción al VALOR drenado. Guarda v_pre > 0 obligatoria.
            for i in 0..values.len() {
                if taken[i] > M::zero() {
                    let v_pre = values[i] + taken[i];
                    if v_pre > M::zero() {
                        basis[i] = shrink_basis_g(basis[i], values[i], v_pre);
                    }
                }
            }
        }
        let attempted_net = if target_is_need {
            need_net
        } else {
            crate::tax::after_tax_monthly_g(target_gross, brackets, taxes_enabled, g_scalar)
        };
        out.account(
            need_net,
            attempted_net,
            drawn_net,
            target_is_need,
            forced_gross.is_some(),
            target_gross > M::zero(),
        );
    } else {
        // Vía MIXTA (#178): el solver por tramos decide venta bruta Y reparto a la vez — la base
        // agregada `Σ g_i·venta_i` atraviesa los tramos progresivos y ninguna `g` escalar puede
        // representarla. El orden es EL MISMO de `drain_from_assets_g`.
        let order = drain_order_g(liquid, rates);
        let segments: Vec<MixedSegment<M>> = order
            .iter()
            .map(|&i| MixedSegment {
                capacity_monthly: values[i].max(M::zero()),
                gain_ratio: gains[i],
            })
            .collect();
        // Con venta forzada el objetivo ya es BRUTO y el paseo va en directo. Sin ella se
        // resuelve la necesidad (paseo inverso) y solo se rehace en directo si el techo de verdad
        // recorta esa venta.
        let inverse = match forced_gross {
            Some(_) => None,
            None => Some(crate::tax::gross_up_mixed_monthly(
                need_net,
                &segments,
                brackets,
                taxes_enabled,
            )),
        };
        let binding_cap = match (&inverse, forced_gross, allowed_gross) {
            (None, Some(a), _) => Some(a),
            (Some(dd), _, Some(a)) if dd.gross_monthly > a => Some(a),
            _ => None,
        };
        let (per_segment, attempted_gross, obtained_net, attempted_net, target_is_need) =
            match binding_cap {
                Some(a) => {
                    let w = crate::tax::mixed_drawdown_for_gross_cap(
                        a,
                        &segments,
                        brackets,
                        taxes_enabled,
                    );
                    // ¿Se vendió el techo entero? Si las capacidades no llegaron, la cartera se
                    // vació: lo que no se pudo vender se atribuye a la NECESIDAD (el paseo no
                    // puede poner precio fiscal a un bruto que ningún activo respalda).
                    //
                    // **Lo dice el paseo, no una comparación** (WP5.5): `w.gross_monthly >= a`
                    // era exacto en `Decimal` y un filo de navaja en aritmética aproximada, y de
                    // esa rama cuelga qué es recorte informativo y qué es descubierto que RESTA
                    // patrimonio. Ver `MixedGrossDrawdown::cap_exhausted`.
                    let fully_sold = w.cap_exhausted;
                    let attempted_net = if fully_sold { w.net_monthly } else { need_net };
                    (
                        w.per_segment_monthly,
                        a,
                        w.net_monthly,
                        attempted_net,
                        false,
                    )
                }
                None => {
                    let dd = inverse.expect("sin venta forzada el paseo inverso siempre existe");
                    // El descubierto sale NETO por construcción del solver — sin segunda llamada.
                    let obtained = need_net - dd.net_shortfall_monthly;
                    (
                        dd.per_segment_monthly,
                        dd.gross_monthly,
                        obtained,
                        need_net,
                        true,
                    )
                }
            };
        // Agotamiento (#119), misma semántica `>=` de la vía escalar sobre la venta INTENTADA.
        if watch_depletion {
            let drainable: M = M::sum_of(segments.iter().map(|s| s.capacity_monthly));
            if attempted_gross >= drainable {
                out.depleted_portfolio = true;
            }
        }
        for (pos, &i) in order.iter().enumerate() {
            let take = per_segment[pos];
            if take > M::zero() {
                values[i] = values[i] - take;
                // #120: b' = b·v_post/v_pre, mismas guardas que la vía escalar.
                let v_pre = values[i] + take;
                if v_pre > M::zero() {
                    basis[i] = shrink_basis_g(basis[i], values[i], v_pre);
                }
            }
        }
        out.account(
            need_net,
            attempted_net,
            obtained_net,
            target_is_need,
            forced_gross.is_some(),
            true,
        );
    }

    out
}

// =============================================================================================
// El colchón de caja (P4)
// =============================================================================================

/// **El relleno del colchón de caja** (P4, §B.6 del plan de #207): vende del RESTO de la cartera
/// y abona el neto en el activo colchón. Devuelve el neto movido, siempre `≥ 0`.
///
/// # Qué es exactamente
///
/// Una **venta más**, con la misma maquinaria fiscal que la venta del mes
/// ([`execute_month_sale_g`]): `g` por activo cuando su base es un dato, cortocircuito escalar
/// cuando todas coinciden, paseo mixto por tramos cuando no, y `shrink_basis_g` sobre lo vendido.
/// El euro que llega al colchón entra como **base de coste** (`basis[b] += net`), igual que una
/// aportación de la cascada: ya pagó su plusvalía al salir del otro activo y no puede volver a
/// pagarla al salir de este.
///
/// Lo único que el trasvase destruye es el impuesto de la venta, y eso lo recoge el patrimonio
/// solo: no hace falta contabilidad aparte.
///
/// # De dónde vende, y de dónde NO
///
/// Del orden de drenaje ([`drain_order_g`]) **restringido a `i ≠ buffer_index`**: líquidos
/// primero, menor rentabilidad esperada primero. Dos consecuencias que conviene decir:
///
/// - **Un activo ilíquido es alcanzable, pero el último**: solo cuando todos los líquidos están a
///   cero. Es el mismo orden que ya usa la venta del mes, que también acaba vendiendo la vivienda
///   cuando no queda otra; darle al relleno un orden propio sería una segunda política de venta.
/// - **El motor no sabe qué activo es «volátil»**: `σ` vive en `crates/engine-stochastic`. Aquí el
///   único excluido es el colchón mismo, y quien decide si el colchón siquiera se instala —en
///   Monte Carlo, solo cuando hay volatilidad declarada de la que protegerse— es el llamante.
///
/// # Efecto colateral declarado sobre `contributed_capital`
///
/// `contributed_capital(k) = Σ basis_i(k)` es una identidad del motor, y un trasvase la SUBE en la
/// plusvalía realizada neta de impuesto (`net − coste_vendido`): el euro de plusvalía, una vez
/// tributado, es coste en su nuevo destino y no puede volver a tributar. Es correcto —es lo que
/// hace un traspaso real en una cuenta gravable— pero deja de leerse como «lo que el hogar aportó
/// de su bolsillo». No llega a ningún usuario: el colchón solo actúa en Monte Carlo, que publica
/// bandas y probabilidades, nunca `contributed_capital`.
///
/// # Lo que NO hace
///
/// - **No produce descubierto.** Si la cartera no da para el objetivo, se mueve lo que haya: un
///   relleno es discrecional y no dejar de rellenar no es una deuda del hogar.
/// - **No marca agotamiento.** Vaciar la cartera rellenando no es el mes en que el hogar se quedó
///   sin patrimonio; ese mes lo decide la venta que paga el gasto (#119).
#[allow(clippy::too_many_arguments)]
pub(crate) fn refill_cash_buffer_g<M: MoneyOps>(
    values: &mut [M],
    basis: &mut [M],
    basis_declared: &mut [bool],
    liquid: &[bool],
    rates: &[Option<M>],
    scalar_gain_ratio: M,
    brackets: &[TaxBracketG<M>],
    taxes_enabled: bool,
    buffer_index: usize,
    target_net: M,
) -> M {
    if target_net <= M::zero() || buffer_index >= values.len() {
        return M::zero();
    }
    // El conjunto vendible: el orden de drenaje SIN el colchón. Si el hogar solo tiene el
    // colchón, aquí no queda nada y el relleno es cero — sin ramas especiales.
    let order: Vec<usize> = drain_order_g(liquid, rates)
        .into_iter()
        .filter(|&i| i != buffer_index)
        .collect();

    // #178: misma regla que la venta del mes — `g_i = 1 − b_i/v_i` cuando la base es un DATO,
    // el escalar configurado cuando no. `checked_div` por la misma razón que allí (#208).
    let gains: Vec<M> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if basis_declared[i] && *v > M::zero() {
                match basis[i].checked_div(*v) {
                    Some(ratio) => (M::one() - ratio).clamp(M::zero(), M::one()),
                    None => M::zero(),
                }
            } else {
                scalar_gain_ratio
            }
        })
        .collect();
    // Cortocircuito uniforme sobre lo VENDIBLE del conjunto restringido, con la igualdad que
    // declara el tipo ([`MoneyOps::gains_equal`]). Recorre el orden de drenaje —no los índices—
    // porque ese es el conjunto que de verdad se va a vender.
    let mut uniform_g: Option<M> = None;
    let mut is_uniform = true;
    for &i in &order {
        if values[i] > M::zero() {
            match uniform_g {
                None => uniform_g = Some(gains[i]),
                Some(u) if M::gains_equal(u, gains[i]) => {}
                Some(_) => {
                    is_uniform = false;
                    break;
                }
            }
        }
    }

    let net_moved = if is_uniform {
        // Vía escalar: el bruto a vender es `gross_up_monthly` del objetivo neto, topado por lo
        // que haya. Con `taxes_enabled = false` es la identidad.
        let g_scalar = uniform_g.unwrap_or(scalar_gain_ratio);
        let target_gross =
            crate::tax::gross_up_monthly_g(target_net, brackets, taxes_enabled, g_scalar);
        let mut remaining = target_gross;
        let mut drawn_gross = M::zero();
        for &i in &order {
            if remaining <= M::zero() {
                break;
            }
            let take = values[i].max(M::zero()).min(remaining);
            if take > M::zero() {
                let v_pre = values[i];
                values[i] = values[i] - take;
                // #120: la base baja en proporción al VALOR vendido. `v_pre > 0` por el `take > 0`.
                basis[i] = shrink_basis_g(basis[i], values[i], v_pre);
                drawn_gross = drawn_gross + take;
                remaining = remaining - take;
            }
        }
        crate::tax::after_tax_monthly_g(drawn_gross, brackets, taxes_enabled, g_scalar)
    } else {
        // Vía MIXTA (#178): el paseo inverso decide venta bruta y reparto a la vez, porque la
        // base agregada `Σ g_i·venta_i` atraviesa los tramos progresivos y ninguna `g` escalar
        // la representa. Mismo orden que arriba.
        let segments: Vec<MixedSegment<M>> = order
            .iter()
            .map(|&i| MixedSegment {
                capacity_monthly: values[i].max(M::zero()),
                gain_ratio: gains[i],
            })
            .collect();
        let dd = crate::tax::gross_up_mixed_monthly(target_net, &segments, brackets, taxes_enabled);
        for (pos, &i) in order.iter().enumerate() {
            let take = dd.per_segment_monthly[pos];
            if take > M::zero() {
                let v_pre = values[i];
                values[i] = values[i] - take;
                basis[i] = shrink_basis_g(basis[i], values[i], v_pre);
            }
        }
        // El solver publica el descubierto NETO; lo que se movió es el objetivo menos eso. Aquí
        // ese «descubierto» no es deuda de nadie: es colchón que se quedó sin llenar.
        (target_net - dd.net_shortfall_monthly).max(M::zero())
    };

    if net_moved <= M::zero() {
        return M::zero();
    }
    values[buffer_index] = values[buffer_index] + net_moved;
    // El euro movido ES base de coste en el colchón (como una aportación de la cascada, #120), y
    // desde aquí la base de este activo es un DATO observado (#178).
    basis[buffer_index] = basis[buffer_index] + net_moved;
    basis_declared[buffer_index] = true;
    net_moved
}

// =============================================================================================
// Cascada de asignación
// =============================================================================================

/// Resuelve el tope de una regla en un techo absoluto en euros para el activo destino.
/// `None` para una regla sin tope.
pub(crate) fn resolve_cap_ceiling_g<M: MoneyOps>(
    cap: Option<AllocationCapG<M>>,
    monthly_expense_with_debt: M,
    monthly_income: M,
) -> Option<M> {
    match cap {
        None => None,
        Some(AllocationCapG::Amount(v)) => Some(v.max(M::zero())),
        Some(AllocationCapG::MonthsExpense(n)) => {
            Some((n.max(M::zero()) * monthly_expense_with_debt).max(M::zero()))
        }
        Some(AllocationCapG::IncomeMultiple(n)) => {
            Some((n.max(M::zero()) * monthly_income).max(M::zero()))
        }
    }
}

/// Techo absoluto y hueco restante del cap de UNA regla, contra los valores VIVOS de los activos.
/// `(None, None)` para regla sin tope o con `target_index` fuera de rango.
fn rule_cap_ceiling_and_room_g<M: MoneyOps>(
    rule: &AllocationRuleG<M>,
    live_values: &[M],
    monthly_expense_with_debt: M,
    monthly_income: M,
) -> (Option<M>, Option<M>) {
    let Some(ceiling) = resolve_cap_ceiling_g(rule.cap, monthly_expense_with_debt, monthly_income)
    else {
        return (None, None);
    };
    let room = live_values
        .get(rule.target_index)
        .map(|v| (ceiling - *v).max(M::zero()));
    (Some(ceiling), room)
}

/// Cascada del sobrante (`pool > 0`) sobre los activos siguiendo las `rules` en orden.
///
/// Por regla:
/// - se resuelve el hueco del cap del activo destino (`techo − valor actual`); si es 0, se salta;
/// - se calcula la intención: `Fixed` → `min(amount, remaining)`; `Percent` → `remaining × amount
///   / 100` (sobre lo que queda EN ESTE paso); `Remainder` → `remaining`;
/// - se toma `min(intención, hueco?, remaining)`, se suma a `alloc[target]` y se resta de
///   `remaining`.
///
/// Devuelve `(alloc, leftover)`: `alloc[i] ≥ 0` añadido al activo `i`; `leftover` es el pool que
/// ninguna regla absorbió (el caller lo cuenta en `unallocated_savings_total` — fuera del balance).
///
/// **La cascada no puede sobre-asignar**: `take` está acotado tres veces (intención de la regla,
/// hueco del cap, caja restante) y el bucle corta cuando la caja se agota.
///
/// `trace` es un **sumidero opcional**: con `None` no se asigna nada y el coste es idéntico al de
/// antes de existir — importa porque el bucle de proyección llama a esta función hasta 840 veces
/// por request y nadie lee la traza ahí. Con `Some`, se emite un `RuleOutcome` por regla,
/// incluidas las que no reciben nada. **Una sola implementación de la cascada**: dos divergirían
/// en silencio al primer cambio de caps, y una explicación que no coincide con lo que el motor
/// hace es peor que no tener explicación.
pub(crate) fn distribute_contributions_g<M: MoneyOps>(
    pool: M,
    rules: &[AllocationRuleG<M>],
    values: &[M],
    monthly_expense_with_debt: M,
    monthly_income: M,
    mut trace: Option<&mut Vec<RuleOutcomeG<M>>>,
) -> (Vec<M>, M) {
    let n = values.len();
    let mut alloc = vec![M::zero(); n];
    if pool <= M::zero() || n == 0 {
        if let Some(t) = trace.as_deref_mut() {
            for (rule_index, rule) in rules.iter().enumerate() {
                // Issue #96: el techo se resuelve TAMBIÉN sin sobrante — depende de la regla y
                // de los escalares del mes, no de la caja.
                let (ceiling, room) = rule_cap_ceiling_and_room_g(
                    rule,
                    values,
                    monthly_expense_with_debt,
                    monthly_income,
                );
                t.push(RuleOutcomeG {
                    rule_index,
                    target_index: rule.target_index,
                    amount_intent: M::zero(),
                    amount_resolved: M::zero(),
                    cap_ceiling: ceiling,
                    cap_room: room,
                    skipped_reason: Some(AllocationSkipReason::NoCash),
                });
            }
        }
        return (alloc, pool.max(M::zero()));
    }
    let mut remaining = pool;
    // Vista viva de los valores para los caps a medida que la cascada progresa (así varias reglas
    // hacia el mismo activo respetan un techo compartido).
    let mut live_values: Vec<M> = values.to_vec();

    for (rule_index, rule) in rules.iter().enumerate() {
        // Emite la traza de una regla que no llegó a repartir y sigue.
        macro_rules! skip {
            ($reason:expr, $intent:expr, $ceiling:expr, $room:expr) => {{
                if let Some(t) = trace.as_deref_mut() {
                    t.push(RuleOutcomeG {
                        rule_index,
                        target_index: rule.target_index,
                        amount_intent: $intent,
                        amount_resolved: M::zero(),
                        cap_ceiling: $ceiling,
                        cap_room: $room,
                        skipped_reason: Some($reason),
                    });
                }
            }};
        }

        if remaining <= M::zero() {
            // La caja se agotó: esta regla y todas las siguientes quedan sin evaluar. Se emiten
            // igualmente — omitirlas reproduciría el hueco de observabilidad que la traza cierra.
            if let Some(t) = trace.as_deref_mut() {
                for (i, r) in rules.iter().enumerate().skip(rule_index) {
                    t.push(RuleOutcomeG {
                        rule_index: i,
                        target_index: r.target_index,
                        amount_intent: M::zero(),
                        amount_resolved: M::zero(),
                        cap_ceiling: None,
                        cap_room: None,
                        skipped_reason: Some(AllocationSkipReason::NotReached),
                    });
                }
            }
            break;
        }
        let target = rule.target_index;
        if target >= n {
            skip!(AllocationSkipReason::InvalidTarget, M::zero(), None, None);
            continue;
        }
        let ceiling = resolve_cap_ceiling_g(rule.cap, monthly_expense_with_debt, monthly_income);
        let cap_room = ceiling.map(|c| (c - live_values[target]).max(M::zero()));
        if let Some(room) = cap_room {
            if room <= M::zero() {
                skip!(AllocationSkipReason::CapFull, M::zero(), ceiling, cap_room);
                continue;
            }
        }
        let intent = match rule.kind {
            AllocationKind::Fixed => rule.amount.unwrap_or(M::zero()).max(M::zero()),
            AllocationKind::Percent => {
                let pct = rule.amount.unwrap_or(M::zero()).max(M::zero());
                (remaining * pct) / M::from_u32(100)
            }
            AllocationKind::Remainder => remaining,
        };
        let mut take = intent.min(remaining);
        if let Some(room) = cap_room {
            take = take.min(room);
        }
        if take <= M::zero() {
            skip!(AllocationSkipReason::ZeroAmount, intent, ceiling, cap_room);
            continue;
        }
        alloc[target] = alloc[target] + take;
        live_values[target] = live_values[target] + take;
        remaining = remaining - take;
        if let Some(t) = trace.as_deref_mut() {
            t.push(RuleOutcomeG {
                rule_index,
                target_index: target,
                amount_intent: intent,
                amount_resolved: take,
                cap_ceiling: ceiling,
                cap_room,
                skipped_reason: None,
            });
        }
    }

    (alloc, remaining.max(M::zero()))
}

// =============================================================================================
// El mes 1, resuelto igual que el bucle
// =============================================================================================

/// Resolución completa de la cascada del **primer mes**: lo que se reparte, de dónde sale y qué
/// queda sin repartir. Resuelve el estado del mes 1 EXACTAMENTE como el bucle de simulación
/// (mismo `PhasePlan`, mismo objetivo consciente del plan, misma fase, mismo techo).
pub(crate) fn first_month_allocation_g<M: MoneyOps>(
    input: &SimInput<M>,
) -> Result<FirstMonthAllocationG<M>, EngineError> {
    if input.horizon_months < 1 {
        return Err(EngineError::InvalidHorizon);
    }
    // Mismas puertas que el bucle: esta función RESUELVE EL MES 1 igual que él, así que no puede
    // aceptar un plan que él rechaza.
    input.phase_plan.ensure_supported()?;
    if input.planning_monthly_cash_adjustment.len() != input.horizon_months as usize {
        return Err(EngineError::InvalidPlanningAdjustments);
    }
    let n = input.assets.len();
    let mut out = vec![M::zero(); n];
    for r in &input.allocation_rules {
        if r.target_index >= n {
            return Err(EngineError::InvalidAllocationRuleTarget);
        }
    }
    // Sin activos NO hay atajo a ceros (#127): la caja del mes 1 existe aunque no haya dónde
    // asignarla, y los KPIs la leen de aquí.

    let values: Vec<M> = input.assets.iter().map(|a| a.value).collect();
    let principals: Vec<M> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(M::zero()))
        .collect();

    let start_month_first = month_first_calendar(input.ref_date);
    let month_first = add_months(start_month_first, 0);
    let (m_start, _m_end) = month_window(month_first);

    let mut debt_service = M::zero();
    for (i, liab) in input.liabilities.iter().enumerate() {
        // Mismos helpers que el bucle; el principal de cierre se descarta aquí porque esta
        // función solo resuelve el mes 1 — pero la amortización extra SÍ entra en el servicio de
        // deuda, que es lo que decide cuánto sobrante llega a la cascada.
        let active = liability_active_g(liab, m_start);
        let opening = principals.get(i).copied().unwrap_or(M::zero());
        let (cash, closing) = liability_month_g(liab, opening, liab.monthly_payment, active);
        let (extra, fee) = liability_extra_principal_g(liab, 1, closing, active);
        debt_service = debt_service + cash + extra + fee;
    }

    let planning_adj = input.planning_monthly_cash_adjustment[0];

    // El mes 0 no tiene sobrante acumulado ni caja pendiente. El cruce se decide contra el
    // patrimonio LÍQUIDO (#143), igual que en el bucle: Σ de los activos vendibles.
    let liquid_month_zero: M = M::sum_of(
        input
            .assets
            .iter()
            .zip(values.iter())
            .filter(|(a, _)| a.is_liquid)
            .map(|(_, v)| *v),
    );
    let plan = &input.phase_plan;
    let ft_view = input.fire_target.as_ref().map(|f| f.view());
    let fire_reached = crate::target::plan_target_at(ft_view, plan, 0).is_some_and(|t| liquid_month_zero >= t);
    let in_retirement = (fire_reached && !plan.crossing_is_reading_only)
        || plan.retirement_trigger.forced_month().is_some_and(|s| 1 >= s);
    let phase = if in_retirement {
        Phase::Retired
    } else if plan.partial.is_some_and(|p| 1 >= p.start_month) {
        Phase::Partial
    } else {
        Phase::Accumulating
    };
    let income = match phase {
        Phase::Retired => plan.income_retirement_monthly,
        Phase::Partial => plan
            .partial
            .map_or(input.income_regular_monthly, |p| p.income_monthly),
        Phase::Accumulating => input.income_regular_monthly,
    };
    let income = match plan.income_pause.and_then(|p| p.factor_at(1)) {
        Some(f) => income * f,
        None => income,
    };
    // El mes 1 evalúa el índice 0 y `f(0) = 1` exacto, así que una pensión indexada que ya
    // hubiera empezado se cobra por su importe de hoy — el mismo valor que el bucle calcula.
    let pension_income = match plan.pension {
        Some(pen) => {
            let gross = pen.monthly_at(0, M::one());
            if matches!(phase, Phase::Partial) {
                gross * pen.partial_fraction()
            } else {
                gross
            }
        }
        None => M::zero(),
    };
    let income = if pension_income.is_zero() {
        income
    } else {
        income + pension_income
    };
    let expense = match phase {
        Phase::Retired => plan.expense_retirement_monthly,
        Phase::Partial => plan
            .partial_expense_basis_monthly(input.expense_regular_monthly)
            .unwrap_or(input.expense_regular_monthly),
        Phase::Accumulating => input.expense_regular_monthly,
    };
    let retirement_withdrawal = if in_retirement {
        plan.extra_monthly_withdrawal
    } else {
        M::zero()
    };

    let recurring_net = income - expense - debt_service;
    let planning_component = planning_adj - retirement_withdrawal;
    let net_cash_month = recurring_net + planning_component;

    let mut rules_trace: Vec<RuleOutcomeG<M>> = Vec::new();
    // 4.12.1 (#175): la cascada corre TAMBIÉN jubilado. Los techos se resuelven con la FASE del
    // mes, y desde 4.12.1 esos techos GOBIERNAN euros de verdad, no solo la explicación. Mismo
    // techo de aportación que el bucle (§B.7).
    let (pool, disposable) = match plan.contribution_cap_at(1) {
        Some(cap) if net_cash_month > M::zero() => {
            let invested = net_cash_month.min(cap);
            (invested, net_cash_month - invested)
        }
        _ => (net_cash_month, M::zero()),
    };
    let (alloc, leftover) = distribute_contributions_g(
        pool,
        &input.allocation_rules,
        &values,
        expense + debt_service,
        income,
        Some(&mut rules_trace),
    );
    for i in 0..n {
        out[i] = alloc[i];
    }
    Ok(FirstMonthAllocationG {
        per_asset: out,
        base_cash: net_cash_month,
        recurring_net,
        planning_component,
        debt_service,
        leftover: if net_cash_month > M::zero() {
            leftover
        } else {
            M::zero()
        },
        disposable,
        rules: rules_trace,
    })
}

// =============================================================================================
// EL BUCLE
// =============================================================================================

/// **La simulación mensual completa**, sobre cualquier tipo numérico que cumpla [`MoneyOps`].
///
/// Orden de los pasos del mes, invariante desde 4.2.0: servicio de deuda → transición de fase →
/// caja (ingreso, gasto indexado, pensión con fecha, ajustes de planning) → cascada del sobrante
/// → venta del mes → crecimiento de activos → asiento de principales → series.
pub fn simulate<M: MoneyOps>(input: &SimInput<M>) -> Result<SimOutput<M>, EngineError> {
    if input.horizon_months < 1 {
        return Err(EngineError::InvalidHorizon);
    }
    // Lo que este motor no sabe simular no se simula (ver `PhasePlanG::ensure_supported`).
    input.phase_plan.ensure_supported()?;
    let plan: &PhasePlanG<M> = &input.phase_plan;
    if input.planning_monthly_cash_adjustment.len() != input.horizon_months as usize {
        return Err(EngineError::InvalidPlanningAdjustments);
    }
    let n = input.assets.len();
    for r in &input.allocation_rules {
        if r.target_index >= n {
            return Err(EngineError::InvalidAllocationRuleTarget);
        }
    }

    let mut values: Vec<M> = input.assets.iter().map(|a| a.value).collect();
    let liquid: Vec<bool> = input.assets.iter().map(|a| a.is_liquid).collect();
    let rates: Vec<Option<M>> = input
        .assets
        .iter()
        .map(|a| a.expected_annual_return_percent)
        .collect();
    // Factor de crecimiento mensual POR ACTIVO, calculado UNA vez (WP1a de 5.0.0). Es
    // loop-invariante por construcción: `rates` se deriva de `input.assets` y nadie la muta en
    // toda la función. Hasta 4.15.0 el paso de crecimiento llamaba a `monthly_multiplier` —y con
    // ella a `powd`— una vez por activo Y POR MES. MISMA llamada, MISMO argumento, mismo
    // resultado: el pin dorado lo comprueba bit a bit.
    //
    // Lo que NO se precalcula, y por qué: `inflation_factor_at_index_g(…, k−1)` y el objetivo del
    // mes se evalúan una vez por mes DENTRO del bucle. Un vector por `k` haría EXACTAMENTE las
    // mismas llamadas (las dos se evalúan incondicionalmente cada mes), así que no ahorra una
    // sola potencia — medido — a cambio de dos vectores de 840. Y jamás por producto acumulado:
    // `powd` enruta los exponentes enteros por `checked_powu` (potencia exacta).
    let growth_multipliers: Vec<M> = rates.iter().map(|r| monthly_multiplier_g(*r)).collect();

    let mut principals: Vec<M> = input
        .liabilities
        .iter()
        .map(|l| l.principal.max(M::zero()))
        .collect();
    // Cuota efectiva por pasivo (#151): solo la muta «reducir cuota» (λ-escala). Con el efecto
    // default (`ReduceTerm`) nunca se toca y la simulación es bit-idéntica a 4.6.0.
    let mut effective_payment: Vec<M> = input
        .liabilities
        .iter()
        .map(|l| l.monthly_payment)
        .collect();
    // La jubilación es un estado ABSORBENTE (#141): una vez cruzado el objetivo (o alcanzado el
    // mes forzado), el hogar no «vuelve al trabajo» porque el patrimonio caiga un mes por debajo
    // del target inflado.
    let mut retired = false;
    // Lecturas de fase (§B.8). `retirement_month_index` es el mes EFECTIVO (lo que el latch
    // decide); `liquid_crossing_month_index` es el cruce puro y NO gobierna nada.
    let mut retirement_month_index: Option<u32> = None;
    let mut liquid_crossing_month_index: Option<u32> = None;
    // La fase parcial es el segundo latch, y también monótono: se entra por
    // `k ≥ partial.start_month` y solo se sale hacia `Retired`.
    let mut partial_month_index: Option<u32> = None;
    let mut partial_capital_shrank = false;
    let mut warnings: Vec<EngineWarning> = Vec::new();
    // El objetivo CONSCIENTE DEL PLAN (§B.3). Se construye UNA vez: con puente activo tabula
    // `O(P)` gross-ups y potencias, y consultarlo mes a mes es `O(1)`. Sin pensión con fecha
    // evalúa el objetivo de 4.15.0 tal cual, así que el camino de siempre pasa por la misma
    // función y el pin dorado no puede moverse.
    let ft_view = input.fire_target.as_ref().map(|f| f.view());
    let plan_target = crate::target::PlanTargetG::new(ft_view, plan);
    // Caja que el techo de aportación deja fuera de la cascada (§B.7). Índice 0 = 0, como todas.
    let mut disposable_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    disposable_series.push(M::zero());
    let mut disposable_total = M::zero();
    // Retirada neta efectiva del mes (§B.8). El índice 0 es el estado inicial, no un mes
    // simulado: cero por definición, como el resto de series.
    let mut withdrawal_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    withdrawal_series.push(M::zero());
    // Las otras dos magnitudes de B.1.5, con la misma base de índice: el recorte de la regla y el
    // sobrante que `rule_is_spend` gasta. Ninguna de las dos toca el balance.
    let mut shortfall_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    shortfall_series.push(M::zero());
    let mut excess_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    excess_series.push(M::zero());
    // P4: lo que el colchón absorbió cada mes, con el mismo eje. Todo ceros sin colchón — y sin
    // colchón el bucle ni evalúa el objetivo, así que el coste es un `push` de un cero.
    let mut buffer_refill_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    buffer_refill_series.push(M::zero());
    let mut buffer_refill_months: u32 = 0;
    // Estado de la regla de retirada (§B.2). Vive FUERA del bucle porque `hybrid` y `guardrails`
    // tienen memoria: un latch que no se recuerda no es un latch.
    let mut planner = crate::withdrawal::WithdrawalPlanner::new(plan.withdrawal);

    let start_month_first = month_first_calendar(input.ref_date);

    let mut net_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut contrib_series = Vec::with_capacity(input.horizon_months as usize + 1);
    let mut per_asset_series: Vec<Vec<M>> = input
        .assets
        .iter()
        .map(|_| Vec::with_capacity(input.horizon_months as usize + 1))
        .collect();

    // #120: base de coste POR ACTIVO (coste medio, no FIFO). Arranca en el precio de compra
    // (> 0) y desde aquí: sube con lo que la cascada aporta a ese activo, y BAJA
    // proporcionalmente al valor drenado. La rentabilidad nunca la toca — el hueco valor−base ES
    // la plusvalía latente. `contributed_capital(k) = Σ basis_i(k)` es una IDENTIDAD.
    let mut basis: Vec<M> = input
        .assets
        .iter()
        .map(|a| a.purchase_price.filter(|p| *p > M::zero()).unwrap_or(M::zero()))
        .collect();
    // #178 extensión (4.12.1): un activo cuya base ALIMENTA la simulación (la cascada le aportó)
    // deriva su g aunque no declarara purchase_price — el euro aportado ES el dato.
    let mut basis_declared: Vec<bool> = input
        .assets
        .iter()
        .map(|a| a.purchase_price.is_some())
        .collect();
    let contributed_fn = |basis: &[M]| -> M { M::sum_of(basis.iter().copied()) };
    let mut undrained_cumulative = M::zero();
    let mut assets_depleted_month_index: Option<u32> = None;
    // 4.12.1: el ahorro que ninguna regla absorbe NO entra al balance — no compone, no cuenta
    // como aportado, no es riqueza líquida. Solo se CUANTIFICA aquí.
    let mut unallocated_savings_total = M::zero();

    let nw_fn = |vals: &[M], pr: &[M], und: M| -> M {
        let ta: M = M::sum_of(vals.iter().copied());
        let tl: M = M::sum_of(pr.iter().copied());
        ta - tl - und
    };
    // Base líquida del cruce (#143): lo vendible — activos `is_liquid`. Brutos a propósito (sin
    // restar pasivos ni descubierto): el objetivo empareja ese hueco con su término de cuotas.
    let liquid_fn = |vals: &[M]| -> M {
        M::sum_of(
            input
                .assets
                .iter()
                .zip(vals.iter())
                .filter(|(a, _)| a.is_liquid)
                .map(|(_, v)| *v),
        )
    };
    let mut liquid_series = Vec::with_capacity(input.horizon_months as usize + 1);

    net_series.push(nw_fn(&values, &principals, undrained_cumulative));
    liquid_series.push(liquid_fn(&values));
    contrib_series.push(contributed_fn(&basis));
    for (i, s) in per_asset_series.iter_mut().enumerate() {
        s.push(values[i]);
    }

    for k in 1..=input.horizon_months {
        let month_first = add_months(start_month_first, k - 1);
        let (m_start, _m_end) = month_window(month_first);

        // Servicio de deuda del mes. La recurrencia del pasivo se resuelve **una sola vez** por
        // mes: aquí salen a la vez la caja que se paga y el principal de cierre, que se guarda y
        // se aplica en el paso de amortización más abajo.
        let mut debt_service = M::zero();
        let mut closing_principals: Vec<M> = Vec::with_capacity(principals.len());
        for (i, liab) in input.liabilities.iter().enumerate() {
            if i >= principals.len() {
                break;
            }
            let active = liability_active_g(liab, m_start);
            let (cash, closing) =
                liability_month_g(liab, principals[i], effective_payment[i], active);
            // Amortización extra (what-if): sale de la caja del mes como servicio de deuda Y baja
            // el principal el mismo importe. Las dos cosas o ninguna. La comisión (#151) es la
            // excepción asimétrica A PROPÓSITO: sale de la caja y NO baja nada.
            let (extra, fee) = liability_extra_principal_g(liab, k, closing, active);
            debt_service = debt_service + cash + extra + fee;
            let new_closing = closing - extra;
            // «Reducir cuota» (#151): λ = P'/P sobre el saldo TRAS la cuota del mes.
            if extra > M::zero()
                && liab.early_repayment_effect == EarlyRepaymentEffect::ReducePayment
                && closing > M::zero()
            {
                effective_payment[i] = effective_payment[i] * new_closing / closing;
            }
            closing_principals.push(new_closing);
        }

        let planning_adj = input.planning_monthly_cash_adjustment[(k - 1) as usize];
        // La casilla del mes se reserva ANTES de la cascada para poder escribirla desde dentro
        // del `if` del sobrante sin depender del orden de los `push` del final del mes.
        disposable_series.push(M::zero());

        // El cruce se decide contra el patrimonio LÍQUIDO al cierre del mes k-1 (#143): la regla
        // del SWR está calibrada sobre cartera vendible; una vivienda no produce retirada.
        let liquid_prev = liquid_fn(&values);
        // El objetivo lo evalúa el evaluador CONSCIENTE DEL PLAN. Sin pensión con fecha delega en
        // el objetivo de 4.15.0 evaluado en `k−1` — misma llamada, mismos dígitos.
        let target_prev = plan_target.at(ft_view, k - 1);
        let fire_reached = target_prev.map_or(false, |t| liquid_prev >= t);
        // Lectura pura: el cruce se evalúa TODOS los meses —también después de que el latch
        // cierre, porque la línea de arriba no depende de `retired`— así que anotar su primera
        // vez no toca una sola decisión ni una sola cifra.
        if fire_reached && liquid_crossing_month_index.is_none() {
            liquid_crossing_month_index = Some(k);
        }
        // Latch (#141): `retired` solo puede pasar a true; el objetivo deja de mirarse después.
        // Se conserva la UNIÓN de 4.15.0 —cruce O mes forzado, es decir `min(cruce, s)`— en vez
        // de hacer exclusivo el trigger: la regla «un solo trigger por simulación» (D17) es de
        // ESTRATEGIA y la hace cumplir el handler. Con `crossing_is_reading_only` el cruce deja
        // de jubilar y solo se anota (D17).
        retired = retired
            || (fire_reached && !plan.crossing_is_reading_only)
            || plan.retirement_trigger.forced_month().map_or(false, |s| k >= s);
        if retired && retirement_month_index.is_none() {
            retirement_month_index = Some(k);
            // D17, «aviso rojo grande»: se entra en la jubilación con el líquido POR DEBAJO del
            // objetivo de ese mes. Se mira el OBJETIVO, no el trigger: si quien jubiló fue el
            // cruce, `liquid_prev ≥ t` por definición y esta rama no puede darse.
            if target_prev.is_some_and(|t| liquid_prev < t) {
                warnings.push(EngineWarning::RetireAtAgeUnderfunded);
            }
        }
        let in_retirement = retired;
        // Fase del mes (§B.1), monótona: `Retired` manda sobre `Partial`, y la parcial solo se
        // entra si el latch de jubilación aún no cerró.
        let phase = if in_retirement {
            Phase::Retired
        } else if plan.partial.is_some_and(|p| k >= p.start_month) {
            if partial_month_index.is_none() {
                partial_month_index = Some(k);
            }
            Phase::Partial
        } else {
            Phase::Accumulating
        };
        let income = match phase {
            Phase::Retired => plan.income_retirement_monthly,
            // Ingreso de la media jornada: PLANO como todos los ingresos del motor (#139).
            Phase::Partial => plan
                .partial
                .map_or(input.income_regular_monthly, |p| p.income_monthly),
            Phase::Accumulating => input.income_regular_monthly,
        };
        // Pausa de ingresos (P8.c): multiplica el ingreso GANADO de la fase, y solo dentro de la
        // ventana. Fuera de ella no se ejecuta ninguna multiplicación — por eso `factor_at`
        // devuelve `Option` y no un 1: `x·1` conserva el valor pero puede cambiar la escala, y la
        // escala es justo lo que el pin dorado hashea.
        let income = match plan.income_pause.and_then(|p| p.factor_at(k)) {
            Some(f) => income * f,
            None => income,
        };
        // #139: el GASTO se indexa al IPC de la instalación con el factor único sobre el MISMO
        // eje que el trigger del target, `(k−1)/12` — el mes 1 cobra el gasto base tal cual
        // (`f(1)=1`). Los INGRESOS quedan planos a propósito.
        let expense_factor = inflation_factor_at_index_g(input.annual_inflation_percent, k - 1);
        let expense = expense_factor
            * match phase {
                Phase::Retired => plan.expense_retirement_monthly,
                // D10: el gasto de la media jornada es CONFIGURABLE — el de jubilación por
                // defecto, el regular si el perfil lo dice. Mismo factor (#139).
                Phase::Partial => plan
                    .partial_expense_basis_monthly(input.expense_regular_monthly)
                    .unwrap_or(input.expense_regular_monthly),
                Phase::Accumulating => input.expense_regular_monthly,
            };

        // **Pensión con fecha** (§B.1 paso 3): es INGRESO en cualquier fase desde `start_index`,
        // con la rejilla 0-based (`k−1`) y el MISMO factor de inflación que el gasto del bucle si
        // está indexada. Durante la media jornada se cobra la fracción declarada (D8).
        //
        // Se suma SOLO si es positiva: sin pensión con fecha —el caso de 4.15.0— aquí no se
        // ejecuta ni una suma.
        let pension_income = match plan.pension {
            Some(pen) => {
                let gross = pen.monthly_at(k - 1, expense_factor);
                if matches!(phase, Phase::Partial) {
                    gross * pen.partial_fraction()
                } else {
                    gross
                }
            }
            None => M::zero(),
        };
        let income = if pension_income.is_zero() {
            income
        } else {
            income + pension_income
        };

        let retirement_withdrawal = if in_retirement {
            plan.extra_monthly_withdrawal
        } else {
            M::zero()
        };

        let net_cash_month =
            income - expense - debt_service + planning_adj - retirement_withdrawal;

        // REGLA DE RETIRADA (§B.2). El ancla de la fase jubilada (`L(R−1)`, `f(R−1)`) se fija el
        // PRIMER mes jubilado con los MISMOS escalares que el cruce acaba de usar, y el techo del
        // mes se pide UNA sola vez: `hybrid` y `guardrails` tienen memoria.
        //
        // `None` = sin techo. Lo es con `fixed_real` y lo es mientras el hogar no se ha jubilado.
        // **La media jornada NO pasa por la regla**: las reglas se anclan en `L(R−1)`, que en la
        // fase parcial todavía no existe.
        let allowed_gross = if in_retirement {
            planner.anchor_retirement(k, liquid_prev, expense_factor);
            planner.allowed_gross(k, liquid_prev, expense_factor)
        } else {
            None
        };

        // La necesidad NETA del mes: lo que la caja no cubre. Misma expresión y mismo valor que
        // el `need_assets_net` que 4.15.0 calculaba dentro de la rama de déficit.
        let need_assets_net = if net_cash_month <= M::zero() {
            -net_cash_month
        } else {
            M::zero()
        };

        // 4.12.1 (#175): la MISMA cascada, jubilado o no. `None`: el bucle corre hasta 840 veces
        // por request y nadie lee la traza aquí.
        //
        // **La venta ya no vive en un `else`**: baja a `execute_month_sale_g`, DESPUÉS del
        // reparto. Hasta 4.15.0 las dos ramas eran excluyentes, así que bajarla no mueve un
        // dígito de ningún caso de 4.15.0. Quien necesita ese orden es `rule_is_spend` (R7).
        if net_cash_month > M::zero() {
            // **Techo de aportación** (§B.7): la cascada solo ve `min(sobrante, c)`; el resto es
            // caja DISPONIBLE. Sin techo el pool es el sobrante entero y no se ejecuta ni una
            // operación de más: bit-identidad.
            let pool = match plan.contribution_cap_at(k) {
                Some(cap) => {
                    let invested = net_cash_month.min(cap);
                    let disposable = net_cash_month - invested;
                    if disposable > M::zero() {
                        disposable_series[k as usize] = disposable;
                        disposable_total = disposable_total + disposable;
                    }
                    invested
                }
                None => net_cash_month,
            };
            let (alloc, leftover) = distribute_contributions_g(
                pool,
                &input.allocation_rules,
                &values,
                expense + debt_service,
                income,
                None,
            );
            for i in 0..values.len() {
                if alloc[i] > M::zero() {
                    values[i] = values[i] + alloc[i];
                    // También jubilado (#120): lo reinvertido ES base de coste — sube b_i y
                    // abarata las ventas futuras (#178). Y desde aquí la base de este activo es
                    // un DATO observado.
                    basis[i] = basis[i] + alloc[i];
                    basis_declared[i] = true;
                }
            }
            if leftover > M::zero() {
                // El euro sin destino declarado NO se simula — fuera del balance, solo
                // cuantificado. Inalcanzable en producción (#176).
                unallocated_savings_total = unallocated_savings_total + leftover;
            }
        }

        // La venta del mes: la necesidad (topada por la regla) y/o —en `rule_is_spend`— el gasto
        // que la regla ES. Devuelve las TRES magnitudes de B.1.5 ya separadas.
        let sale = execute_month_sale_g(
            &mut values,
            &mut basis,
            &basis_declared,
            &liquid,
            &rates,
            input.taxable_gain_ratio,
            &input.tax_brackets,
            input.taxes_enabled,
            need_assets_net,
            allowed_gross,
            plan.spend_mode,
            assets_depleted_month_index.is_none(),
        );
        if sale.depleted_portfolio && assets_depleted_month_index.is_none() {
            assets_depleted_month_index = Some(k);
        }
        // Solo el descubierto RESTA patrimonio (D22/D24): el recorte y el sobrante de la regla
        // son lecturas. `None` = no hubo venta este mes y el acumulador NO se toca.
        if let Some(undrained_month) = sale.undrained {
            undrained_cumulative = undrained_cumulative + undrained_month;
        }

        // **El colchón de caja** (P4, §B.6): DESPUÉS de la venta —el colchón se rellena sobre el
        // saldo que la retirada del mes ya dejó— y ANTES del crecimiento, para que el euro
        // trasvasado componga este mes donde de verdad está.
        //
        // Tres puertas, y las tres tienen que abrirse: hay colchón declarado, el hogar está
        // JUBILADO (antes de jubilarse la cascada ya reparte el superávit y no hay retirada de la
        // que protegerse) y el mes está AUTORIZADO (en Monte Carlo, shock positivo). Sin colchón
        // —el camino determinista— no se ejecuta ni una comparación de más.
        let mut buffer_refill = M::zero();
        if in_retirement {
            if let Some(cb) = input
                .cash_buffer
                .as_ref()
                .filter(|cb| cb.buffer_index < values.len())
            {
                if cb
                    .refill_months
                    .get((k - 1) as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    // Objetivo del mes: `n` meses del gasto YA INDEXADO menos lo que el colchón
                    // conserva. Es el gasto de la fase (`expense`), no el gasto con deuda que usa
                    // el tope `MonthsExpense` de la cascada: el colchón cubre la RETIRADA, y el
                    // servicio de deuda ya sale de la caja del mes antes de que exista déficit.
                    let target_net =
                        (cb.target_months * expense - values[cb.buffer_index]).max(M::zero());
                    buffer_refill = refill_cash_buffer_g(
                        &mut values,
                        &mut basis,
                        &mut basis_declared,
                        &liquid,
                        &rates,
                        input.taxable_gain_ratio,
                        &input.tax_brackets,
                        input.taxes_enabled,
                        cb.buffer_index,
                        target_net,
                    );
                    if buffer_refill > M::zero() {
                        buffer_refill_months += 1;
                    }
                }
            }
        }

        // **Crecimiento.** El slice de factores del mes se elige UNA vez (no un `if` por activo):
        // sin `growth_overrides` —el único caso del camino determinista— es el vector hoisted de
        // siempre, así que el bucle interior ejecuta exactamente las mismas operaciones. Una fila
        // de overrides mal dimensionada se ignora en vez de panicar: el motor es una función pura.
        let month_growth: &[M] = input
            .growth_overrides
            .as_ref()
            .and_then(|ov| ov.get((k - 1) as usize))
            .filter(|row| row.len() == values.len())
            .map(|row| row.as_slice())
            .unwrap_or(growth_multipliers.as_slice());
        for i in 0..values.len() {
            let m = month_growth[i];
            // `checked_mul`, no `*`: con una tasa desorbitada y horizonte largo el producto
            // desborda `Decimal` y `*` PANICA — el pool blocking lo convertía en un 400
            // `task_panic` permanente e ininteligible. Error tipado.
            values[i] = values[i]
                .checked_mul(m)
                .ok_or(EngineError::AssetValueOverflow)?;
        }

        // Amortización: solo se asienta el cierre ya calculado arriba. Sin recomputar nada.
        for (i, closing) in closing_principals.iter().enumerate() {
            principals[i] = *closing;
        }

        let nw = nw_fn(&values, &principals, undrained_cumulative);
        net_series.push(nw);
        withdrawal_series.push(sale.net_obtained);
        shortfall_series.push(sale.shortfall);
        excess_series.push(sale.excess);
        buffer_refill_series.push(buffer_refill);
        let liquid_close = liquid_fn(&values);
        // §B.3: ¿la media jornada deja crecer el capital? Se compara el cierre del mes con el
        // cierre del anterior —el mismo par que el cruce usa— y basta UN mes a la baja.
        if matches!(phase, Phase::Partial) && liquid_close < liquid_series[(k - 1) as usize] {
            partial_capital_shrank = true;
        }
        liquid_series.push(liquid_close);
        contrib_series.push(contributed_fn(&basis));
        for (i, s) in per_asset_series.iter_mut().enumerate() {
            s.push(values[i]);
        }
    }

    // Fases atravesadas (§B.8), en orden y con el mes 1-based en que empieza cada una.
    // `partial_month_index` solo se rellena si la fase se pisó de verdad — una media jornada
    // declarada DESPUÉS del cruce nunca ocurre, y publicarla igualmente pintaría en el chart una
    // fase que la simulación no vivió.
    let mut phase_transitions: Vec<(Phase, u32)> = Vec::with_capacity(3);
    phase_transitions.push((Phase::Accumulating, 0));
    if let Some(k) = partial_month_index {
        phase_transitions.push((Phase::Partial, k));
    }
    if let Some(k) = retirement_month_index {
        phase_transitions.push((Phase::Retired, k));
    }

    // -----------------------------------------------------------------------------------------
    // Lecturas de WP3 (§B.3). Ninguna toca la aritmética: todas se derivan de series ya cerradas.
    // -----------------------------------------------------------------------------------------
    if partial_month_index.is_some() && partial_capital_shrank {
        warnings.push(EngineWarning::PartialPhaseCapitalShrinking);
    }
    let partial_phase_capital_growing = partial_month_index.is_some() && !partial_capital_shrank;

    // Primer mes del BUCLE (1-based) en que la pensión con fecha entra en caja. El mes `k`
    // evalúa el índice `k−1`, así que la pensión de `start_index` se cobra en `start_index + 1`.
    let pension_start_month_index = plan.pension.and_then(|pen| {
        let month = pen.start_index.saturating_add(1);
        (month <= input.horizon_months).then_some(month)
    });

    // Tasa de retirada efectiva del puente: `100 · 12·need_full_m(R−1) / L(R−1)`.
    // `12·need_full_m(i)` ES `need_full_annual_at(i)` — no se multiplica y divide por 12 para
    // volver al mismo sitio.
    let bridge_effective_withdrawal_pct = (plan.pension.is_some()
        && plan.target_basis == crate::phases::TargetBasis::BridgeToPension)
        .then(|| {
            let r = retirement_month_index?;
            let liquid_at_r = *liquid_series.get((r - 1) as usize)?;
            if liquid_at_r <= M::zero() {
                return None;
            }
            let need_annual = plan_target.need_full_annual_at(ft_view, r - 1)?;
            need_annual
                .checked_mul(M::from_u32(100))
                .and_then(|x| x.checked_div(liquid_at_r))
        })
        .flatten();

    let pension_coverage_ratio = plan_target.pension_coverage_ratio(ft_view);
    let partial_gap_target =
        plan_target.partial_gap_target(ft_view, plan, input.expense_regular_monthly);

    Ok(SimOutput {
        net_worth: net_series,
        contributed_capital: contrib_series,
        per_asset_series,
        assets_depleted_month_index,
        uncovered_deficit_total: undrained_cumulative,
        unallocated_savings_total,
        liquid_worth: liquid_series,
        retirement_month_index,
        liquid_crossing_month_index,
        phase_transitions,
        withdrawal: withdrawal_series,
        // Con `fixed_real` estas dos siguen siendo cero mes a mes —el permitido ES la necesidad—,
        // pero ahora por el camino general, no por un `vec![0]` que fingía calcularlas.
        withdrawal_shortfall: shortfall_series,
        withdrawal_excess: excess_series,
        pension_start_month_index,
        partial_retirement_month_index: partial_month_index,
        warnings,
        bridge_effective_withdrawal_pct,
        pension_coverage_ratio,
        partial_gap_target,
        partial_phase_capital_growing,
        disposable_cash: disposable_series,
        disposable_cash_total: disposable_total,
        buffer_refill_net: buffer_refill_series,
        buffer_refill_months,
    })
}

// =============================================================================================
// Tests del colchón (P4)
// =============================================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn es_brackets_g() -> Vec<TaxBracketG<Decimal>> {
        TaxBracketG::<Decimal>::from_decimal_slice(&crate::tax::es_brackets_for_tests())
    }

    fn dec(v: &str) -> Decimal {
        v.parse().expect("literal decimal válido")
    }

    /// **El relleno del colchón, derivado a mano antes de ejecutarlo.**
    ///
    /// Dos activos: colchón al 0 % con 1.000 € (base 1.000 ⇒ `g₀ = 0`) y renta variable al 10 %
    /// con 100.000 € y base 50.000 ⇒ `g₁ = 1 − 50.000/100.000 = 0,5`. Escala ES, impuestos ON,
    /// objetivo **3.000 € netos**.
    ///
    /// El conjunto vendible es `{1}` (el colchón nunca se vende a sí mismo), así que la `g` es
    /// uniforme y la venta va por la vía escalar:
    ///
    /// ```text
    ///   gross_up_anual(36.000, g = 0,5):  base B = 0,5·G cae en el tramo del 21 %
    ///     tax(B) = 1.140 + 0,21·(B − 6.000) = 0,21·B − 120
    ///     G − 0,21·0,5·G + 120 = 36.000  ⇒  0,895·G = 35.880  ⇒  G = 40.089,3854748603351955…
    ///     comprobación: B = 20.044,69… ∈ (6.000, 50.000] ✓
    ///   venta BRUTA mensual = G/12 = 3.340,78212290502793296…
    ///   impuesto           = 3.340,782… − 3.000 = 340,78212290502793296…
    /// ```
    ///
    /// Predicciones, escritas ANTES de ejecutar:
    ///
    /// | magnitud | predicho |
    /// |---|---|
    /// | neto movido | 3.000 exacto (par redondo `after_tax(gross_up(n)) = n`) |
    /// | colchón: valor y base | 4.000 y 4.000 |
    /// | RV: valor | 100.000 − 3.340,782122905027932960 = 96.659,217877094972067039… |
    /// | RV: base | 50.000·96.659,2178…/100.000 = 48.329,6089385474860335… |
    /// | patrimonio total | baja EXACTAMENTE el impuesto: 101.000 − 340,7821229050279329… |
    #[test]
    fn refill_cash_buffer_sells_gross_credits_net_and_shrinks_the_basis() {
        let mut values = vec![dec("1000"), dec("100000")];
        let mut basis = vec![dec("1000"), dec("50000")];
        let mut declared = vec![true, true];
        let liquid = vec![true, true];
        let rates = vec![Some(Decimal::ZERO), Some(Decimal::from(10))];
        let brackets = es_brackets_g();

        // El colchón es el líquido de menor rentabilidad: el índice 0.
        let assets: Vec<crate::sim::SimAssetG<Decimal>> = vec![
            crate::sim::SimAssetG {
                value: values[0],
                purchase_price: Some(basis[0]),
                is_liquid: true,
                expected_annual_return_percent: rates[0],
            },
            crate::sim::SimAssetG {
                value: values[1],
                purchase_price: Some(basis[1]),
                is_liquid: true,
                expected_annual_return_percent: rates[1],
            },
        ];
        assert_eq!(cash_buffer_index(&assets), Some(0));

        let total_before: Decimal = values.iter().copied().sum();
        let net = refill_cash_buffer_g(
            &mut values,
            &mut basis,
            &mut declared,
            &liquid,
            &rates,
            Decimal::ONE,
            &brackets,
            true,
            0,
            dec("3000"),
        );
        let total_after: Decimal = values.iter().copied().sum();
        println!(
            "\n[colchón] neto movido      = {net}\n\
             [colchón] colchón valor/base = {} / {}\n\
             [colchón] RV      valor/base = {} / {}\n\
             [colchón] patrimonio {total_before} → {total_after} (impuesto {})",
            values[0],
            basis[0],
            values[1],
            basis[1],
            total_before - total_after
        );

        // (1) El neto movido es EXACTAMENTE el objetivo: el par gross_up/after_tax es redondo.
        assert_eq!(net, dec("3000"));
        // (2) El colchón sube en el neto, y ese euro es BASE (ya tributó al salir de la RV).
        assert_eq!(values[0], dec("4000"));
        assert_eq!(basis[0], dec("4000"));
        assert!(declared[0]);
        // (3) La RV baja en el BRUTO vendido, no en el neto.
        let gross_sold = dec("100000") - values[1];
        let expected_gross = dec("3340.782122905027932960893855");
        assert!(
            (gross_sold - expected_gross).abs() < dec("0.000000000000000001"),
            "venta bruta {gross_sold}, predicha {expected_gross}"
        );
        // (4) La base baja PROPORCIONALMENTE al valor vendido (#120): b' = b·v_post/v_pre.
        let expected_basis = dec("50000") * values[1] / dec("100000");
        assert_eq!(basis[1], expected_basis);
        // (5) Lo único que el trasvase destruye es el impuesto — ni un euro más.
        assert_eq!(total_before - total_after, gross_sold - net);
    }

    /// Sin nada que vender no hay relleno, y el colchón **jamás se vende a sí mismo** (eso sería
    /// un trasvase circular que sube su propia base sin mover un euro).
    #[test]
    fn refill_cash_buffer_never_sells_the_buffer_itself() {
        let mut values = vec![dec("1000")];
        let mut basis = vec![dec("1000")];
        let mut declared = vec![true];
        let net = refill_cash_buffer_g(
            &mut values,
            &mut basis,
            &mut declared,
            &[true],
            &[Some(Decimal::ZERO)],
            Decimal::ONE,
            &es_brackets_g(),
            true,
            0,
            dec("5000"),
        );
        assert_eq!(net, Decimal::ZERO);
        assert_eq!(values[0], dec("1000"));
        assert_eq!(basis[0], dec("1000"));
    }

    /// Si la cartera no llega al objetivo se mueve **lo que haya**, sin descubierto: un relleno es
    /// discrecional. Sin impuestos el bruto ES el neto, así que el número es exacto a mano.
    #[test]
    fn refill_cash_buffer_moves_what_it_can_without_creating_a_deficit() {
        let mut values = vec![dec("100"), dec("700")];
        let mut basis = vec![dec("100"), dec("700")];
        let mut declared = vec![true, true];
        let net = refill_cash_buffer_g(
            &mut values,
            &mut basis,
            &mut declared,
            &[true, true],
            &[Some(Decimal::ZERO), Some(Decimal::from(10))],
            Decimal::ONE,
            &[],
            false,
            0,
            dec("5000"),
        );
        assert_eq!(net, dec("700"), "se mueve toda la capacidad, no más");
        assert_eq!(values[0], dec("800"));
        assert_eq!(values[1], Decimal::ZERO);
        assert_eq!(basis[1], Decimal::ZERO, "vender el activo entero deja base 0");
    }

    /// `cash_buffer_index` elige entre los LÍQUIDOS y solo entre ellos: una vivienda al 1 % no
    /// puede hacer de colchón por mucho que sea el activo de menor rentabilidad.
    #[test]
    fn cash_buffer_index_only_considers_liquid_assets() {
        let asset = |value: u32, liquid: bool, rate: Option<u32>| crate::sim::SimAssetG {
            value: Decimal::from(value),
            purchase_price: None,
            is_liquid: liquid,
            expected_annual_return_percent: rate.map(Decimal::from),
        };
        // Ilíquido al 1 %, líquido al 7 %, líquido al 2 % ⇒ gana el índice 2.
        let assets = vec![
            asset(1, false, Some(1)),
            asset(1, true, Some(7)),
            asset(1, true, Some(2)),
        ];
        assert_eq!(cash_buffer_index(&assets), Some(2));
        // Empate por rentabilidad ⇒ menor índice.
        let tie = vec![asset(1, true, Some(3)), asset(1, true, Some(3))];
        assert_eq!(cash_buffer_index(&tie), Some(0));
        // Sin líquidos no hay colchón posible.
        assert_eq!(cash_buffer_index(&[asset(1, false, None)]), None);
        assert_eq!(cash_buffer_index::<Decimal>(&[]), None);
    }
}
