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

/// Un tramo del drenaje mixto (#178): capacidad vendible MENSUAL (≥ 0, ya clampada por el
/// caller) y su fracción de plusvalía gravable `g` ∈ [0,1], en el ORDEN de drenaje.
#[derive(Debug, Clone, Copy)]
pub struct MixedSegment {
    pub capacity_monthly: Decimal,
    pub gain_ratio: Decimal,
}

/// Resultado del solver mixto (#178). Todo MENSUAL, como el par `gross_up_monthly` /
/// `after_tax_monthly` al que sustituye en la rama de déficit.
#[derive(Debug, Clone)]
pub struct MixedDrawdown {
    /// Venta bruta total del mes.
    pub gross_monthly: Decimal,
    /// Paralelo a `segments`: bruto vendido de cada tramo.
    pub per_segment_monthly: Vec<Decimal>,
    /// Neto que las capacidades no llegaron a cubrir (descubierto del mes, en euros de GASTO).
    pub net_shortfall_monthly: Decimal,
}

/// Gross-up EXACTO con `g` POR TRAMO (#178): el bruto mínimo que, vendido secuencialmente sobre
/// `segments`, netea `net_monthly` tras el impuesto por tramos progresivos sobre la base
/// agregada `Σ g_i·venta_i`.
///
/// Es la generalización de `gross_up_net_annual_fire` a `g` heterogénea: el neto
/// `F(G) = G − tax(B(G))` es lineal a trozos (pendiente `1 − r·g_j` mientras se vacía el tramo
/// `j` bajo el tipo `r`), y se invierte con un paseo sobre sus puntos de quiebro — fronteras de
/// capacidad (cambia `g`) y techos de tramo fiscal (cambia `r`) — resolviendo una ecuación afín
/// por trozo con su test de validez, la misma disciplina de la forma cerrada escalar. Sin
/// búsqueda binaria ni tolerancias (la familia iterada está retirada por arqueología: el punto
/// fijo escalar converge a razón ~0,11 — nueve iteraciones para 1e-6 € — y puede oscilar en las
/// fronteras de activo).
///
/// Convención M1, coherente con `gross_up_monthly`: se trabaja en unidades ANUALES
/// (`12·net`, capacidades `12·cap` — la ficción de M1 es «este mes repetido doce veces», así que
/// la capacidad anual del activo es `12·v`) y se divide por 12 al devolver.
///
/// Casos: `taxes_enabled = false` ⇒ llenado secuencial sin impuesto (identidad, como el resto
/// del módulo). `g = 0` (activo todo coste) ⇒ vende a euro por euro sin tributar. `den ≤ 0`
/// (tipo efectivo ≥ 100 % sobre este tramo) ⇒ el RESTO de ese tramo no puede netear nada y se
/// salta al siguiente — nunca se vende lo que netea ≤ 0 (un tramo posterior con `g` menor aún
/// puede netear; con `g` uniforme el caller ni llega aquí: cortocircuita al camino escalar).
///
/// **El caller decide el cortocircuito**: con `g` uniforme este paseo es ALGEBRAICAMENTE la
/// forma cerrada escalar, pero no bit-idéntico (trocear un tramo lineal añade divisiones que
/// `rust_decimal` redondea a 28 dígitos) — la rama de déficit y el runway usan el camino
/// literal de 4.11.0 cuando todos los `g` coinciden, y este solver solo cuando de verdad hay
/// mezcla.
pub fn gross_up_mixed_monthly(
    net_monthly: Decimal,
    segments: &[MixedSegment],
    brackets: &[TaxBracket],
    taxes_enabled: bool,
) -> MixedDrawdown {
    let twelve = Decimal::from(12u32);
    let mut out = MixedDrawdown {
        gross_monthly: Decimal::ZERO,
        per_segment_monthly: vec![Decimal::ZERO; segments.len()],
        net_shortfall_monthly: Decimal::ZERO,
    };
    if net_monthly <= Decimal::ZERO {
        return out;
    }
    let n_annual = net_monthly * twelve;

    if !taxes_enabled {
        // Llenado secuencial puro: cada euro bruto netea un euro.
        let mut remaining = n_annual;
        for (j, s) in segments.iter().enumerate() {
            if remaining <= Decimal::ZERO {
                break;
            }
            let take = (s.capacity_monthly * twelve).max(Decimal::ZERO).min(remaining);
            out.per_segment_monthly[j] = take / twelve;
            out.gross_monthly += take / twelve;
            remaining -= take;
        }
        out.net_shortfall_monthly = remaining.max(Decimal::ZERO) / twelve;
        return out;
    }

    let hundred = Decimal::from(100u32);
    let mut base = Decimal::ZERO; // base imponible ANUAL acumulada
    let mut net_acc = Decimal::ZERO;
    let mut gross_annual = Decimal::ZERO;
    let mut m = 0usize; // índice de tramo fiscal — monótono, nunca retrocede

    for (j, s) in segments.iter().enumerate() {
        let mut cap = (s.capacity_monthly * twelve).max(Decimal::ZERO);
        let g = s.gain_ratio.clamp(Decimal::ZERO, Decimal::ONE);
        let mut taken_annual = Decimal::ZERO;
        while cap > Decimal::ZERO && net_acc < n_annual && m < brackets.len() {
            let r = brackets[m].pct / hundred;
            let den = Decimal::ONE - r * g;
            if den <= Decimal::ZERO {
                // Tipo efectivo ≥ 100 % para ESTE g: nada de lo que quede en este tramo netea.
                break;
            }
            // Candidato que completa el neto en este trozo…
            let mut x = (n_annual - net_acc) / den;
            // …topado por el techo del tramo fiscal (la base llena el tramo)…
            if g > Decimal::ZERO {
                if let Some(ceiling) = brackets[m].up_to {
                    x = x.min((ceiling - base) / g);
                }
            }
            // …y por la capacidad del activo.
            x = x.min(cap);
            if x <= Decimal::ZERO {
                break;
            }
            net_acc += x * den;
            base += x * g;
            gross_annual += x;
            cap -= x;
            taken_annual += x;
            if let Some(ceiling) = brackets[m].up_to {
                if base >= ceiling {
                    m += 1;
                }
            }
        }
        out.per_segment_monthly[j] = taken_annual / twelve;
    }
    out.gross_monthly = gross_annual / twelve;
    out.net_shortfall_monthly = (n_annual - net_acc).max(Decimal::ZERO) / twelve;
    out
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

    // ── Solver mixto (#178) — números del spike, verificados por TRES fuentes (spike Opus +
    // réplica independiente en sesión + estos tests). Tramos ES, todo sintético. ──

    fn seg(v: u32, g: &str) -> MixedSegment {
        MixedSegment {
            capacity_monthly: Decimal::from(v),
            gain_ratio: g.parse().unwrap(),
        }
    }

    /// Caso 5a: A(10.000, g=0,2) + B(5.000, g=0,8), neto 1.000 €/mes. Cabe entero en el primer
    /// activo dentro del primer tramo: bruto = 1.000/0,962 y B ni se toca.
    #[test]
    fn mixed_two_assets_resolves_in_the_first_segment() {
        let br = es_brackets_for_tests();
        let dd = gross_up_mixed_monthly(
            Decimal::from(1000u32),
            &[seg(10_000, "0.2"), seg(5_000, "0.8")],
            &br,
            true,
        );
        assert_eq!(dd.gross_monthly.round_dp(10), "1039.5010395010".parse::<Decimal>().unwrap());
        assert_eq!(dd.per_segment_monthly[0], dd.gross_monthly);
        assert_eq!(dd.per_segment_monthly[1], Decimal::ZERO);
        assert_eq!(dd.net_shortfall_monthly, Decimal::ZERO);
        // Partida doble: G − tax(base) == neto, con la base agregada real.
        let base_annual = dd.gross_monthly * Decimal::from(12u32) * "0.2".parse::<Decimal>().unwrap();
        let netea = dd.gross_monthly * Decimal::from(12u32)
            - tax_on_gross_capital_annual(base_annual, &br);
        assert!((netea - Decimal::from(12_000u32)).abs() < Decimal::new(1, 8), "{netea}");
    }

    /// Caso 5a-bis: la capacidad muerde — A(1.000, g=0,2) se agota dentro del mes y el resto
    /// sale de B(5.000, g=0,8) a pendiente 0,848. Ningún escalar reproduce este número.
    #[test]
    fn mixed_capacity_boundary_switches_slope_mid_month() {
        let br = es_brackets_for_tests();
        let dd = gross_up_mixed_monthly(
            Decimal::from(1000u32),
            &[seg(1_000, "0.2"), seg(5_000, "0.8")],
            &br,
            true,
        );
        assert_eq!(dd.gross_monthly.round_dp(10), "1044.8113207547".parse::<Decimal>().unwrap());
        assert_eq!(dd.per_segment_monthly[0], Decimal::from(1000u32));
        assert_eq!(
            dd.per_segment_monthly[1].round_dp(10),
            "44.8113207547".parse::<Decimal>().unwrap()
        );
        assert_eq!(dd.net_shortfall_monthly, Decimal::ZERO);
    }

    /// Con `g` uniforme el paseo es ALGEBRAICAMENTE la forma cerrada escalar: mismo valor a
    /// plena precisión. (La bit-identidad de producción la garantiza el cortocircuito del
    /// caller, que ni llama aquí — este test fija la igualdad de VALOR del solver.)
    #[test]
    fn mixed_with_uniform_g_equals_the_scalar_closed_form() {
        let br = es_brackets_for_tests();
        for g in ["1", "0.5", "0.2"] {
            let scalar = gross_up_monthly(
                Decimal::from(2000u32),
                &br,
                true,
                g.parse().unwrap(),
            );
            let dd = gross_up_mixed_monthly(
                Decimal::from(2000u32),
                &[seg(1_000_000, g), seg(1_000_000, g)],
                &br,
                true,
            );
            assert!(
                (dd.gross_monthly - scalar).abs() < Decimal::new(1, 12),
                "g={g}: paseo {} vs escalar {scalar}",
                dd.gross_monthly
            );
        }
    }

    /// `g = 0` (activo todo coste): vende a euro por euro, sin impuesto — y sin tope de tramo,
    /// porque la base no crece.
    #[test]
    fn mixed_all_cost_asset_sells_euro_for_euro() {
        let br = es_brackets_for_tests();
        let dd = gross_up_mixed_monthly(Decimal::from(9000u32), &[seg(20_000, "0")], &br, true);
        assert_eq!(dd.gross_monthly, Decimal::from(9000u32));
        assert_eq!(dd.net_shortfall_monthly, Decimal::ZERO);
    }

    /// Capacidad insuficiente: se vende TODO y el descubierto sale NETO por construcción —
    /// 100 € brutos al 19 % (g=1) netean 81; faltan 919 del gasto.
    #[test]
    fn mixed_shortfall_is_net_by_construction() {
        let br = es_brackets_for_tests();
        let dd = gross_up_mixed_monthly(Decimal::from(1000u32), &[seg(100, "1")], &br, true);
        assert_eq!(dd.gross_monthly, Decimal::from(100u32));
        assert_eq!(dd.net_shortfall_monthly.round_dp(10), Decimal::from(919u32).round_dp(10));
    }

    /// `taxes_enabled = false`: llenado secuencial puro, identidad — la misma regla que el
    /// resto del módulo.
    #[test]
    fn mixed_with_taxes_off_is_the_identity_fill() {
        let br = es_brackets_for_tests();
        let dd = gross_up_mixed_monthly(
            Decimal::from(1500u32),
            &[seg(1_000, "0.9"), seg(10_000, "0.1")],
            &br,
            false,
        );
        assert_eq!(dd.gross_monthly, Decimal::from(1500u32));
        assert_eq!(dd.per_segment_monthly[0], Decimal::from(1000u32));
        assert_eq!(dd.per_segment_monthly[1], Decimal::from(500u32));
        assert_eq!(dd.net_shortfall_monthly, Decimal::ZERO);
    }
}
