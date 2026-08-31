//! Fiscalidad del ahorro: la escala por tramos y su inverso (gross-up), EN el motor desde la
//! Ola 6 (#140): la retirada simulada y el objetivo FIRE deben grossearse con LA MISMA función,
//! y eso solo es posible si vive donde los dos pueden llamarla. Hasta 4.9.0 el gross-up era del
//! handler (`apps/api/src/handlers/projection.rs`) y `tax_on_gross_capital_annual` era
//! `#[cfg(test)]`; la mudanza es bit-idéntica (los tests de la forma cerrada viajaron con ella,
//! oráculo de bisección incluido).
//!
//! El espejo TS (preview del formulario) es `apps/web/src/lib/fire.ts`
//! (`taxOnGrossCapitalAnnual` / `grossUpNetAnnualFire`), atado por `fire-parity.json`.

use rust_decimal::Decimal;

/// Tramo de la escala del ahorro. `up_to = None` = tramo abierto (el último por contrato:
/// `validate_tax_brackets` en la API lo exige). La serde (Decimal-as-string) es EXACTAMENTE la
/// histórica del tipo cuando vivía en `installation.rs` — el JSONB almacenado de `fire_settings`
/// deserializa idéntico.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaxBracket {
    #[serde(with = "rust_decimal::serde::str_option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub up_to: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub pct: Decimal,
}

/// Impuesto sobre una base bruta anual, escala MARGINAL por tramos. Producción desde la Ola 6
/// (el `undrained` neto y el after-tax del drain la necesitan); antes era un helper de test.
pub fn tax_on_gross_capital_annual(gross: Decimal, brackets: &[TaxBracket]) -> Decimal {
    if gross <= Decimal::ZERO || brackets.is_empty() {
        return Decimal::ZERO;
    }
    let mut prev_ceiling = Decimal::ZERO;
    let mut tax = Decimal::ZERO;
    for b in brackets {
        let r = b.pct / Decimal::from(100u32);
        match b.up_to {
            None => {
                let taxable = (gross - prev_ceiling).max(Decimal::ZERO);
                tax += taxable * r;
                break;
            }
            Some(ceiling) => {
                let slice_end = gross.min(ceiling);
                let taxable = (slice_end - prev_ceiling).max(Decimal::ZERO);
                tax += taxable * r;
                prev_ceiling = ceiling;
                if gross <= ceiling {
                    break;
                }
            }
        }
    }
    tax
}

/// Devuelve el `gross` tal que `gross − tax(gross) == net_annual`, sin búsqueda binaria.
///
/// La función `tax(·)` es lineal por tramos: dentro del tramo i con tipo `r_i` y umbral
/// inferior `prev_i`, `after(g) = g·(1 − r_i) + (r_i·prev_i − K_i)`, donde `K_i` es el impuesto
/// acumulado de los tramos anteriores. Despejando `g = (net − r_i·prev_i + K_i) / (1 − r_i)` se
/// obtiene un candidato; si cae dentro del tramo (≤ `ceiling_i`), es la solución; si no, se
/// avanza al siguiente y se actualiza `K_i`.
pub fn gross_up_net_annual_fire(net_annual: Decimal, brackets: &[TaxBracket], taxes_enabled: bool) -> Decimal {
    if !taxes_enabled || net_annual <= Decimal::ZERO {
        return net_annual.max(Decimal::ZERO);
    }
    let hundred = Decimal::from(100u32);
    let mut prev_ceiling = Decimal::ZERO;
    let mut k_cumulative = Decimal::ZERO;
    for b in brackets {
        let r = b.pct / hundred;
        let denom = Decimal::ONE - r;
        if denom <= Decimal::ZERO {
            // Tipo del 100% (o superior): imposible recuperar `net` positivo; degeneración.
            return prev_ceiling;
        }
        let gross = (net_annual + k_cumulative - r * prev_ceiling) / denom;
        match b.up_to {
            None => return gross,
            Some(ceiling) => {
                if gross <= ceiling {
                    return gross;
                }
                let width = ceiling - prev_ceiling;
                k_cumulative += r * width;
                prev_ceiling = ceiling;
            }
        }
    }
    // Inalcanzable: `validate_tax_brackets` exige que el último tramo tenga `up_to = None`.
    net_annual
}

#[cfg(test)]
mod tests {
    use super::*;

    fn es_brackets() -> Vec<TaxBracket> {
        vec![
            TaxBracket { up_to: Some(Decimal::from(6_000u32)),   pct: Decimal::from(19u32) },
            TaxBracket { up_to: Some(Decimal::from(50_000u32)),  pct: Decimal::from(21u32) },
            TaxBracket { up_to: Some(Decimal::from(200_000u32)), pct: Decimal::from(23u32) },
            TaxBracket { up_to: Some(Decimal::from(300_000u32)), pct: Decimal::from(27u32) },
            TaxBracket { up_to: None,                            pct: Decimal::from(30u32) },
        ]
    }

    /// Versión binaria de referencia (la que tenía el handler antes de Fase 2.4). Sirve para
    /// confirmar que la forma cerrada es numéricamente equivalente a ≤ 0.01 €.
    fn gross_up_binary_reference(net_annual: Decimal, brackets: &[TaxBracket]) -> Decimal {
        if net_annual <= Decimal::ZERO { return net_annual.max(Decimal::ZERO); }
        let mut lo = net_annual;
        let mut hi = (net_annual * Decimal::from(4u32))
            .max(net_annual + Decimal::from(200_000u32));
        for _ in 0..90 {
            let mid = (lo + hi) / Decimal::from(2u32);
            let after = mid - tax_on_gross_capital_annual(mid, brackets);
            if after < net_annual { lo = mid; } else { hi = mid; }
        }
        hi
    }

    #[test]
    fn closed_form_matches_binary_search_across_es_brackets() {
        let brackets = es_brackets();
        let nets = [
            Decimal::from(1_000u32),
            Decimal::from(5_000u32),
            Decimal::from(20_000u32),
            Decimal::from(40_000u32),
            Decimal::from(80_000u32),
            Decimal::from(150_000u32),
            Decimal::from(250_000u32),
            Decimal::from(400_000u32),
            Decimal::from(1_000_000u32),
        ];
        let tol = Decimal::new(1, 2); // 0.01 €
        for net in nets {
            let g_closed = gross_up_net_annual_fire(net, &brackets, true);
            let g_binary = gross_up_binary_reference(net, &brackets);
            let diff = (g_closed - g_binary).abs();
            assert!(
                diff <= tol,
                "diff {diff} excede tolerancia para net={net}: closed={g_closed}, binary={g_binary}"
            );
            // Y verifica que el gross resultante deja después-de-tax ≈ net.
            let after = g_closed - tax_on_gross_capital_annual(g_closed, &brackets);
            assert!(
                (after - net).abs() <= tol,
                "after-tax({g_closed}) = {after} no recupera net={net}"
            );
        }
    }

    #[test]
    fn closed_form_handles_taxes_disabled_and_zero_net() {
        let brackets = es_brackets();
        assert_eq!(gross_up_net_annual_fire(Decimal::from(50_000u32), &brackets, false), Decimal::from(50_000u32));
        assert_eq!(gross_up_net_annual_fire(Decimal::ZERO, &brackets, true), Decimal::ZERO);
        assert_eq!(
            gross_up_net_annual_fire(-Decimal::from(100u32), &brackets, true),
            Decimal::ZERO,
            "net negativo se clipea a 0"
        );
    }
}
