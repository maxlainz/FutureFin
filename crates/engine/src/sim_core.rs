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
    RuleOutcomeG, SimInput, SimLiability, SimOutput, TaxBracketG,
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
pub(crate) fn debt_term_at_index_g<M: MoneyOps>(
    debt_payments_remaining: &[M],
    month_index: u32,
) -> M {
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
    let fee = extra
        * liab
            .early_repayment_fee_pct
            .unwrap_or(M::zero())
            .max(M::zero())
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
    /// **La venta dejó la cartera vendible a cero.** Lo dice la VENTA, no una comparación entre
    /// dos cantidades calculadas por separado: hasta la revisión adversarial esto era
    /// `target_gross >= drainable` ANTES de vender, un filo de navaja que `Decimal` y `f64`
    /// resolvían al revés en el aterrizaje exacto (medido: la misma entrada daba `None` en
    /// `Decimal` y `Some(120)` en `f64`, que es el tipo sobre el que corre cada camino de Monte
    /// Carlo). Ahora se mira el saldo DESPUÉS: si cada activo se llevó su capacidad entera, la
    /// suma de lo vendible es cero EXACTO en los dos tipos (`x − x = 0`).
    depleted_portfolio: bool,
    /// **La venta no pudo fundar el bruto que perseguía.** Es la señal de FALLO real, y la
    /// publica el paseo (`und_gross > 0`, `!cap_exhausted`, `net_shortfall > 0`), nunca una
    /// resta de dos netos: `undrained` puede salir ±1e-24 por cola de redondeo y no distingue.
    ///
    /// Sin ella, vaciar la cartera con un aterrizaje EXACTO cuyo gasto posterior está cubierto
    /// (el puente que acaba justo cuando entra la pensión) se publicaba como «cartera agotada».
    unfunded_sale: bool,
    /// **El NETO que la REGLA de retirada permitió este mes** (E1). `None` ⟺ no había techo
    /// (`fixed_real`, o mes no jubilado): entonces no hay nada que comparar y F3 no aplica.
    ///
    /// No es `attempted_net`, y la distinción es de dinero: `attempted_net` vale `need_net`
    /// cuando el techo NO ata, así que un mes con un «Próximo» que cubre la caja (necesidad neta
    /// pequeña, necesidad ordinaria grande) publicaría un intentado por debajo de la ordinaria
    /// con una regla que permitía de sobra — F3 se encendería por un ingreso puntual, que es
    /// justo lo que C1 excluye del veredicto.
    rule_allowance_net: Option<M>,
}

impl<M: MoneyOps> MonthSale<M> {
    fn empty() -> Self {
        Self {
            net_obtained: M::zero(),
            undrained: None,
            shortfall: M::zero(),
            excess: M::zero(),
            depleted_portfolio: false,
            unfunded_sale: false,
            rule_allowance_net: None,
        }
    }

    /// Reparte el resultado de la venta entre las tres magnitudes.
    ///
    /// `attempted_net` = el neto que la venta intentada pretendía obtener; `target_is_need` = esa
    /// venta ERA la necesidad (sin techo, o con un techo que no ataba).
    ///
    /// `literal_undrained` = el descubierto que la propia venta PUBLICA como operando (el
    /// `net_shortfall_monthly` del paseo mixto). Cuando existe se acumula TAL CUAL: 4.15.0 hacía
    /// `undrained_cumulative += dd.net_shortfall_monthly`, y re-derivarlo como
    /// `need − (need − s)` no devuelve `s` en `Decimal` — ni el mismo dígito 28 ni la misma
    /// ESCALA, que es lo que el `Display` del pin dorado hashea (`0` frente a `0.0000`).
    fn account(
        &mut self,
        need_net: M,
        attempted_net: M,
        obtained_net: M,
        target_is_need: bool,
        forced_by_rule: bool,
        sold: bool,
        literal_undrained: Option<M>,
    ) {
        self.net_obtained = obtained_net;
        // DESCUBIERTO. Lo que los activos no pudieron fundar de la venta intentada, **acotado por
        // la necesidad real**: bajo `rule_is_spend`, la parte discrecional de una venta que la
        // cartera no cubre no es deuda — nadie se endeuda para gastar de más. Con el objetivo =
        // necesidad se conserva la expresión LITERAL de 4.15.0 (sin `min` ni `max`), que es lo
        // que mantiene el pin dorado bit a bit.
        self.undrained = sold.then(|| match literal_undrained {
            Some(u) => u,
            None if target_is_need => need_net - obtained_net,
            None => (attempted_net.min(need_net) - obtained_net).max(M::zero()),
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

/// **La plusvalía relativa de cada activo y la `g` única del mes, si existe** (#178).
///
/// `g_i = 1 − b_i/v_i` clampada a `[0,1]` cuando la base es un DATO (`purchase_price` declarado —
/// aunque sea 0 — o base alimentada por la propia cascada); el ESCALAR configurado si no lo es.
/// El segundo elemento es `Some(g)` cuando todos los activos CON VALOR comparten `g` (camino
/// escalar literal de 4.11.0) y `None` cuando hay que pasear por tramos.
///
/// `checked_div`, no `/` (issue #208): `*v > 0` no basta como guarda — una rentabilidad muy
/// negativa deja el valor pegado al mínimo representable con la base entera y `b/v` se sale de
/// rango.
fn month_gains_g<M: MoneyOps>(
    values: &[M],
    basis: &[M],
    basis_declared: &[bool],
    scalar_gain_ratio: M,
) -> (Vec<M>, Option<M>) {
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
    (gains, effective_uniform)
}

/// Los tramos del paseo mixto, en el ORDEN de [`drain_order_g`] — divergir de él gravaría una
/// venta que no ocurre.
fn mixed_segments_g<M: MoneyOps>(
    order: &[usize],
    values: &[M],
    gains: &[M],
) -> Vec<MixedSegment<M>> {
    order
        .iter()
        .map(|&i| MixedSegment {
            capacity_monthly: values[i].max(M::zero()),
            gain_ratio: gains[i],
        })
        .collect()
}

/// Los mismos tramos con la capacidad del ÚLTIMO con material ampliada en `bound`.
///
/// Sirve para **tasar un bruto que la cartera no puede fundar**, que es justo lo que la vía
/// escalar hace gratis (una `g` única pone precio a cualquier bruto). El tramo marginal es el
/// que la venta vaciaría al final, así que su `g` es la del euro siguiente. Cuando todas las `g`
/// coinciden, el paseo sobre estos tramos devuelve `after_tax_monthly(bruto, g)` dígito a dígito.
fn extend_marginal_g<M: MoneyOps>(segments: &[MixedSegment<M>], bound: M) -> Vec<MixedSegment<M>> {
    let mut v = segments.to_vec();
    if let Some(j) = v.iter().rposition(|s| s.capacity_monthly > M::zero()) {
        v[j].capacity_monthly = v[j].capacity_monthly + bound.max(M::zero());
    }
    v
}

/// **Lo que NETEA vender `gross`** con la fiscalidad del mes, tase o no la cartera ese bruto.
/// Es la operación que la vía escalar escribe como `after_tax_monthly` y la mixta como un paseo
/// por tramos; una sola definición para que el presupuesto del mes (`rule_is_spend`) y el
/// reparto de magnitudes de la venta no puedan discrepar.
fn net_of_gross_g<M: MoneyOps>(
    gross: M,
    values: &[M],
    gains: &[M],
    effective_uniform: Option<M>,
    liquid: &[bool],
    rates: &[Option<M>],
    brackets: &[TaxBracketG<M>],
    taxes_enabled: bool,
) -> M {
    if gross <= M::zero() {
        return M::zero();
    }
    match effective_uniform {
        Some(g) => crate::tax::after_tax_monthly_g(gross, brackets, taxes_enabled, g),
        None => {
            let order = drain_order_g(liquid, rates);
            let segments = mixed_segments_g(&order, values, gains);
            crate::tax::mixed_drawdown_for_gross_cap(
                gross,
                &extend_marginal_g(&segments, gross),
                brackets,
                taxes_enabled,
            )
            .net_monthly
        }
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
    spend_from_cash: M,
) -> MonthSale<M> {
    let mut out = MonthSale::empty();

    // Objetivo BRUTO forzado por la regla: solo en `rule_is_spend` y solo si hay techo.
    let forced_gross = match spend_mode {
        SpendMode::RuleIsSpend => allowed_gross,
        SpendMode::Ceiling => None,
    };
    match forced_gross {
        // Techo no positivo (cartera vacía bajo `percent_of_balance`): no se vende nada, pero el
        // recorte sigue siendo la necesidad entera. La regla permitió CERO neto, y eso es un dato
        // (F3 lo compara contra la necesidad ordinaria), no una ausencia.
        Some(a) if a <= M::zero() => {
            out.shortfall = need_net.max(M::zero());
            out.rule_allowance_net = Some(M::zero());
            return out;
        }
        Some(_) => {}
        // Sin venta forzada y sin necesidad no hay nada que hacer: es el mes de superávit de
        // 4.15.0, donde la rama de déficit ni se rozaba.
        //
        // **Pero el permitido de la regla sigue existiendo** y F3 lo necesita: un mes puede no
        // tener déficit de CAJA (un «Próximo» lo tapó) y tener necesidad ordinaria por encima de
        // lo que la regla deja sacar. Se tasa solo si hay techo — con `fixed_real` (el camino de
        // 4.15.0 y el de los pines) no se ejecuta ni una operación de más.
        None if need_net <= M::zero() => {
            if let Some(a) = allowed_gross.filter(|a| *a > M::zero()) {
                let (gains, uniform) =
                    month_gains_g(values, basis, basis_declared, scalar_gain_ratio);
                out.rule_allowance_net = Some(net_of_gross_g(
                    a,
                    values,
                    &gains,
                    uniform,
                    liquid,
                    rates,
                    brackets,
                    taxes_enabled,
                ));
            }
            return out;
        }
        None => {}
    }

    // Cortocircuito de `g` uniforme (sobre lo VENDIBLE): camino LITERAL de 4.11.0, operando a
    // operando — el paseo mixto es algebraicamente igual pero no bit a bit (trocear un tramo
    // lineal añade divisiones que se redondean).
    let (gains, effective_uniform) =
        month_gains_g(values, basis, basis_declared, scalar_gain_ratio);

    if let Some(g_scalar) = effective_uniform {
        // #140 fase 1: lo que falta se cubre VENDIENDO, y la venta tributa — el bruto a drenar es
        // gross_up_monthly(neto). Con `taxes_enabled = false` es la identidad.
        //
        // 5.0.0 WP2: sobre ese bruto se aplica el techo de la regla. `target_is_need` distingue
        // «la venta ERA la necesidad» (el camino de 4.15.0) de «la venta la fijó la regla».
        let (target_gross, target_is_need, forced_spend_net) = match forced_gross {
            // **El gasto de la regla se financia PRIMERO con la caja del mes** (R7 corregida por
            // la revisión adversarial): vender un fondo para gastar euros que la nómina ya puso
            // sobre la mesa —y reinvertir esos euros en el MISMO fondo— realiza plusvalía por
            // nada. Medido: 3.991,72 €/año de impuesto donde el hecho económico costaba 373,11
            // (×10,7). El techo sigue siendo BRUTO, así que la caja se descuenta en NETO y lo
            // que queda se vuelve a grossear.
            Some(a) => {
                let spend_net =
                    crate::tax::after_tax_monthly_g(a, brackets, taxes_enabled, g_scalar);
                // El permitido de la regla, en neto: ya está tasado, no se vuelve a tasar.
                out.rule_allowance_net = Some(spend_net);
                let to_sell_net = spend_net - spend_from_cash.max(M::zero());
                let gross = if to_sell_net <= M::zero() {
                    M::zero()
                } else {
                    crate::tax::gross_up_monthly_g(to_sell_net, brackets, taxes_enabled, g_scalar)
                };
                (gross, false, Some(spend_net))
            }
            None => {
                let need_gross =
                    crate::tax::gross_up_monthly_g(need_net, brackets, taxes_enabled, g_scalar);
                // El permitido de la regla se tasa SIEMPRE que haya techo, ate o no: cuando no
                // ata, `attempted_net` pasa a ser la necesidad y dejaría de decir qué permitía la
                // regla — el operando que F3 necesita.
                if let Some(a) = allowed_gross {
                    out.rule_allowance_net = Some(crate::tax::after_tax_monthly_g(
                        a,
                        brackets,
                        taxes_enabled,
                        g_scalar,
                    ));
                }
                match allowed_gross {
                    Some(a) if a < need_gross => (a, false, None),
                    _ => (need_gross, true, None),
                }
            }
        };
        let mut drawn_net = M::zero();
        if target_gross > M::zero() {
            let mut taken = vec![M::zero(); values.len()];
            let und_gross =
                drain_from_assets_g(values, liquid, rates, target_gross, Some(&mut taken));
            // FALLO de la venta: el paseo lo publica, no se deduce. `und_gross > 0` ⟺ las
            // capacidades no llegaron al bruto perseguido.
            out.unfunded_sale = und_gross > M::zero();
            // Mes de agotamiento (#119), medido DESPUÉS de vender: **o la venta no se pudo
            // fundar, o la cartera vendible se quedó a cero**. Si cada activo entregó su
            // capacidad entera, `v − v = 0` EXACTO en los dos tipos numéricos; el viejo
            // `target_gross >= drainable` comparaba dos cantidades calculadas por caminos
            // distintos y `Decimal` y `f64` lo resolvían al revés en el aterrizaje exacto.
            //
            // El primer término no es redundante: el paseo mixto reparte `taken/12` por tramo y
            // puede dejar una brizna de ULP en un activo cuya capacidad SÍ agotó, y sin él un mes
            // con descubierto real quedaría sin marcar (invariante roto: `uncovered > 0` con
            // `assets_depleted_month_index = None`).
            if watch_depletion {
                let left: M = M::sum_of(values.iter().map(|v| (*v).max(M::zero())));
                out.depleted_portfolio = out.unfunded_sale || left <= M::zero();
            }
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
        // El gasto de la regla es UNO, lo pague la caja o la venta: el intentado es el neto
        // entero y lo obtenido incluye la parte de caja. Solo así `withdrawal` sigue siendo «lo
        // que el hogar gastó» y el sobrante de la regla no se parte en dos.
        let (attempted_net, obtained_net) = match forced_spend_net {
            Some(spend_net) => (spend_net, spend_from_cash.max(M::zero()) + drawn_net),
            None if target_is_need => (need_net, drawn_net),
            None => (
                crate::tax::after_tax_monthly_g(target_gross, brackets, taxes_enabled, g_scalar),
                drawn_net,
            ),
        };
        out.account(
            need_net,
            attempted_net,
            obtained_net,
            target_is_need,
            forced_gross.is_some(),
            target_gross > M::zero() || forced_spend_net.is_some(),
            // Vía escalar: 4.15.0 acumulaba `need_assets_net − after_tax(drawn_gross)`, que es
            // exactamente `need_net − obtained_net`. La expresión literal ya la escribe
            // `account`, así que aquí no hay operando que publicar.
            None,
        );
    } else {
        // Vía MIXTA (#178): el solver por tramos decide venta bruta Y reparto a la vez — la base
        // agregada `Σ g_i·venta_i` atraviesa los tramos progresivos y ninguna `g` escalar puede
        // representarla. El orden es EL MISMO de `drain_from_assets_g`.
        let order = drain_order_g(liquid, rates);
        let segments = mixed_segments_g(&order, values, &gains);
        // **El precio del bruto que la cartera NO puede fundar.** La vía escalar tasa cualquier
        // bruto con su `g` única, también uno por encima de lo vendible: por eso
        // `attempted_net = after_tax(techo)` existe siempre allí. Aquí no hay `g` única, así que
        // se extiende la capacidad del ÚLTIMO tramo con material (el marginal, el que la venta
        // vaciaría al final) por el bruto que se está tasando. Es la generalización EXACTA de la
        // vía escalar: si todas las `g` coinciden, el paseo sobre estos tramos devuelve
        // `after_tax_monthly(bruto, g)` dígito a dígito.
        //
        // Sin esto, un techo por encima de la capacidad NO ataba (se comparaba contra
        // `dd.gross_monthly`, que el paseo ya había recortado a la capacidad) y el rechazo de la
        // regla se contabilizaba como DESCUBIERTO: 916 € de patrimonio en el caso `b1` de la
        // revisión, con la venta byte a byte idéntica a la de la vía escalar.
        let extended = |bound: M| -> Vec<MixedSegment<M>> { extend_marginal_g(&segments, bound) };
        // ¿ATA el techo? Se decide como en la vía escalar: contra lo que la NECESIDAD pide, no
        // contra lo que la cartera da. `attempted_net(a) < need_net` ⟺ `a < need_gross`, porque
        // el neto es monótono creciente en el bruto.
        let binding = match (forced_gross, allowed_gross) {
            (Some(a), _) => Some((a, None)),
            (None, Some(a)) => {
                let w = crate::tax::mixed_drawdown_for_gross_cap(
                    a,
                    &extended(a),
                    brackets,
                    taxes_enabled,
                );
                // Mismo criterio que la vía escalar: el permitido de la regla se publica ate o no
                // el techo. Aquí ya estaba calculado — decidir si ata EXIGE tasarlo.
                out.rule_allowance_net = Some(w.net_monthly);
                (w.net_monthly < need_net).then_some((a, Some(w.net_monthly)))
            }
            (None, None) => None,
        };
        let (per_segment, obtained_net, attempted_net, target_is_need, literal_undrained, unfunded) =
            match binding {
                Some((a, precomputed_net)) => {
                    // El neto que la regla PERMITIÓ, tasado sobre el tramo marginal extendido. Ya no
                    // se cae a `need_net` cuando la capacidad no llega: esa caída era justo la que
                    // convertía el recorte de la regla en descubierto.
                    let attempted_net = match precomputed_net {
                        Some(n) => n,
                        None => {
                            crate::tax::mixed_drawdown_for_gross_cap(
                                a,
                                &extended(a),
                                brackets,
                                taxes_enabled,
                            )
                            .net_monthly
                        }
                    };
                    if forced_gross.is_some() {
                        // `rule_is_spend`: el intentado ES el permitido de la regla (el gasto que
                        // la regla manda), lo pague la caja o la venta.
                        out.rule_allowance_net = Some(attempted_net);
                    }
                    // **La caja del mes paga primero** (fix D, gemelo de la vía escalar): el bruto a
                    // vender es el que grossea el neto que la caja NO cubre.
                    let to_sell_net = match forced_gross {
                        Some(_) => attempted_net - spend_from_cash.max(M::zero()),
                        None => attempted_net,
                    };
                    let sell_gross = if forced_gross.is_some() {
                        if to_sell_net <= M::zero() {
                            M::zero()
                        } else {
                            crate::tax::gross_up_mixed_monthly(
                                to_sell_net,
                                &extended(a),
                                brackets,
                                taxes_enabled,
                            )
                            .gross_monthly
                        }
                    } else {
                        a
                    };
                    let w = crate::tax::mixed_drawdown_for_gross_cap(
                        sell_gross,
                        &segments,
                        brackets,
                        taxes_enabled,
                    );
                    // **Lo dice el paseo, no una comparación** (WP5.5): `w.gross_monthly >= a` era
                    // exacto en `Decimal` y un filo de navaja en aritmética aproximada.
                    let unfunded = sell_gross > M::zero() && !w.cap_exhausted;
                    let obtained = if forced_gross.is_some() {
                        spend_from_cash.max(M::zero()) + w.net_monthly
                    } else {
                        w.net_monthly
                    };
                    (
                        w.per_segment_monthly,
                        obtained,
                        attempted_net,
                        false,
                        None,
                        unfunded,
                    )
                }
                None => {
                    let dd = crate::tax::gross_up_mixed_monthly(
                        need_net,
                        &segments,
                        brackets,
                        taxes_enabled,
                    );
                    // El descubierto sale NETO por construcción del solver — sin segunda llamada.
                    let obtained = need_net - dd.net_shortfall_monthly;
                    let unfunded = dd.net_shortfall_monthly > M::zero();
                    (
                        dd.per_segment_monthly,
                        obtained,
                        need_net,
                        true,
                        // El OPERANDO de 4.15.0, no su reconstrucción: el bucle hacía
                        // `undrained_cumulative += dd.net_shortfall_monthly`.
                        Some(dd.net_shortfall_monthly),
                        unfunded,
                    )
                }
            };
        out.unfunded_sale = unfunded;
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
        // Agotamiento (#119) medido DESPUÉS de vender, igual que en la vía escalar: la venta no
        // se pudo fundar, o lo vendible quedó a cero.
        if watch_depletion {
            let left: M = M::sum_of(values.iter().map(|v| (*v).max(M::zero())));
            out.depleted_portfolio = out.unfunded_sale || left <= M::zero();
        }
        out.account(
            need_net,
            attempted_net,
            obtained_net,
            target_is_need,
            forced_gross.is_some(),
            true,
            literal_undrained,
        );
    }

    out
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
        // AGRUPACIÓN LITERAL DE 4.15.0. El original era `debt_service += cash + extra + fee`,
        // es decir `acc + ((cash + extra) + fee)`. Desparejar el `+=` en tres sumas sueltas
        // (`((acc + cash) + extra) + fee`) es la MISMA álgebra y NO el mismo número: en
        // `Decimal` cada suma redondea a 28 dígitos y la asociatividad se pierde en el último.
        // Costó 67 casos de `net_worth` en el fuzz diferencial contra 4.15.0.
        debt_service = debt_service + (cash + extra + fee);
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
    // E4: el objetivo del plan ES el clásico de 4.15.0 — una sola base, sin restar la pensión
    // con fecha. Se llama a la función del núcleo directamente para que no haya dos caminos.
    let fire_reached = fire_target_at_index_g(ft_view, 0).is_some_and(|t| liquid_month_zero >= t);
    let in_retirement = (fire_reached && !plan.crossing_is_reading_only)
        || plan
            .retirement_trigger
            .forced_month()
            .is_some_and(|s| 1 >= s);
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
    // El objetivo FIRE clásico (E4): UNA base, la de 4.15.0, evaluada mes a mes con la misma
    // función del núcleo que usan `fire_target_at_month_index` y `PlanFireTarget`. WP3 construía
    // aquí un evaluador «consciente del plan» que tabulaba `O(P)` gross-ups para el puente; el
    // modelo v2 retiró el puente como base del objetivo (M4/C2) y con él la tabla.
    let ft_view = input.fire_target.as_ref().map(|f| f.view());
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
    // **Necesidad NO cubierta del mes** (fix F de la revisión adversarial): el incremento del
    // descubierto, mes a mes. Existe porque las lecturas de cobertura de Monte Carlo sumaban
    // `withdrawal + shortfall` como «necesidad» y ese denominador ignora justo lo que el hogar
    // no pudo vender: con `fixed_real` el recorte es CERO por construcción y el cociente salía
    // 1,0 («la regla cubrió el 100 %») en caminos que cubrían el 8,8 %.
    let mut unmet_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    unmet_series.push(M::zero());
    // **Necesidad ORDINARIA** del mes (E1): gasto + retirada extra − ingresos, sin deuda ni
    // «Próximos». Es lo que juzgan la puerta de tasa inicial y la regla por saldo, y el crate
    // estocástico la necesita mes a mes; no se refleja en `ProjectionOutput` porque la API no
    // publica esta serie.
    let mut ordinary_need_series: Vec<M> = Vec::with_capacity(input.horizon_months as usize + 1);
    ordinary_need_series.push(M::zero());
    // **El veredicto de ESTE camino** (E1): latches monótonos, fijados la primera vez que uno de
    // los tres motivos se cumple. `None` = el camino aguanta hasta el horizonte.
    let mut failure_month_index: Option<u32> = None;
    let mut failure_kind: Option<crate::phases::PathFailure> = None;
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
        .map(|a| {
            a.purchase_price
                .filter(|p| *p > M::zero())
                .unwrap_or(M::zero())
        })
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
    // **Agotamiento en dos pasos** (revisión adversarial, hallazgo #2). El candidato es el primer
    // mes que deja la cartera vendible a cero; se PUBLICA solo si desde ese mes en adelante alguna
    // venta se quedó sin fundar. Un aterrizaje exacto cuyo gasto posterior está cubierto —el
    // puente que se vacía justo cuando entra la pensión— vacía la cartera y no agota nada: sale
    // `None`, y el contrato («desde el siguiente mes el descubierto se acumula») vuelve a ser
    // cierto para todo lo que se publica.
    let mut assets_depleted_month_index: Option<u32> = None;
    let mut depletion_confirmed = false;
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
            // Agrupación LITERAL de 4.15.0 (`+=` sobre `cash + extra + fee`), no tres sumas
            // sueltas: ver la nota gemela en `first_month_allocation_core`.
            debt_service = debt_service + (cash + extra + fee);
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
        let target_prev = fire_target_at_index_g(ft_view, k - 1);
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
            || plan
                .retirement_trigger
                .forced_month()
                .map_or(false, |s| k >= s);
        // El PRIMER mes jubilado, `R`: el ancla de las reglas de retirada y el único mes en que
        // se evalúa la puerta de tasa inicial (E1/C1 — el SWR es una tasa INICIAL, no una
        // comprobación mensual sobre el saldo vivo).
        //
        // Hasta E1 aquí se emitía `RetireAtAgeUnderfunded` comparando `L(R−1)` con el objetivo
        // del mes. Ese aviso murió con el objetivo como criterio: «me jubilo por edad y no
        // llego» es hoy `1 − éxito(R)`, una probabilidad sobre miles de caminos.
        let mut is_first_retired_month = false;
        if retired && retirement_month_index.is_none() {
            retirement_month_index = Some(k);
            is_first_retired_month = true;
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

        // **NECESIDAD ORDINARIA del mes** (E1, C1): el gasto de vivir que el patrimonio tiene que
        // fundar. `gasto + retirada extra − ingresos`, con la pensión con fecha y las rentas
        // persistentes ya dentro de `income`.
        //
        // Lo que NO entra, y es la mitad de la definición: el **servicio de deuda** (una cuota se
        // extingue; capitalizarla al SWR pide capital para un gasto que se acaba) y el
        // **`planning_adj`** de «Próximos» (un ingreso o un gasto puntual no describe el tren de
        // vida). Es la magnitud que juzgan la puerta de tasa inicial (F2) y la regla por saldo
        // (F3) — nunca `need_assets_net`, que es un déficit de CAJA y sí lleva las dos cosas.
        let ordinary_need = (expense + retirement_withdrawal - income).max(M::zero());

        // **PUERTA DE TASA INICIAL** (F2, C1/C2): se evalúa UNA vez, en `R`, contra el líquido
        // con el que el hogar entra en la jubilación. Sin puerta configurada
        // (`initial_rate: None`, el default de los dos constructores) no se ejecuta ni una
        // comparación: los pines de 4.15.0 no pueden moverse.
        //
        // `12 · monthly_allowance(tope, L(R−1))` ES `tope/100 · L(R−1)`, por la MISMA función que
        // topa las reglas por saldo (una sola escritura de la fórmula). El `<` lo decide el TIPO
        // (`MoneyOps::strictly_below`), como el resto de booleanos publicados del bucle: es un
        // veredicto colgando de una comparación entre dos cantidades que `Decimal` y `f64`
        // calculan por caminos distintos.
        let initial_rate_exceeded = is_first_retired_month
            && plan.initial_rate.is_some_and(|gate| {
                let cap_pct = gate.cap_pct_at(k, plan.pension);
                let cap_annual =
                    M::from_u32(12) * crate::withdrawal::monthly_allowance(cap_pct, liquid_prev);
                let need_full_annual = M::from_u32(12) * ordinary_need;
                M::strictly_below(cap_annual, need_full_annual)
            });

        let net_cash_month = income - expense - debt_service + planning_adj - retirement_withdrawal;

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
        // **El gasto de la regla se paga primero con la caja del mes** (`rule_is_spend`; fix D de
        // la revisión adversarial). Se decide AQUÍ, antes de la cascada, porque lo que hay que
        // evitar es comprar y vender el MISMO fondo el mismo mes: reinvertir el sobrante para
        // acto seguido venderlo realiza plusvalía por nada — 3.991,72 €/año de impuesto frente a
        // 373,11 € del hecho económico en el hogar medido por la revisión (×10,7).
        //
        // El techo de la regla es BRUTO, así que el presupuesto se compara en NETO. La `g` se lee
        // sobre la cartera ANTES de la aportación, que es justo la cartera de la que se vendería.
        let spend_from_cash = match (plan.spend_mode, allowed_gross) {
            (SpendMode::RuleIsSpend, Some(a)) if a > M::zero() && net_cash_month > M::zero() => {
                let (gains_now, uniform_now) =
                    month_gains_g(&values, &basis, &basis_declared, input.taxable_gain_ratio);
                let spend_net = net_of_gross_g(
                    a,
                    &values,
                    &gains_now,
                    uniform_now,
                    &liquid,
                    &rates,
                    &input.tax_brackets,
                    input.taxes_enabled,
                );
                net_cash_month.min(spend_net).max(M::zero())
            }
            _ => M::zero(),
        };
        let investable = net_cash_month - spend_from_cash;

        if investable > M::zero() {
            // **Techo de aportación** (§B.7): la cascada solo ve `min(sobrante, c)`; el resto es
            // caja DISPONIBLE. Sin techo el pool es el sobrante entero y no se ejecuta ni una
            // operación de más: bit-identidad.
            let pool = match plan.contribution_cap_at(k) {
                Some(cap) => {
                    let invested = investable.min(cap);
                    let disposable = investable - invested;
                    if disposable > M::zero() {
                        disposable_series[k as usize] = disposable;
                        disposable_total = disposable_total + disposable;
                    }
                    invested
                }
                None => investable,
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
            spend_from_cash,
        );
        if sale.depleted_portfolio && assets_depleted_month_index.is_none() {
            assets_depleted_month_index = Some(k);
        }
        // La confirmación mira el mes del candidato y todos los siguientes. `unfunded_sale` lo
        // publica el paseo (`und_gross > 0` / `!cap_exhausted` / `net_shortfall > 0`): una resta
        // de netos no serviría, porque el redondeo la deja en ±1e-24 sin que falte un euro.
        if sale.unfunded_sale && assets_depleted_month_index.is_some() {
            depletion_confirmed = true;
        }
        // Solo el descubierto RESTA patrimonio (D22/D24): el recorte y el sobrante de la regla
        // son lecturas. `None` = no hubo venta este mes y el acumulador NO se toca.
        if let Some(undrained_month) = sale.undrained {
            undrained_cumulative = undrained_cumulative + undrained_month;
        }
        // El descubierto PUBLICABLE del mes (clampado, ver el `push` de abajo): es también el
        // operando de F1, y se calcula una sola vez.
        let unmet_month = sale.undrained.unwrap_or_else(M::zero).max(M::zero());

        // -------------------------------------------------------------------------------------
        // **EL VEREDICTO DE ESTE CAMINO** (E1, M3 + C1, C10). Tres motivos, prioridad F1 > F2 > F3
        // dentro del mismo mes, y un latch monótono que se fija la PRIMERA vez.
        //
        // Solo se evalúa en meses de jubilación o de media jornada, y durante la media jornada
        // solo F1 (supuesto S1): las reglas de retirada se anclan en `L(R−1)`, que todavía no
        // existe, y la tasa inicial es una propiedad de la fecha de jubilación. Un mes ACUMULANDO
        // con déficit puede vaciar la cartera —y `assets_depleted_month_index` lo marca—, pero
        // eso no es un plan de jubilación que falla: es un hogar que gasta más de lo que gana
        // hoy, y su remedio es otro.
        //
        // **F2 y F3 se deciden solo en `R`** (C10): los dos son propiedades de la FECHA. Después
        // manda F1 mes a mes, y el recorte de la regla viaja como lectura informativa
        // (`withdrawal_shortfall`), nunca como fracaso.
        // -------------------------------------------------------------------------------------
        if failure_month_index.is_none() && matches!(phase, Phase::Retired | Phase::Partial) {
            // **F1 — la cartera no pudo cubrir la necesidad.** DOS operandos, y ninguno sobra:
            // `unfunded_sale` lo publica el paseo (`und_gross > 0` / `!cap_exhausted`), que es
            // quien sabe si la venta se quedó corta, y `unmet_month` acota el fallo a la
            // NECESIDAD (bajo `rule_is_spend` una venta discrecional sin fundar no es hambre).
            //
            // Sin el primero, el `unmet > 0` literal marca como fallido cualquier camino con la
            // cola de redondeo de `after_tax(gross_up(n))`. Medido sobre la batería: 6 de los 25
            // casos arrastran una cola de **1e-25 €**, y en 3 de ellos cae en un mes jubilado —
            // `P7` (mes 2), `P18` (mes 155) y `P21` (mes 122)—, hogares que jamás se acercan a
            // quedarse sin cartera. Un camino marcado así hunde el éxito del plan a cero sin que
            // falte un céntimo.
            let f1 = sale.unfunded_sale && M::strictly_below(M::zero(), unmet_month);
            // **F3 — la regla por saldo permite menos que el gasto ordinario.** Solo con techo
            // (`fixed_real` devuelve `None`: el permitido ES la necesidad) y **solo en `R`, el
            // primer mes jubilado** (C10), la MISMA marca que usa la puerta F2 de arriba.
            //
            // Mes a mes era un problema de BARRERA, no una medición del plan: con una regla por
            // saldo el permitido sigue al líquido, que en Monte Carlo pasea con ~17 % de
            // volatilidad frente a una deriva de ~0,8 %/año, así que sobre 840 meses la
            // probabilidad de tocar la barrera alguna vez tiende a 1 por la varianza y no por la
            // salud del plan. Medido sobre la demo sintética con «3,5 % del saldo» +
            // `rule_is_spend`: el capital necesario hoy salía **2,52 M€** (620 k€ con
            // `fixed_real`), los 67 fallos de 2.500 caminos eran TODOS F3 y el primero caía
            // siempre antes de la pensión (mediana: mes 293). Con F3 solo en `R` el mismo hogar
            // pide 860 k€, y F2 y F3 pasan a ser la misma pregunta en el mes de jubilación —
            // bruta la de F2 (`12·necesidad ≤ SWR·L(R−1)`), neta la de F3
            // (`after_tax(regla(L(R−1))) ≥ necesidad`)—. Después de `R` manda F1 mes a mes.
            //
            // Retirar F3 del todo NO era la alternativa: colapsaría al suelo de F2 y el modo
            // porcentual sería infalible por construcción (una fracción del saldo nunca lo vacía,
            // así que F1 tampoco puede firmar).
            //
            // `is_first_retired_month` implica `Phase::Retired`, así que no hace falta
            // comprobarlo aparte: la fase parcial no pasa por la regla (S1).
            let f3 = is_first_retired_month
                && sale
                    .rule_allowance_net
                    .is_some_and(|allowed| M::strictly_below(allowed, ordinary_need));
            let kind = if f1 {
                Some(crate::phases::PathFailure::PortfolioDepleted)
            } else if initial_rate_exceeded {
                Some(crate::phases::PathFailure::InitialRateExceeded)
            } else if f3 {
                Some(crate::phases::PathFailure::RuleBelowNeed)
            } else {
                None
            };
            if kind.is_some() {
                failure_month_index = Some(k);
                failure_kind = kind;
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
        // El descubierto del mes, clampado a 0 al PUBLICAR: el operando literal de 4.15.0 puede
        // salir ±1e-24 por cola de redondeo y una serie publicada no lleva números negativos
        // que nadie puede explicar. El acumulador (`uncovered_deficit_total`) conserva el
        // operando sin tocar — ahí manda la bit-identidad.
        unmet_series.push(unmet_month);
        ordinary_need_series.push(ordinary_need);
        let liquid_close = liquid_fn(&values);
        // §B.3: ¿la media jornada deja crecer el capital? Se compara el cierre del mes con el
        // cierre del anterior —el mismo par que el cruce usa— y basta UN mes a la baja.
        //
        // El `<` lo decide el TIPO (`MoneyOps::strictly_below`), como el `==` de las `g`: es un
        // booleano PUBLICADO —y con aviso propio— colgando de una comparación entre dos cierres
        // que en una serie plana coinciden hasta el ulp.
        if matches!(phase, Phase::Partial)
            && M::strictly_below(liquid_close, liquid_series[(k - 1) as usize])
        {
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

    // E4 retiró las tres lecturas del puente y de la media jornada que se derivaban del objetivo
    // (`bridge_effective_withdrawal_pct`, `pension_coverage_ratio`, `partial_gap_target`): las
    // tres capitalizaban una necesidad al SWR para decir algo sobre una FASE, y la fase ya no se
    // juzga contra un objetivo determinista sino contra el umbral de éxito.

    Ok(SimOutput {
        net_worth: net_series,
        contributed_capital: contrib_series,
        per_asset_series,
        assets_depleted_month_index: assets_depleted_month_index.filter(|_| depletion_confirmed),
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
        unmet_need: unmet_series,
        ordinary_need: ordinary_need_series,
        pension_start_month_index,
        partial_retirement_month_index: partial_month_index,
        warnings,
        partial_phase_capital_growing,
        disposable_cash: disposable_series,
        disposable_cash_total: disposable_total,
        failure_month_index,
        failure_kind,
    })
}
