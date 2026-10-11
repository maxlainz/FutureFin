//! **La tasa de retirada como porcentaje del líquido** — lo único que queda de las reglas de
//! retirada de 5.0.0-dev.
//!
//! WP2 de 5.0.0 había puesto aquí cuatro reglas (`fixed_real`, `percent_of_balance`, `hybrid`,
//! `guardrails`) × dos modos de gasto (`ceiling`, `rule_is_spend`), con un planificador con
//! memoria (latch del híbrido, multiplicador de Guyton-Klinger). **Se retiraron antes de publicar
//! 5.0.0** (decisión «solo SWR» del owner, 2026-10-10): jubilado, el hogar vende cada mes lo que
//! su gasto indexado necesita —el drenaje de 4.15.0, la antigua `fixed_real`— y el SWR es solo
//! la puerta de tasa INICIAL del mes `R` ([`crate::InitialRateGate`]). Lo medido que llevó a
//! retirarlas está en `futurefin-failure-archaeology`.
//!
//! Lo que sobrevive es la conversión de un porcentaje ANUAL del líquido en un importe mensual,
//! que la puerta F2 usa con el SWR (o el tope del puente) y `L(R−1)`.

use crate::money::MoneyOps;

/// `pct/100 · balance / 12` — el permitido MENSUAL bruto de una regla de porcentaje.
///
/// Se calcula como `balance·pct/1200` (una sola división, el mínimo redondeo posible). Si el
/// producto no cabe en el rango de `Decimal` (~7,9e28) se reordena a `(balance/1200)·pct`, que
/// no puede desbordar para ningún `pct` sensato: misma disciplina que el `checked_mul` de la
/// base de coste (#209) y que los `checked_div` de #208 — la forma reordenada SOLO se ejecuta
/// donde la directa no cabe, así que ninguna entrada que hoy funciona cambia de valor.
///
/// Balance ≤ 0 (cartera vacía o en negativo) ⇒ permitido 0: un porcentaje de nada es nada, y
/// nunca una retirada negativa.
///
/// La puerta de tasa inicial (F2) compara la necesidad ORDINARIA anual del mes de jubilación
/// contra `12 · monthly_allowance(tope, L(R−1))`, que es `tope/100 · L(R−1)`. Hasta el
/// 2026-10-10 esta misma función topaba además las reglas por saldo; la fórmula, el redondeo y
/// la reordenación anti-desbordamiento no cambiaron al retirarlas.
pub(crate) fn monthly_allowance<M: MoneyOps>(pct: M, balance: M) -> M {
    if balance <= M::zero() || pct <= M::zero() {
        return M::zero();
    }
    let twelve_hundred = M::from_u32(1_200);
    match balance.checked_mul(pct) {
        Some(p) => p / twelve_hundred,
        None => (balance / twelve_hundred) * pct,
    }
}

#[cfg(test)]
mod tests {
    use super::monthly_allowance;
    use rust_decimal::Decimal;

    #[test]
    fn the_allowance_is_the_annual_percentage_of_the_balance_over_twelve() {
        // 3,5 % de 600.000 € = 21.000 €/año = 1.750 €/mes.
        assert_eq!(
            monthly_allowance(Decimal::new(35, 1), Decimal::from(600_000)),
            Decimal::from(1_750)
        );
    }

    #[test]
    fn nothing_is_allowed_from_an_empty_balance_or_a_non_positive_rate() {
        assert_eq!(monthly_allowance(Decimal::from(4), Decimal::ZERO), Decimal::ZERO);
        assert_eq!(monthly_allowance(Decimal::from(4), Decimal::from(-10)), Decimal::ZERO);
        assert_eq!(monthly_allowance(Decimal::ZERO, Decimal::from(100)), Decimal::ZERO);
    }

    #[test]
    fn a_product_out_of_range_is_reordered_instead_of_overflowing() {
        let huge = Decimal::MAX / Decimal::from(2);
        let got = monthly_allowance(Decimal::from(4), huge);
        assert!(got > Decimal::ZERO);
    }
}
