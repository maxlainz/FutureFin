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

/// Mensualización M1 del gross-up (#140 fase 1): `gross_up(12·need)/12`, reevaluada cada mes.
/// Exacta cuando los 12 `need` del año coinciden (gasto plano); con `need` variable sobreestima
/// solo en los años cuyos valores anualizados cruzan un techo de tramo — medido: 1,88 € de bruto
/// en 30 años frente al acumulador anual óptimo, que costaría estado por año fiscal, sensibilidad
/// al mes de `ref_date` y un diente de sierra intraanual del +2,5 %. No se compra.
pub fn gross_up_monthly(
    net_monthly: Decimal,
    brackets: &[TaxBracket],
    taxes_enabled: bool,
    taxable_gain_ratio: Decimal,
) -> Decimal {
    if !taxes_enabled || net_monthly <= Decimal::ZERO {
        return net_monthly.max(Decimal::ZERO);
    }
    gross_up_net_annual_fire(
        net_monthly * Decimal::from(12u32),
        brackets,
        taxes_enabled,
        taxable_gain_ratio,
    ) / Decimal::from(12u32)
}

/// Inversa mensualizada: lo que NETEA una venta bruta mensual — `gross − tax(12·gross)/12`,
/// misma anualización M1 que `gross_up_monthly` (par redondo: `after_tax(gross_up(n)) = n`).
/// La necesita el `undrained` NETO: el descubierto se mide en euros que faltaron por GASTAR,
/// no en ventas que nunca ocurrieron.
pub fn after_tax_monthly(
    gross_monthly: Decimal,
    brackets: &[TaxBracket],
    taxes_enabled: bool,
    taxable_gain_ratio: Decimal,
) -> Decimal {
    if !taxes_enabled || gross_monthly <= Decimal::ZERO {
        return gross_monthly;
    }
    gross_monthly
        - tax_on_gross_capital_annual(
            gross_monthly * Decimal::from(12u32) * taxable_gain_ratio,
            brackets,
        ) / Decimal::from(12u32)
}

/// Devuelve el `gross` tal que `gross − tax(taxable_gain_ratio·gross) == net_annual`, sin
/// búsqueda binaria. `taxable_gain_ratio` (g, fracción [0,1]) es la parte de cada euro bruto
/// que es plusvalía gravable (#140 fase 2): `g = 1` = reembolso íntegro gravado (histórico),
/// `g = 0` = nada tributa (≡ `taxes_enabled = false`).
///
/// La función `tax(·)` es lineal por tramos: dentro del tramo i con tipo `r_i` y umbral
/// inferior `prev_i`, `after(g) = g·(1 − r_i) + (r_i·prev_i − K_i)`, donde `K_i` es el impuesto
/// acumulado de los tramos anteriores. Despejando `g = (net − r_i·prev_i + K_i) / (1 − r_i)` se
/// obtiene un candidato; si cae dentro del tramo (≤ `ceiling_i`), es la solución; si no, se
/// avanza al siguiente y se actualiza `K_i`.
pub fn gross_up_net_annual_fire(
    net_annual: Decimal,
    brackets: &[TaxBracket],
    taxes_enabled: bool,
    taxable_gain_ratio: Decimal,
) -> Decimal {
    if !taxes_enabled || net_annual <= Decimal::ZERO {
        return net_annual.max(Decimal::ZERO);
    }
    let hundred = Decimal::from(100u32);
    let g = taxable_gain_ratio;
    let mut prev_ceiling = Decimal::ZERO;
    let mut k_cumulative = Decimal::ZERO;
    for b in brackets {
        let r = b.pct / hundred;
        // Fase 2 (#140): la base imponible es `g·G` — se busca `G − tax(g·G) = net`, así que en
        // el tramo de tipo r la ecuación es `net = G·(1 − r·g) − K + r·prev` y el TEST DE
        // VALIDEZ cambia de forma con el denominador: hay que comparar `g·G ≤ techo`, no
        // `G ≤ techo` — escribir solo el denominador y dejar el test viejo es el bug silencioso
        // de esta fase (con net 250.000 y g 0,5 la base cae DOS tramos por debajo). Con
        // `g = ONE` todo colapsa término a término a la forma histórica: `r·ONE` y `ONE·gross`
        // son exactos en rust_decimal, así que la igualdad es de valor exacto, sin tolerancia.
        let denom = Decimal::ONE - r * g;
        if denom <= Decimal::ZERO {
            // Tipo efectivo ≥ 100 %: imposible netear. `prev_ceiling` TAL CUAL — no `prev/g`,
            // que con g = ONE cambiaría la escala del Decimal (mismo valor, otro to_string).
            return prev_ceiling;
        }
        let gross = (net_annual + k_cumulative - r * prev_ceiling) / denom;
        match b.up_to {
            None => return gross,
            Some(ceiling) => {
                if g * gross <= ceiling {
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

/// Escala ES por defecto, SOLO para tests del crate (la de producción vive en la API,
/// `default_es_tax_brackets`): evita que cada mod de tests duplique la lista.
#[cfg(test)]
pub(crate) fn es_brackets_for_tests() -> Vec<TaxBracket> {
    vec![
        TaxBracket { up_to: Some(Decimal::from(6_000u32)), pct: Decimal::from(19u32) },
        TaxBracket { up_to: Some(Decimal::from(50_000u32)), pct: Decimal::from(21u32) },
        TaxBracket { up_to: Some(Decimal::from(200_000u32)), pct: Decimal::from(23u32) },
        TaxBracket { up_to: Some(Decimal::from(300_000u32)), pct: Decimal::from(27u32) },
        TaxBracket { up_to: None, pct: Decimal::from(30u32) },
    ]
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
            let g_closed = gross_up_net_annual_fire(net, &brackets, true, Decimal::ONE);
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
        assert_eq!(
            gross_up_net_annual_fire(Decimal::from(50_000u32), &brackets, false, Decimal::ONE),
            Decimal::from(50_000u32)
        );
        assert_eq!(
            gross_up_net_annual_fire(Decimal::ZERO, &brackets, true, Decimal::ONE),
            Decimal::ZERO
        );
        assert_eq!(
            gross_up_net_annual_fire(-Decimal::from(100u32), &brackets, true, Decimal::ONE),
            Decimal::ZERO,
            "net negativo se clipea a 0"
        );
    }

    /// Referencia PRE-fase-2 (la forma cerrada sin `g`, copiada verbatim del estado 4.9.0):
    /// existe solo para afirmar la bit-identidad de `g = 1` — sobre la FUNCIÓN, no sobre la
    /// ola entera.
    fn gross_up_pre_fase2(net_annual: Decimal, brackets: &[TaxBracket], taxes_enabled: bool) -> Decimal {
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
        net_annual
    }

    /// #140 fase 2 — `g = 1` es BIT-idéntico a la forma sin `g`: `r·ONE` y `ONE·gross` son
    /// exactos en rust_decimal y no hay redondeo intermedio, así que la igualdad es
    /// `assert_eq!` de valor, sin tolerancia. Los netos 4.859/4.860/4.861 y
    /// 39.619/39.620/39.621 ponen el bruto EXACTAMENTE en los bordes de tramo 6.000 y 50.000,
    /// donde un `<=` mal puesto se ve.
    #[test]
    fn gross_up_with_g_one_is_bit_identical_to_the_ungrossed_form() {
        let escalas: [Vec<TaxBracket>; 3] = [
            es_brackets(),
            vec![TaxBracket { up_to: None, pct: Decimal::from(80u32) }],
            Vec::new(),
        ];
        let nets = [
            0i64, 1, 4_859, 4_860, 4_861, 24_000, 25_650, 39_619, 39_620, 39_621, 114_000,
            200_000, 250_000, 1_000_000,
        ];
        for brackets in &escalas {
            for enabled in [true, false] {
                for n in nets {
                    let net = Decimal::from(n);
                    assert_eq!(
                        gross_up_net_annual_fire(net, brackets, enabled, Decimal::ONE),
                        gross_up_pre_fase2(net, brackets, enabled),
                        "net={n}, enabled={enabled}, tramos={}",
                        brackets.len()
                    );
                }
            }
        }
    }

    /// `g = 0` ⇒ nada tributa: idéntico a `taxes_enabled = false` para todo net.
    #[test]
    fn g_zero_equals_taxes_disabled() {
        let brackets = es_brackets();
        for n in [1i64, 24_000, 250_000, 1_000_000] {
            let net = Decimal::from(n);
            assert_eq!(
                gross_up_net_annual_fire(net, &brackets, true, Decimal::ZERO),
                net,
                "g=0 debe devolver el neto tal cual"
            );
            assert_eq!(
                gross_up_net_annual_fire(net, &brackets, true, Decimal::ZERO),
                gross_up_net_annual_fire(net, &brackets, false, Decimal::ONE),
            );
        }
    }

    /// La trampa del techo (#140 fase 2): el test de validez es `g·G ≤ techo`, NO `G ≤ techo`.
    /// net 250.000 con g = 0,5: a g = 1 el bruto (331.257,14) caía en el tramo ABIERTO del
    /// 30 %; con g = 0,5 la base gravable (140.610,17) cae en el 23 % — DOS tramos por debajo:
    /// G = (250.000 + 10.380 − 0,23·50.000)/(1 − 0,115) = 248.880/0,885 = **281.220,3390**,
    /// y la comprobación redonda `G − tax(0,5·G) = 250.000` exacta. (El K del tramo es 10.380
    /// = 1.140 + 0,21·44.000 — un «11.640» que circuló en el spike era errata de nota, no de
    /// cifra.) Escribir solo el denominador y dejar el test viejo daría el tramo del 27/30 %.
    #[test]
    fn the_validity_test_uses_the_taxable_base_not_the_gross() {
        let brackets = es_brackets();
        let half = Decimal::new(5, 1);
        let g = gross_up_net_annual_fire(Decimal::from(250_000u32), &brackets, true, half);
        assert_eq!(g.round_dp(4), "281220.3390".parse::<Decimal>().unwrap());
        let neteado = g - tax_on_gross_capital_annual(half * g, &brackets);
        assert!(
            (neteado - Decimal::from(250_000u32)).abs() < Decimal::new(1, 6),
            "G − tax(g·G) debe recuperar el neto: {neteado}"
        );
    }
}
