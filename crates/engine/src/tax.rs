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
            //
            // **`checked_div`, no `/` (issue #208).** Los dos topes de abajo dividen por un
            // número que el motor MISMO fabrica y que solo está guardado por «> 0»; con un
            // divisor positivo pero DENORMAL el cociente exacto se sale del rango de `Decimal`
            // (~7,9e28) y `/` **panica** («Division overflowed») — en producción, un 400
            // `task_panic` opaco y permanente para ese hogar, el mismo precedente que forzó
            // `checked_mul` en el crecimiento de activos. Un cociente que desborda significa
            // «este tope no ata»: el techo real lo pone `cap`, que se aplica siempre. Por eso
            // el fallback es EXACTO y ningún input que hoy no desborda cambia de valor
            // (`min(⊤, cap) = cap`, y `x.min(top)` se salta cuando `top` no es representable).
            let mut x = match (n_annual - net_acc).checked_div(den) {
                Some(v) => v,
                // `den` denormal (tipo efectivo a un pelo del 100 %): el candidato es
                // ilimitado y manda la capacidad.
                None => cap,
            };
            // …topado por el techo del tramo fiscal (la base llena el tramo)…
            if g > Decimal::ZERO {
                if let Some(ceiling) = brackets[m].up_to {
                    // `g` denormal: la venta que haría falta para llenar el tramo fiscal no es
                    // representable ⇒ este tramo no se llena nunca con este activo y el tope no
                    // recorta. Lo fabrica el propio motor: una cuenta al 0 % alimentada por la
                    // cascada tiene `b` pegada a `v`, el drenaje conserva `b/v` y tras una
                    // venta fuerte queda `g = 1 − b/v ≈ 1e-27` (caso P13 del pin dorado).
                    if let Some(top) = (ceiling - base).checked_div(g) {
                        x = x.min(top);
                    }
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

/// Resultado del paseo DIRECTO (5.0.0 WP2): dado un techo BRUTO, qué se vende de cada tramo y
/// qué netea. Es el gemelo de [`MixedDrawdown`] con la flecha al revés — allí se conoce el neto
/// que hace falta, aquí el bruto que la regla de retirada permite (§B.2 del plan de #207).
#[derive(Debug, Clone)]
pub struct MixedGrossDrawdown {
    /// Bruto realmente vendido: el techo, o menos si las capacidades no llegan.
    pub gross_monthly: Decimal,
    /// Paralelo a `segments`: bruto vendido de cada tramo.
    pub per_segment_monthly: Vec<Decimal>,
    /// Neto que esa venta deja en el bolsillo, tras el impuesto por tramos sobre la base
    /// agregada `Σ g_i·venta_i`.
    pub net_monthly: Decimal,
}

/// **Paseo DIRECTO por el mapa lineal a trozos** (5.0.0 WP2): vende exactamente `gross_cap`
/// (o todo lo que haya, si es menos) sobre `segments` en su orden, y devuelve lo que netea.
///
/// Por qué existe: las reglas de retirada de 5.0.0 ponen un techo **BRUTO** (R9 — el `pct` es
/// bruto de impuestos, como el SWR). Con `g` uniforme el neto de un bruto es
/// [`after_tax_monthly`] y no hace falta nada más; con `g` heterogénea por activo (#178) la base
/// imponible es `Σ g_i·venta_i` y el neto depende del REPARTO, así que hay que recorrer el mismo
/// mapa que [`gross_up_mixed_monthly`] invierte.
///
/// **Y se RECORRE, no se busca.** El neto `F(G) = G − tax(B(G))` es lineal a trozos con pendiente
/// `1 − r·g_j` mientras se vacía el tramo `j` bajo el tipo `r`; sus quiebros son las fronteras de
/// capacidad (cambia `g`) y los techos de tramo fiscal (cambia `r`). Recorrer los quiebros da el
/// resultado EXACTO en ≤ `n + |tramos|` pasos. Una bisección sobre esta misma función es la
/// familia retirada por arqueología (§2.23 de `futurefin-failure-archaeology`): convergencia
/// lineal a razón ~0,11, oscilación en las fronteras de activo y ningún número reproducible a
/// mano. Aquí no hay tolerancias porque no hay búsqueda.
///
/// Convención M1 idéntica a la del resto del módulo: se trabaja en unidades ANUALES (`12·cap`,
/// capacidades `12·v`) y se divide por 12 al devolver.
///
/// Casos: `taxes_enabled = false` ⇒ llenado secuencial sin impuesto (el neto ES el bruto).
/// `gross_cap ≤ 0` ⇒ no se vende nada. Un tipo efectivo ≥ 100 % sobre un tramo (que la API no
/// permite: los `pct` están acotados) haría el neto marginal negativo; el resultado se publica
/// clampado a 0 porque **vender no puede costarle dinero al hogar** en este modelo.
pub fn mixed_drawdown_for_gross_cap(
    gross_cap_monthly: Decimal,
    segments: &[MixedSegment],
    brackets: &[TaxBracket],
    taxes_enabled: bool,
) -> MixedGrossDrawdown {
    let twelve = Decimal::from(12u32);
    let mut out = MixedGrossDrawdown {
        gross_monthly: Decimal::ZERO,
        per_segment_monthly: vec![Decimal::ZERO; segments.len()],
        net_monthly: Decimal::ZERO,
    };
    if gross_cap_monthly <= Decimal::ZERO {
        return out;
    }
    let cap_annual = gross_cap_monthly * twelve;

    if !taxes_enabled {
        let mut remaining = cap_annual;
        for (j, s) in segments.iter().enumerate() {
            if remaining <= Decimal::ZERO {
                break;
            }
            let take = (s.capacity_monthly * twelve)
                .max(Decimal::ZERO)
                .min(remaining);
            out.per_segment_monthly[j] = take / twelve;
            out.gross_monthly += take / twelve;
            remaining -= take;
        }
        out.net_monthly = out.gross_monthly;
        return out;
    }

    let hundred = Decimal::from(100u32);
    let mut base = Decimal::ZERO; // base imponible ANUAL acumulada
    let mut net_acc = Decimal::ZERO;
    let mut gross_annual = Decimal::ZERO;
    let mut remaining = cap_annual;
    let mut m = 0usize; // índice de tramo fiscal — monótono, nunca retrocede

    for (j, s) in segments.iter().enumerate() {
        let mut cap = (s.capacity_monthly * twelve).max(Decimal::ZERO);
        let g = s.gain_ratio.clamp(Decimal::ZERO, Decimal::ONE);
        let mut taken_annual = Decimal::ZERO;
        while cap > Decimal::ZERO && remaining > Decimal::ZERO {
            // Sin escala (o agotada — imposible por contrato: el último tramo es abierto) el
            // resto se vende sin impuesto adicional.
            let (r, ceiling) = match brackets.get(m) {
                Some(b) => (b.pct / hundred, b.up_to),
                None => (Decimal::ZERO, None),
            };
            let mut x = remaining.min(cap);
            if g > Decimal::ZERO {
                if let Some(c) = ceiling {
                    // `checked_div` por la misma razón que en el paseo inverso (#208): con una
                    // `g` DENORMAL el cociente se sale del rango de `Decimal` y `/` panica. Un
                    // tope que no es representable es un tope que no ata.
                    if let Some(top) = (c - base).checked_div(g) {
                        if top < x {
                            x = top;
                        }
                    }
                }
            }
            if x <= Decimal::ZERO {
                // El tramo fiscal está lleno EXACTAMENTE aquí: se avanza y se reintenta. Si no
                // es eso, se corta — un `x` no positivo que no avanza sería un bucle infinito.
                match ceiling {
                    Some(c) if base >= c => {
                        m += 1;
                        continue;
                    }
                    _ => break,
                }
            }
            let den = Decimal::ONE - r * g;
            net_acc += x * den;
            base += x * g;
            gross_annual += x;
            cap -= x;
            remaining -= x;
            taken_annual += x;
            if let Some(c) = ceiling {
                if base >= c {
                    m += 1;
                }
            }
        }
        out.per_segment_monthly[j] = taken_annual / twelve;
    }
    out.gross_monthly = gross_annual / twelve;
    out.net_monthly = (net_acc / twelve).max(Decimal::ZERO);
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

    /// **Frontera exacta del desbordamiento de la issue #208.** El tope de tramo del solver
    /// mixto es `(techo − base) / g` y solo estaba guardado por `g > 0`. Con el techo de
    /// 200.000 € de la escala ES y `g = 1e-27` el cociente vale `2e32` y NO cabe en un `Decimal`
    /// (~7,9e28): `/` panicaba con «Division overflowed» y el pool blocking lo publicaba como un
    /// 400 `task_panic`. Con `g = 1e-20` el mismo cociente es `2e25` y siempre cupo — las dos
    /// mitades van juntas a propósito: la primera es el caso que ANTES panicaba, la segunda el
    /// control que prueba que el arreglo no movió nada donde no desbordaba.
    ///
    /// La `g` denormal no es de laboratorio: la fabrica el propio motor (cuenta al 0 % alimentada
    /// por la cascada + venta fuerte ⇒ `g = 1 − b/v ≈ 1e-27`). El caso `P13_cash8k_denormal_g`
    /// del pin dorado lo reproduce dentro de una proyección completa de 840 meses.
    #[test]
    fn a_denormal_gain_ratio_does_not_overflow_the_bracket_ceiling() {
        // Un tramo cerrado en 200.000 € (el techo del issue) y el abierto obligatorio.
        let br = vec![
            TaxBracket {
                up_to: Some(Decimal::from(200_000u32)),
                pct: Decimal::from(19u32),
            },
            TaxBracket {
                up_to: None,
                pct: Decimal::from(30u32),
            },
        ];
        let run = |net_monthly: u32, cap_monthly: u32, g: Decimal| {
            gross_up_mixed_monthly(
                Decimal::from(net_monthly),
                &[MixedSegment {
                    capacity_monthly: Decimal::from(cap_monthly),
                    gain_ratio: g,
                }],
                &br,
                true,
            )
        };

        // (a) 1e-27 — ANTES: pánico. AHORA: el tope no ata (no es representable) y manda la
        //     capacidad, que sobra: se vende lo justo para netear 1.000 €, y con una plusvalía
        //     gravable de 1e-27 el impuesto es aritméticamente despreciable.
        let denormal = run(1_000, 10_000, Decimal::new(1, 27));
        assert_eq!(denormal.net_shortfall_monthly, Decimal::ZERO);
        assert!(
            denormal.gross_monthly >= Decimal::from(1_000u32)
                && denormal.gross_monthly < Decimal::from(1_001u32),
            "con g≈0 el bruto es el neto: {}",
            denormal.gross_monthly
        );
        assert_eq!(
            denormal.per_segment_monthly[0], denormal.gross_monthly,
            "un único tramo se lleva toda la venta"
        );

        // (b) 1e-20 — el cociente cabía ya en 4.15.0 y el arreglo NO lo toca: mismo camino
        //     (`checked_div` devuelve `Some`), mismo valor.
        let normal = run(1_000, 10_000, Decimal::new(1, 20));
        assert_eq!(normal.net_shortfall_monthly, Decimal::ZERO);
        assert!(
            normal.gross_monthly >= Decimal::from(1_000u32)
                && normal.gross_monthly < Decimal::from(1_001u32),
            "el control tampoco se mueve: {}",
            normal.gross_monthly
        );

        // (c) Control del tope que SÍ ata: con `g = 1` y 25.000 €/mes netos la base cruza el
        //     techo de 200.000 €, así que `x.min((techo − base)/g)` recorta de verdad y el
        //     paseo salta al tramo del 30 %. A mano: los primeros 200.000 € brutos netean
        //     200.000·0,81 = 162.000; faltan 138.000 netos que salen a 0,70 ⇒
        //     138.000/0,7 = 197.142,857142857142857142857143; total 397.142,857142857142857142857143
        //     anuales ⇒ 33.095,238095238095238095238095 €/mes. Es la rama que el arreglo NO
        //     puede haber tocado, y el número lo demuestra.
        let gravada = run(25_000, 100_000, Decimal::ONE);
        assert_eq!(
            gravada.net_shortfall_monthly,
            Decimal::ZERO,
            "100.000 €/mes de capacidad cubren de sobra la venta"
        );
        assert_eq!(
            gravada.gross_monthly.round_dp(12),
            Decimal::new(33_095_238_095_238_095, 12)
        );
    }

    // ── Paseo DIRECTO con techo BRUTO (5.0.0 WP2) ───────────────────────────────────────────

    /// **PREDICCIÓN a mano (§B.2).** Techo bruto 2.010 €/mes sobre A(1.000, g=0,2) +
    /// B(200.000, g=0,5), escala ES. En unidades anuales (M1: 24.120 €):
    ///
    /// | tramo | `g` | tipo | venta | base | neto |
    /// |---|---|---|---|---|---|
    /// | A (capacidad 12.000) | 0,2 | 19 % | 12.000 | 2.400 | `12.000·0,962 = 11.544` |
    /// | B hasta llenar el tramo | 0,5 | 19 % | `(6.000−2.400)/0,5 = 7.200` | 6.000 | `7.200·0,905 = 6.516` |
    /// | B, resto del techo | 0,5 | 21 % | 4.920 | 8.460 | `4.920·0,895 = 4.403,40` |
    ///
    /// Bruto 24.120 (el techo EXACTO), neto 22.463,40 ⇒ 2.010 y **1.871,95** €/mes. Partida
    /// doble contra la escala: `tax(8.460) = 6.000·0,19 + 2.460·0,21 = 1.656,60`, y
    /// `24.120 − 1.656,60 = 22.463,40`. ✓
    #[test]
    fn the_gross_walk_crosses_a_capacity_boundary_and_a_bracket_ceiling() {
        let br = es_brackets_for_tests();
        let segs = [seg(1_000, "0.2"), seg(200_000, "0.5")];
        let w = mixed_drawdown_for_gross_cap(Decimal::from(2010u32), &segs, &br, true);
        assert_eq!(
            w.gross_monthly,
            Decimal::from(2010u32),
            "vende el techo exacto"
        );
        assert_eq!(w.per_segment_monthly[0], Decimal::from(1000u32));
        assert_eq!(w.per_segment_monthly[1], Decimal::from(1010u32));
        assert_eq!(w.net_monthly, "1871.95".parse::<Decimal>().unwrap());
        // Partida doble con la escala, sobre la base agregada real.
        let base = Decimal::from(12u32)
            * (w.per_segment_monthly[0] * "0.2".parse::<Decimal>().unwrap()
                + w.per_segment_monthly[1] * "0.5".parse::<Decimal>().unwrap());
        assert_eq!(base, Decimal::from(8_460u32));
        let neto = w.gross_monthly * Decimal::from(12u32) - tax_on_gross_capital_annual(base, &br);
        assert_eq!(neto / Decimal::from(12u32), w.net_monthly);
    }

    /// **El par redondo de los dos paseos**: pedirle al directo el bruto que el inverso calculó
    /// devuelve el MISMO reparto y recupera el neto pedido. Sin esto, las dos direcciones podrían
    /// describir mapas distintos y nadie se enteraría (la rama de déficit usa las dos).
    #[test]
    fn the_gross_walk_is_the_exact_inverse_of_the_net_walk() {
        let br = es_brackets_for_tests();
        for segs in [
            vec![seg(1_000, "0.2"), seg(200_000, "0.5")],
            vec![seg(500, "0"), seg(3_000, "1"), seg(50_000, "0.35")],
            vec![seg(10_000, "0.8"), seg(10_000, "0.1")],
        ] {
            for net in ["800", "2500", "9000"] {
                let net: Decimal = net.parse().unwrap();
                let inverse = gross_up_mixed_monthly(net, &segs, &br, true);
                assert_eq!(
                    inverse.net_shortfall_monthly,
                    Decimal::ZERO,
                    "capacidad de sobra"
                );
                let forward = mixed_drawdown_for_gross_cap(inverse.gross_monthly, &segs, &br, true);
                assert_eq!(
                    forward.gross_monthly, inverse.gross_monthly,
                    "el directo vende el bruto entero"
                );
                // El reparto coincide hasta el último dígito de los 28 de `Decimal`: los dos
                // paseos llegan al mismo trozo por multiplicaciones y divisiones en distinto
                // orden (la razón por la que la rama de déficit CORTOCIRCUITA a la vía escalar
                // con `g` uniforme en vez de fiarse de la igualdad bit a bit).
                for (a, b) in forward
                    .per_segment_monthly
                    .iter()
                    .zip(inverse.per_segment_monthly.iter())
                {
                    assert!(
                        (a - b).abs() < Decimal::new(1, 12),
                        "reparto distinto: {a} vs {b}"
                    );
                }
                assert!(
                    (forward.net_monthly - net).abs() < Decimal::new(1, 12),
                    "el neto recuperado ({}) no es el pedido ({net})",
                    forward.net_monthly
                );
            }
        }
    }

    /// Un techo por encima de la capacidad vende TODO y lo dice: el bruto devuelto es la suma de
    /// capacidades, no el techo. (Es lo que distingue «la regla permitía más» de «la cartera no
    /// daba más», las dos magnitudes que 5.0.0 publica por separado.)
    #[test]
    fn a_gross_cap_above_the_capacity_sells_everything_and_says_so() {
        let br = es_brackets_for_tests();
        let segs = [seg(100, "1"), seg(200, "0.5")];
        let w = mixed_drawdown_for_gross_cap(Decimal::from(5_000u32), &segs, &br, true);
        assert_eq!(w.gross_monthly, Decimal::from(300u32));
        assert_eq!(w.per_segment_monthly[0], Decimal::from(100u32));
        assert_eq!(w.per_segment_monthly[1], Decimal::from(200u32));
        // Base anual = 12·(100·1 + 200·0,5) = 2.400 ⇒ tramo del 19 % ⇒ impuesto 456 ⇒
        // neto anual 3.600 − 456 = 3.144 ⇒ 262 €/mes.
        assert_eq!(w.net_monthly, Decimal::from(262u32));
    }

    /// Con `g` uniforme el paseo directo ES [`after_tax_monthly`] — la comprobación que ata las
    /// dos fiscalidades del drenaje (escalar y mixta) al mismo euro.
    #[test]
    fn the_gross_walk_with_uniform_g_equals_after_tax_monthly() {
        let br = es_brackets_for_tests();
        for g in ["1", "0.5", "0.2", "0"] {
            let gd: Decimal = g.parse().unwrap();
            for gross in ["500", "2010", "12000"] {
                let gross: Decimal = gross.parse().unwrap();
                let w = mixed_drawdown_for_gross_cap(
                    gross,
                    &[seg(1_000_000, g), seg(1_000_000, g)],
                    &br,
                    true,
                );
                assert_eq!(w.gross_monthly, gross);
                let escalar = after_tax_monthly(gross, &br, true, gd);
                assert!(
                    (w.net_monthly - escalar).abs() < Decimal::new(1, 12),
                    "g={g}, bruto={gross}: paseo {} vs escalar {escalar}",
                    w.net_monthly
                );
            }
        }
    }

    /// Sin impuestos el paseo directo es la identidad, y una `g` denormal (la que fabrica el
    /// propio motor, #208) no lo desborda.
    #[test]
    fn the_gross_walk_handles_taxes_off_and_a_denormal_gain_ratio() {
        let br = es_brackets_for_tests();
        let segs = [seg(1_000, "0.9"), seg(10_000, "0.1")];
        let w = mixed_drawdown_for_gross_cap(Decimal::from(1_500u32), &segs, &br, false);
        assert_eq!(w.gross_monthly, Decimal::from(1_500u32));
        assert_eq!(w.net_monthly, Decimal::from(1_500u32));
        assert_eq!(w.per_segment_monthly[0], Decimal::from(1_000u32));
        assert_eq!(w.per_segment_monthly[1], Decimal::from(500u32));

        let denormal = [
            MixedSegment {
                capacity_monthly: Decimal::from(10_000u32),
                gain_ratio: Decimal::new(1, 27),
            },
            seg(10_000, "1"),
        ];
        let w = mixed_drawdown_for_gross_cap(Decimal::from(1_000u32), &denormal, &br, true);
        assert_eq!(w.gross_monthly, Decimal::from(1_000u32));
        assert!(
            w.net_monthly > Decimal::from(999u32) && w.net_monthly <= Decimal::from(1_000u32),
            "con g≈0 el neto es el bruto: {}",
            w.net_monthly
        );
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
