//! Runway de liquidez: cuántos meses aguantan los activos **líquidos** pagando el gasto mensual,
//! componiendo la rentabilidad esperada de esos activos y la inflación del gasto.

use rust_decimal::Decimal;

use crate::projection::monthly_multiplier;

/// Tope del bucle: 1.200 meses = 100 años. Sobrevivir el tope se reporta como `Indefinite`.
pub const MAX_RUNWAY_MONTHS: u32 = 1200;

/// Resultado del runway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunwayOutcome {
    /// Meses cubiertos, con el último mes **fraccionario** (p.ej. `10.5` = diez meses y medio).
    Months(Decimal),
    /// El saldo sobrevive `MAX_RUNWAY_MONTHS` meses: la rentabilidad cubre el gasto inflado.
    Indefinite,
    /// No hay base de gasto (`monthly_expense <= 0`): el runway no está definido.
    NoExpenseBase,
}

/// Meses de runway de los activos líquidos, con rentabilidad e inflación compuestas.
///
/// `liquid_assets` son pares `(valor, rentabilidad anual %)`; `monthly_expense` es el gasto
/// mensual total de partida y `annual_inflation_percent` la inflación anual que lo encarece.
///
/// # Modelo
///
/// - **Marco nominal**: los activos crecen a su rentabilidad *nominal* y el gasto se infla cada
///   mes. No se deflacta nada: el resultado (un número de meses) es invariante al marco, pero
///   mezclar retorno nominal con gasto constante sobreestimaría el runway.
/// - **Orden retirada-antes-de-crecimiento**: cada mes se paga el gasto y luego crece el saldo
///   restante — el mismo orden que el bucle de simulación de `projection.rs` (drenaje del cash
///   flow negativo antes de aplicar los multiplicadores), así que ambas curvas son coherentes.
/// - **Multiplicador ponderado**: se usa una media de los multiplicadores mensuales ponderada por
///   valor, `m = Σ vₐ·monthly_multiplier(rₐ) / Σ vₐ`. Equivale a un **drenaje prorrateado** (cada
///   activo aporta al gasto en proporción a su peso), ligeramente **conservador** frente al drain
///   real del engine, que vacía primero los líquidos de menor rentabilidad y por tanto conserva
///   más tiempo los de rentabilidad alta.
/// - **Tasas ≤ 0 → crecimiento 0**: herencia documentada de [`monthly_multiplier`], que devuelve
///   factor 1 tanto para `None` como para tasas no positivas (el engine no modela pérdidas).
/// - **Cap de 100 años**: si el saldo aguanta `MAX_RUNWAY_MONTHS` meses se devuelve `Indefinite`
///   (sin epsilon ni forma cerrada: `ln` sufre cancelación justo en la frontera `A·j → g`; el
///   bucle mes a mes la evita y cuesta microsegundos).
///
/// # Reducción exacta a `A / g`
///
/// Con rentabilidad e inflación 0 se tiene `m = m_inf = 1`, luego `balance_k = A − k·g` y `g`
/// constante. Sea `n = ⌊A/g⌋`: para todo `k ≤ n` se cumple `balance_{k−1} = A − (k−1)·g ≥ g`, así
/// que el bucle no corta; en `k = n+1` se cumple `balance_n = A − n·g < g` y se devuelve
/// `n + (A − n·g)/g = A/g`. La única división es la final, de modo que el resultado es la división
/// simple exacta (regresión sin tolerancias en los tests).
///
/// Casos límite: `monthly_expense <= 0` → [`RunwayOutcome::NoExpenseBase`]; saldo total ≤ 0 →
/// `Months(0)`.
pub fn liquid_runway_months(
    liquid_assets: &[(Decimal, Option<Decimal>)],
    monthly_expense: Decimal,
    annual_inflation_percent: Decimal,
) -> RunwayOutcome {
    if monthly_expense <= Decimal::ZERO {
        return RunwayOutcome::NoExpenseBase;
    }

    let balance_0: Decimal = liquid_assets.iter().map(|(v, _)| *v).sum();
    if balance_0 <= Decimal::ZERO {
        // Sin saldo no hay nada que cubrir. (El bucle daría lo mismo para 0, pero un saldo
        // negativo produciría meses negativos: se corta aquí.)
        return RunwayOutcome::Months(Decimal::ZERO);
    }

    // Media ponderada por valor de los multiplicadores mensuales (drenaje prorrateado).
    let weighted: Decimal = liquid_assets
        .iter()
        .map(|(v, r)| *v * monthly_multiplier(*r))
        .sum();
    let m = weighted / balance_0;
    let m_inf = monthly_multiplier(Some(annual_inflation_percent));

    let mut balance = balance_0;
    let mut g = monthly_expense;
    for k in 1..=MAX_RUNWAY_MONTHS {
        if balance < g {
            // Mes final fraccionario: la parte del mes k que el saldo aún cubre.
            return RunwayOutcome::Months(Decimal::from(k - 1) + balance / g);
        }
        balance = (balance - g) * m;
        g *= m_inf;
    }
    RunwayOutcome::Indefinite
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    fn months(o: RunwayOutcome) -> Decimal {
        match o {
            RunwayOutcome::Months(m) => m,
            other => panic!("expected Months, got {other:?}"),
        }
    }

    /// Réplica del bucle con un multiplicador ya calculado — solo para el test de ponderación.
    /// Sin inflación: `g` es constante (el test de ponderación la fija a 0).
    fn reference_loop(balance_0: Decimal, m: Decimal, g: Decimal) -> Decimal {
        let mut balance = balance_0;
        for k in 1..=MAX_RUNWAY_MONTHS {
            if balance < g {
                return Decimal::from(k - 1) + balance / g;
            }
            balance = (balance - g) * m;
        }
        panic!("no debería agotar el cap en este test");
    }

    /// Sin rentabilidad ni inflación el runway es la división simple.
    /// 12.000 / 1.000 = **12** exacto; 10.000 / 3.000 = **10000/3000** exacto (periódico).
    #[test]
    fn zero_return_zero_inflation_equals_plain_division() {
        let entero = liquid_runway_months(&[(d(12_000), None)], d(1_000), Decimal::ZERO);
        assert_eq!(entero, RunwayOutcome::Months(d(12)));

        let fraccionario = liquid_runway_months(&[(d(10_000), None)], d(3_000), Decimal::ZERO);
        assert_eq!(
            fraccionario,
            RunwayOutcome::Months(d(10_000) / d(3_000)),
            "el último mes fraccionario reconstruye A/g sin residuos"
        );
    }

    /// 12.000 al 5% anual con gasto 1.000 aguanta MÁS de los 12 meses de la división simple
    /// (el saldo remanente crece ~0,407%/mes mientras se consume).
    #[test]
    fn positive_return_extends_runway() {
        let con_retorno = months(liquid_runway_months(
            &[(d(12_000), Some(d(5)))],
            d(1_000),
            Decimal::ZERO,
        ));
        assert!(
            con_retorno > d(12),
            "esperado > 12 meses, obtenido {con_retorno}"
        );
    }

    /// Los mismos 12.000 con gasto 1.000 pero inflación 3% aguantan MENOS de 12 meses:
    /// el gasto del mes k es 1.000·m_inf^(k−1) > 1.000.
    #[test]
    fn inflation_shortens_runway() {
        let con_inflacion = months(liquid_runway_months(&[(d(12_000), None)], d(1_000), d(3)));
        assert!(
            con_inflacion < d(12),
            "esperado < 12 meses, obtenido {con_inflacion}"
        );
    }

    /// 1.000.000 al 7% anual rinde ~5.650 €/mes, muy por encima del gasto de 1.000 sin inflación:
    /// el saldo nunca baja → `Indefinite` al llegar al cap de 1.200 meses.
    #[test]
    fn return_covering_expense_is_indefinite() {
        let out = liquid_runway_months(&[(d(1_000_000), Some(d(7)))], d(1_000), Decimal::ZERO);
        assert_eq!(out, RunwayOutcome::Indefinite);
    }

    /// La media de multiplicadores se pondera por **valor**, no por número de activos:
    /// 150.000 al 0% + 50.000 al 10% ⇒ m = (150.000·1 + 50.000·m₁₀) / 200.000.
    /// El resultado debe coincidir con el bucle usando ese m, y diferir de la media simple
    /// (1 + m₁₀)/2, que sobrepondera el activo pequeño.
    #[test]
    fn weighted_rate_uses_value_weights() {
        let v_lento = d(150_000);
        let v_rapido = d(50_000);
        let m10 = monthly_multiplier(Some(d(10)));
        let balance_0 = v_lento + v_rapido;
        let m_ponderado = (v_lento * Decimal::ONE + v_rapido * m10) / balance_0;

        let out = liquid_runway_months(
            &[(v_lento, None), (v_rapido, Some(d(10)))],
            d(2_000),
            Decimal::ZERO,
        );
        assert_eq!(
            months(out),
            reference_loop(balance_0, m_ponderado, d(2_000))
        );

        let m_simple = (Decimal::ONE + m10) / d(2);
        assert_ne!(
            m_ponderado, m_simple,
            "la media simple debe diferir de la ponderada por valor"
        );
        assert_ne!(
            reference_loop(balance_0, m_ponderado, d(2_000)),
            reference_loop(balance_0, m_simple, d(2_000)),
            "y esa diferencia debe ser observable en los meses de runway"
        );
    }

    /// El engine no modela pérdidas: una rentabilidad negativa se trata como crecimiento 0, luego
    /// −5% da exactamente el mismo runway que «sin rentabilidad» (12 meses).
    #[test]
    fn negative_return_treated_as_zero_growth() {
        let negativo = liquid_runway_months(&[(d(12_000), Some(d(-5)))], d(1_000), Decimal::ZERO);
        let sin_tasa = liquid_runway_months(&[(d(12_000), None)], d(1_000), Decimal::ZERO);
        assert_eq!(negativo, sin_tasa);
        assert_eq!(negativo, RunwayOutcome::Months(d(12)));
    }

    /// Sin gasto el runway no está definido (ni siquiera «infinito»): `NoExpenseBase`.
    #[test]
    fn zero_expense_is_no_expense_base() {
        assert_eq!(
            liquid_runway_months(&[(d(12_000), Some(d(5)))], Decimal::ZERO, d(3)),
            RunwayOutcome::NoExpenseBase
        );
    }

    /// Sin activos líquidos (o con saldo 0) el runway es 0 meses, no `None`.
    #[test]
    fn zero_balance_is_zero_months() {
        assert_eq!(
            liquid_runway_months(&[], d(1_000), Decimal::ZERO),
            RunwayOutcome::Months(Decimal::ZERO)
        );
        assert_eq!(
            liquid_runway_months(&[(Decimal::ZERO, Some(d(5)))], d(1_000), Decimal::ZERO),
            RunwayOutcome::Months(Decimal::ZERO)
        );
    }
}
