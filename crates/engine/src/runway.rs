//! Runway de liquidez: cuántos meses aguantan los activos **líquidos** pagando el gasto mensual,
//! componiendo la rentabilidad esperada de esos activos y la inflación del gasto.

use rust_decimal::Decimal;

use crate::projection::monthly_multiplier;

/// Tope del bucle finito: 1.200 meses = 100 años. Sobrevivirlo ya **no** significa `Indefinite`
/// (eso lo decide el umbral SWR): se reporta como `Months(1200)`, un **suelo** («al menos 100
/// años»), porque a partir de ahí el número exacto deja de ser informativo.
pub const MAX_RUNWAY_MONTHS: u32 = 1200;

/// Resultado del runway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunwayOutcome {
    /// Meses cubiertos, con el último mes **fraccionario** (p.ej. `10.5` = diez meses y medio).
    /// El valor `1200` (`MAX_RUNWAY_MONTHS`) es el tope del bucle y significa «≥ 100 años»,
    /// no una medida exacta.
    Months(Decimal),
    /// La retirada anual (el gasto anual grosseado, `annual_expense_for_swr`) no supera el SWR
    /// aplicado al saldo líquido — `annual_expense_for_swr ≤ A·(swr_pct/100)` — **y** la cartera
    /// líquida tiene rentabilidad esperada ponderada > 0 (#128): la regla del SWR se validó para
    /// carteras invertidas, nunca para dinero parado al 0 %.
    Indefinite,
    /// No hay base de gasto (`monthly_expense <= 0`): el runway no está definido.
    NoExpenseBase,
}

/// Meses de runway de los activos líquidos, con rentabilidad e inflación compuestas.
///
/// `liquid_assets` son pares `(valor, rentabilidad anual %)`; `monthly_expense` es el gasto
/// mensual total de partida y `annual_inflation_percent` la inflación anual que lo encarece.
/// `swr_pct` es la tasa de retirada segura de la instalación (`fire_settings.swr_pct`, en %) y
/// `annual_expense_for_swr` el gasto **anual** ya grosseado por impuestos por el handler (el
/// mismo `gross_up` del FIRE target; sin impuestos es simplemente `12·monthly_expense`).
///
/// # Modelo
///
/// - **Marco nominal**: los activos crecen a su rentabilidad *nominal* y el gasto se infla cada
///   mes. No se deflacta nada: el resultado (un número de meses) es invariante al marco, pero
///   mezclar retorno nominal con gasto constante sobreestimaría el runway.
/// - **Orden retirada-antes-de-crecimiento**: cada mes se paga el gasto y luego crece el saldo
///   restante — el mismo orden que el bucle de simulación de `projection.rs` (drenaje del cash
///   flow negativo antes de aplicar los multiplicadores), así que ambas curvas son coherentes.
/// - **Drenaje secuencial (#128)**: cada mes el gasto se cubre vaciando los activos en el MISMO
///   orden que `drain_from_assets` en la simulación real — menor rentabilidad esperada primero
///   (`None` cuenta como 0), empate por índice — y después cada saldo restante crece con SU
///   propio multiplicador. Hasta 4.7.x se usaba una media de multiplicadores ponderada por valor
///   (drenaje prorrateado), sistemáticamente **más corta**: prorratear consume también los
///   activos de rentabilidad alta desde el mes 1, mientras el drain real los conserva
///   componiendo. En el caso de un solo activo ambos modelos coinciden exactamente.
/// - **Tasas negativas componen**: herencia documentada de [`monthly_multiplier`] — `None` y 0
///   siguen siendo factor 1, pero una rentabilidad negativa (−100 < r < 0) decrece el saldo de
///   verdad y por tanto **acorta** el runway (≤ −100 se clampa a factor 0). La inflación del
///   gasto puede ser NEGATIVA desde 4.9.0 (#146, rango [−2, 50]): el gasto entonces DECRECE mes a mes y el runway se alarga.
/// - **Umbral SWR (caso infinito)**: el runway es `Indefinite` ⟺ la retirada anual no supera el
///   SWR sobre el saldo inicial: `annual_expense_for_swr ≤ A·(swr_pct/100)`. Se compara sin
///   dividir — `annual_expense_for_swr·100 ≤ A·swr_pct` — para que la frontera sea **exacta** en
///   `Decimal`. Con `swr_pct ≤ 0` el lado derecho es ≤ 0 y el izquierdo > 0, así que nunca se
///   cumple (no necesita guarda aparte). Es el «FIRE number» de liquidez: `A ≥ gasto_bruto/SWR`.
/// - **Puerta de rentabilidad (#128)**: el umbral SWR solo declara `Indefinite` si además la
///   rentabilidad esperada ponderada de la cartera líquida es **estrictamente positiva**
///   (`Σ vₐ·rₐ > 0`, con `None` = 0; sin dividir — `A > 0` hace equivalentes la suma y la
///   media). La regla del 3,5/4 % (Trinity/Bengen) se validó para carteras invertidas con
///   retorno esperado positivo, nunca para dinero parado: 300.000 € al 0 % con gasto de
///   875 €/mes cumplen el umbral por igualdad exacta y aun así se agotan en 342,86 meses — con
///   la puerta, ese caso cae al bucle finito y publica esa cifra. La inflación sigue SIN mirar
///   el disparador (gobierna solo el caso finito): la definición de SWR ya la asume dentro del
///   retorno real de la cartera.
/// - **Orden de checks**: `NoExpenseBase` se decide **antes** que el umbral SWR — con gasto 0 la
///   desigualdad `0 ≤ A·swr` sería trivialmente cierta y marcaría infinito un runway indefinido.
/// - **Tope de 100 años**: si el saldo aguanta `MAX_RUNWAY_MONTHS` meses sin haber cumplido el
///   umbral SWR se devuelve `Months(1200)` como **suelo** («al menos 100 años»), no `Indefinite`:
///   el infinito lo decide en exclusiva el SWR.
///
/// # Reducción exacta a `A / g`
///
/// Cuando no se declara `Indefinite`, con rentabilidad e inflación 0 todos los multiplicadores
/// son 1 y el drenaje secuencial resta exactamente `g` del total cada mes: `total_k = A − k·g`
/// con `g` constante (da igual de qué activo salga cada euro). Sea `n = ⌊A/g⌋`: para todo
/// `k ≤ n` se cumple `total_{k−1} = A − (k−1)·g ≥ g`, así que el bucle no corta; en `k = n+1`
/// se cumple `total_n = A − n·g < g` y se devuelve `n + (A − n·g)/g = A/g`. La única división
/// es la final, de modo que el resultado es la división simple exacta (regresión sin
/// tolerancias en los tests).
///
/// Casos límite: `monthly_expense <= 0` → [`RunwayOutcome::NoExpenseBase`]; saldo total ≤ 0 →
/// `Months(0)`.
pub fn liquid_runway_months(
    liquid_assets: &[(Decimal, Option<Decimal>)],
    monthly_expense: Decimal,
    annual_inflation_percent: Decimal,
    swr_pct: Decimal,
    annual_expense_for_swr: Decimal,
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

    // Infinito ⟺ la retirada anual (grosseada) no supera el SWR **y** la cartera tiene
    // rentabilidad esperada ponderada > 0 (#128). Comparaciones exactas sin dividir.
    // OJO: el `100` desporcentúa `swr_pct` — no confundir ni compartir con `MAX_RUNWAY_MONTHS`
    // aunque 12·100 = 1200 coincidan numéricamente. Y `Σ v·r > 0` equivale a la media ponderada
    // `Σ v·r / A > 0` porque aquí `A > 0` (garantizado por la guarda de arriba).
    let weighted_expected_return: Decimal = liquid_assets
        .iter()
        .map(|(v, r)| *v * r.unwrap_or(Decimal::ZERO))
        .sum();
    if annual_expense_for_swr * Decimal::from(100u32) <= balance_0 * swr_pct
        && weighted_expected_return > Decimal::ZERO
    {
        return RunwayOutcome::Indefinite;
    }

    // Drenaje secuencial (#128): menor rentabilidad primero, empate por índice — el mismo orden
    // que `drain_from_assets` dentro del grupo líquido. Cada mes: pagar el gasto vaciando en ese
    // orden, y DESPUÉS crecer cada saldo restante con su propio multiplicador (retirada antes de
    // crecimiento, como el bucle de simulación).
    let mut vals: Vec<Decimal> = liquid_assets.iter().map(|(v, _)| *v).collect();
    let mults: Vec<Decimal> = liquid_assets
        .iter()
        .map(|(_, r)| monthly_multiplier(*r))
        .collect();
    let mut order: Vec<usize> = (0..vals.len()).collect();
    order.sort_by(|&i, &j| {
        liquid_assets[i]
            .1
            .unwrap_or(Decimal::ZERO)
            .cmp(&liquid_assets[j].1.unwrap_or(Decimal::ZERO))
            .then_with(|| i.cmp(&j))
    });
    let m_inf = monthly_multiplier(Some(annual_inflation_percent));

    let mut g = monthly_expense;
    for k in 1..=MAX_RUNWAY_MONTHS {
        let total: Decimal = vals.iter().sum();
        if total < g {
            // Mes final fraccionario: la parte del mes k que el saldo aún cubre.
            return RunwayOutcome::Months(Decimal::from(k - 1) + total / g);
        }
        let mut need = g;
        for &idx in &order {
            if need <= Decimal::ZERO {
                break;
            }
            // Un valor individual negativo no «drena» (take clampado a ≥ 0): con el modelo
            // antiguo los negativos solo restaban del saldo agregado, y eso sigue siendo su
            // único efecto (entran en `total`, nunca en la cobertura del gasto).
            let take = vals[idx].max(Decimal::ZERO).min(need);
            vals[idx] -= take;
            need -= take;
        }
        for (v, m) in vals.iter_mut().zip(mults.iter()) {
            *v *= *m;
        }
        g *= m_inf;
    }
    // El saldo sobrevive el tope sin haber cumplido el umbral SWR: NO es infinito, pero el
    // número exacto ya no informa. Se devuelve el tope como SUELO («al menos 100 años»); la UI
    // lo renderiza como «+100 años».
    RunwayOutcome::Months(Decimal::from(MAX_RUNWAY_MONTHS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    /// SWR por defecto de la instalación (`default_fire_settings`): 3,5 %.
    fn swr() -> Decimal {
        Decimal::new(35, 1)
    }

    /// Llamada con el gasto anual sin gross-up (`12·g`) — el caso de los tests sin impuestos.
    fn runway(
        assets: &[(Decimal, Option<Decimal>)],
        g: Decimal,
        infl: Decimal,
        swr_pct: Decimal,
    ) -> RunwayOutcome {
        liquid_runway_months(assets, g, infl, swr_pct, g * d(12))
    }

    fn months(o: RunwayOutcome) -> Decimal {
        match o {
            RunwayOutcome::Months(m) => m,
            other => panic!("expected Months, got {other:?}"),
        }
    }

    /// Réplica del modelo ANTIGUO (un único multiplicador sobre el saldo agregado, ≤ 4.7.x) —
    /// solo para los tests que demuestran que el drenaje secuencial (#128) da MÁS meses.
    /// Sin inflación: `g` es constante (los tests que la usan la fijan a 0).
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
        let entero = runway(&[(d(12_000), None)], d(1_000), Decimal::ZERO, swr());
        assert_eq!(entero, RunwayOutcome::Months(d(12)));

        let fraccionario = runway(&[(d(10_000), None)], d(3_000), Decimal::ZERO, swr());
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
        let con_retorno = months(runway(
            &[(d(12_000), Some(d(5)))],
            d(1_000),
            Decimal::ZERO,
            swr(),
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
        let con_inflacion = months(runway(&[(d(12_000), None)], d(1_000), d(3), swr()));
        assert!(
            con_inflacion < d(12),
            "esperado < 12 meses, obtenido {con_inflacion}"
        );
    }

    /// 1.000.000 con gasto de 1.000/mes: la retirada anual es 12.000 €, muy por debajo del
    /// 3,5 % del saldo (35.000 €) → `Indefinite` por el umbral SWR (con el criterio anterior
    /// —sobrevivir el cap— este escenario también era indefinido: el pinning cruza el cambio).
    #[test]
    fn withdrawal_within_swr_is_indefinite() {
        let out = runway(&[(d(1_000_000), Some(d(7)))], d(1_000), Decimal::ZERO, swr());
        assert_eq!(out, RunwayOutcome::Indefinite);
    }

    /// INVERTIDO en la Ola 4 (#128): hasta 4.7.x este test fijaba el multiplicador único
    /// ponderado por valor (`weighted_rate_uses_value_weights`); ahora fija el drenaje
    /// secuencial, que conserva los activos de rentabilidad alta mientras vacía los de baja.
    ///
    /// 150.000 al 0 % + 50.000 al 10 %, gasto 2.000 €/mes, sin inflación. A mano: la fase 1
    /// drena el activo al 0 % en 150.000/2.000 = 75 meses exactos, mientras el activo al 10 %
    /// compone intocado hasta 50.000·1,1^(75/12) ≈ 90.758 €; la fase 2 lo drena componiendo,
    /// ≈ 56 meses más → ≈ 130,96 meses. El modelo antiguo (prorrateo) daba ≈ 111,39: casi 20
    /// meses menos, porque consumía el activo al 10 % desde el mes 1.
    #[test]
    fn sequential_drain_preserves_the_high_return_assets() {
        let v_lento = d(150_000);
        let v_rapido = d(50_000);
        let m10 = monthly_multiplier(Some(d(10)));
        let balance_0 = v_lento + v_rapido;

        let secuencial = months(runway(
            &[(v_lento, None), (v_rapido, Some(d(10)))],
            d(2_000),
            Decimal::ZERO,
            swr(),
        ));
        assert!(
            secuencial > d(13_095) / d(100) && secuencial < d(13_096) / d(100),
            "esperado ≈130,958 meses, obtenido {secuencial}"
        );

        let m_ponderado = (v_lento * Decimal::ONE + v_rapido * m10) / balance_0;
        let antiguo = reference_loop(balance_0, m_ponderado, d(2_000));
        assert!(
            antiguo > d(11_139) / d(100) && antiguo < d(11_140) / d(100),
            "la réplica del modelo antiguo debe dar ≈111,39, obtenido {antiguo}"
        );
        assert!(
            secuencial > antiguo,
            "el drenaje secuencial nunca da menos meses que el prorrateo ({secuencial} vs {antiguo})"
        );
    }

    /// Con UN solo activo el drenaje secuencial y el multiplicador único coinciden exactamente:
    /// no hay orden que elegir y el multiplicador ponderado es el del propio activo. Fija que
    /// el cambio de modelo de #128 NO movió ninguna cifra de cartera mono-activo.
    #[test]
    fn single_asset_matches_the_old_single_multiplier_model() {
        let out = months(runway(
            &[(d(24_000), Some(d(6)))],
            d(1_500),
            Decimal::ZERO,
            swr(),
        ));
        assert_eq!(out, reference_loop(d(24_000), monthly_multiplier(Some(d(6))), d(1_500)));
    }

    /// Desde el fix de `monthly_multiplier` las pérdidas componen: −5% anual decrece el saldo
    /// mientras se consume, así que el runway es ESTRICTAMENTE menor que los 12 meses de la
    /// división simple (antes del cambio −5% y «sin tasa» eran idénticos: 12 exactos).
    #[test]
    fn negative_return_shortens_runway() {
        let negativo = months(runway(
            &[(d(12_000), Some(d(-5)))],
            d(1_000),
            Decimal::ZERO,
            swr(),
        ));
        let sin_tasa = months(runway(&[(d(12_000), None)], d(1_000), Decimal::ZERO, swr()));
        assert_eq!(sin_tasa, d(12));
        assert!(
            negativo < sin_tasa,
            "esperado < 12 meses con retorno −5%, obtenido {negativo}"
        );
        assert!(
            negativo > d(11),
            "−5% anual apenas recorta unos días sobre 12 meses, obtenido {negativo}"
        );
    }

    /// Sin gasto el runway no está definido (ni siquiera «infinito»): `NoExpenseBase`. Este test
    /// también fija el ORDEN de los checks: con gasto 0 la desigualdad SWR (`0 ≤ A·swr`) sería
    /// trivialmente cierta, así que `NoExpenseBase` debe decidirse antes que el umbral.
    #[test]
    fn zero_expense_is_no_expense_base() {
        assert_eq!(
            runway(&[(d(12_000), Some(d(5)))], Decimal::ZERO, d(3), swr()),
            RunwayOutcome::NoExpenseBase
        );
    }

    /// Sin activos líquidos (o con saldo 0) el runway es 0 meses, no `None`.
    #[test]
    fn zero_balance_is_zero_months() {
        assert_eq!(
            runway(&[], d(1_000), Decimal::ZERO, swr()),
            RunwayOutcome::Months(Decimal::ZERO)
        );
        assert_eq!(
            runway(&[(Decimal::ZERO, Some(d(5)))], d(1_000), Decimal::ZERO, swr()),
            RunwayOutcome::Months(Decimal::ZERO)
        );
    }

    /// INVERTIDO en la Ola 4 (#128): hasta 4.7.x la igualdad exacta del umbral bastaba para
    /// `Indefinite` aunque la cartera rindiera 0 % («el KPI mide tasa de retirada, no
    /// supervivencia»). Ahora la puerta de rentabilidad exige retorno esperado ponderado > 0:
    /// el MISMO escenario (300.000 sin rentabilidad, 4 % SWR, 1.000 €/mes = igualdad exacta
    /// 1.200.000 = 1.200.000) cae al bucle finito y publica la verdad — se agota en
    /// 300.000/1.000 = **300 meses** exactos (reducción A/g). Con cualquier retorno positivo,
    /// la igualdad sigue bastando: la frontera exacta en `Decimal` no cambió.
    #[test]
    fn swr_threshold_equality_needs_positive_expected_return() {
        let sin_retorno = runway(&[(d(300_000), None)], d(1_000), Decimal::ZERO, d(4));
        assert_eq!(sin_retorno, RunwayOutcome::Months(d(300)));

        let cero_explicito = runway(
            &[(d(300_000), Some(Decimal::ZERO))],
            d(1_000),
            Decimal::ZERO,
            d(4),
        );
        assert_eq!(cero_explicito, RunwayOutcome::Months(d(300)));

        let con_retorno = runway(&[(d(300_000), Some(d(2)))], d(1_000), Decimal::ZERO, d(4));
        assert_eq!(con_retorno, RunwayOutcome::Indefinite);
    }

    /// El escenario del issue #128: 300.000 € en cuenta corriente al 0 %, gasto 875 €/mes,
    /// SWR 3,5 %. El umbral se cumple por igualdad exacta (10.500·100 = 1.050.000 =
    /// 300.000·3,5) pero la puerta de rentabilidad lo tumba: ese dinero se agota en
    /// 300.000/875 = **342,857142… meses** (≈ 28,6 años), y eso es lo que se publica.
    #[test]
    fn parked_cash_at_zero_return_is_never_indefinite() {
        let out = runway(&[(d(300_000), Some(Decimal::ZERO))], d(875), Decimal::ZERO, swr());
        assert_eq!(out, RunwayOutcome::Months(d(300_000) / d(875)));
    }

    /// El caso mixto del issue #128: 10.000 al 0 % + 10.000 al 10 %, gasto 1.000 €/mes,
    /// SWR 3,5 (umbral no cumplido: 12.000·100 > 20.000·3,5). A mano: 10 meses drenando el
    /// activo al 0 % mientras el del 10 % compone hasta ≈ 10.827 €, luego ≈ 11,3 meses más
    /// drenándolo → ≈ **21,27 meses**. El prorrateo antiguo daba ≈ 20,80: la tarjeta
    /// «Autonomía» ahora coincide con lo que `drain_from_assets` simula de verdad.
    #[test]
    fn mixed_portfolio_matches_the_engines_sequential_drain() {
        let secuencial = months(runway(
            &[(d(10_000), Some(Decimal::ZERO)), (d(10_000), Some(d(10)))],
            d(1_000),
            Decimal::ZERO,
            swr(),
        ));
        assert!(
            secuencial > d(2_127) / d(100) && secuencial < d(2_128) / d(100),
            "esperado ≈21,2746 meses, obtenido {secuencial}"
        );

        let m10 = monthly_multiplier(Some(d(10)));
        let m_ponderado = (d(10_000) * Decimal::ONE + d(10_000) * m10) / d(20_000);
        let antiguo = reference_loop(d(20_000), m_ponderado, d(1_000));
        assert!(
            secuencial > antiguo,
            "y siempre por encima del prorrateo ({secuencial} vs {antiguo})"
        );
    }

    /// Un euro por debajo del umbral (299.999 al 4 % < 12.000 €/año) → finito, y como no hay
    /// rentabilidad ni inflación la reducción exacta a `A/g` sigue intacta: 299.999/1.000.
    #[test]
    fn just_below_swr_threshold_is_finite() {
        let out = runway(&[(d(299_999), None)], d(1_000), Decimal::ZERO, d(4));
        assert_eq!(out, RunwayOutcome::Months(d(299_999) / d(1_000)));
    }

    /// `swr = 0` (válido en la API) nunca marca infinito, por grande que sea el saldo o la
    /// rentabilidad: el escenario de 1M al 7 % que con SWR 3,5 es `Indefinite` aquí agota el
    /// tope y devuelve el suelo `Months(1200)`. También defensivo con `swr < 0`: el JSONB
    /// almacenado no se revalida al leer.
    #[test]
    fn swr_zero_never_indefinite() {
        let cero = runway(&[(d(1_000_000), Some(d(7)))], d(1_000), Decimal::ZERO, Decimal::ZERO);
        assert_eq!(cero, RunwayOutcome::Months(Decimal::from(MAX_RUNWAY_MONTHS)));

        let negativo = runway(&[(d(1_000_000), Some(d(7)))], d(1_000), Decimal::ZERO, d(-1));
        assert_eq!(negativo, RunwayOutcome::Months(Decimal::from(MAX_RUNWAY_MONTHS)));
    }

    /// Retirada por ENCIMA del SWR (48.000 > 35.000 = 1M·3,5 %) pero retorno que cubre el gasto
    /// (7 % ⇒ ~5.654 €/mes > 4.000): el saldo sobrevive el tope y se devuelve `Months(1200)`
    /// como suelo, NO `Indefinite` — el infinito lo decide en exclusiva el SWR. (En 2.2.0 este
    /// escenario era `Indefinite` por el cap.)
    #[test]
    fn cap_reached_without_swr_is_months_at_cap() {
        let out = runway(&[(d(1_000_000), Some(d(7)))], d(4_000), Decimal::ZERO, swr());
        assert_eq!(out, RunwayOutcome::Months(Decimal::from(MAX_RUNWAY_MONTHS)));
    }

    /// El 5.º parámetro participa: con el gasto anual SIN grossear (12.000 ≤ 35.000) el caso es
    /// `Indefinite`, pero si el handler pasa un gasto grosseado mayor que el umbral (36.000 >
    /// 35.000) el mismo escenario pasa a finito. Fija que el umbral usa `annual_expense_for_swr`
    /// y no recalcula `12·monthly_expense` por su cuenta.
    #[test]
    fn grossed_expense_raises_threshold() {
        let assets = [(d(1_000_000), Some(d(7)))];
        let sin_gross = liquid_runway_months(&assets, d(1_000), Decimal::ZERO, swr(), d(12_000));
        assert_eq!(sin_gross, RunwayOutcome::Indefinite);

        let con_gross = liquid_runway_months(&assets, d(1_000), Decimal::ZERO, swr(), d(36_000));
        assert_eq!(
            con_gross,
            RunwayOutcome::Months(Decimal::from(MAX_RUNWAY_MONTHS)),
            "por encima del umbral y con retorno > gasto, el bucle agota el tope (suelo)"
        );
    }

    /// #146 (Ola 5): con inflación NEGATIVA el gasto DECRECE mes a mes (`g *= m_inf`, factor
    /// < 1) y el runway se ALARGA. 12.000 € sin rentabilidad, gasto 1.000 €/mes, i = −2 %:
    /// el bucle corta en k = 13 con total_12 = 12.000 − 1.000·(1 − m^12)/(1 − m) ≈ 110,40 y
    /// g_13 = 1.000·m^12 = 980 exacto (m^12 ≡ 0,98) → 12 + 110,40/980 ≈ **12,1126543321** meses
    /// (forma cerrada verificada a 50 dígitos). Hasta 4.8.0 el handler clampaba la inflación a
    /// ≥ 0 y publicaba 12,0 — el engine siempre supo componer hacia abajo.
    #[test]
    fn negative_inflation_lets_the_expense_shrink_and_extends_the_runway() {
        let out = months(runway(&[(d(12_000), None)], d(1_000), d(-2), swr()));
        assert!(out > d(12), "más que los 12 exactos de inflación 0, got {out}");
        assert!(
            out > d(12_112) / d(1_000) && out < d(12_113) / d(1_000),
            "esperado ≈12,1127 meses, got {out}"
        );
    }
}
