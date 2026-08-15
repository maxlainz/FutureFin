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
    /// aplicado al saldo líquido: `annual_expense_for_swr ≤ A·(swr_pct/100)`.
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
/// - **Multiplicador ponderado**: se usa una media de los multiplicadores mensuales ponderada por
///   valor, `m = Σ vₐ·monthly_multiplier(rₐ) / Σ vₐ`. Equivale a un **drenaje prorrateado** (cada
///   activo aporta al gasto en proporción a su peso), ligeramente **conservador** frente al drain
///   real del engine, que vacía primero los líquidos de menor rentabilidad y por tanto conserva
///   más tiempo los de rentabilidad alta.
/// - **Tasas ≤ 0 → crecimiento 0**: herencia documentada de [`monthly_multiplier`], que devuelve
///   factor 1 tanto para `None` como para tasas no positivas (el engine no modela pérdidas).
/// - **Umbral SWR (caso infinito)**: el runway es `Indefinite` ⟺ la retirada anual no supera el
///   SWR sobre el saldo inicial: `annual_expense_for_swr ≤ A·(swr_pct/100)`. Se compara sin
///   dividir — `annual_expense_for_swr·100 ≤ A·swr_pct` — para que la frontera sea **exacta** en
///   `Decimal`. Con `swr_pct ≤ 0` el lado derecho es ≤ 0 y el izquierdo > 0, así que nunca se
///   cumple (no necesita guarda aparte). Es el «FIRE number» de liquidez: `A ≥ gasto_bruto/SWR`.
/// - **El disparador es deliberadamente independiente de rentabilidad e inflación**: mira solo
///   `A`, el gasto grosseado y el SWR — la definición de SWR ya asume una cartera cuyo retorno
///   real sostiene esa retirada a largo plazo. Rentabilidad e inflación siguen gobernando el
///   caso **finito** (el bucle de abajo). Consecuencia asumida: un saldo por debajo del umbral
///   con rentabilidad altísima ya no es «infinito», y uno en el umbral con rentabilidad 0 sí.
/// - **Orden de checks**: `NoExpenseBase` se decide **antes** que el umbral SWR — con gasto 0 la
///   desigualdad `0 ≤ A·swr` sería trivialmente cierta y marcaría infinito un runway indefinido.
/// - **Tope de 100 años**: si el saldo aguanta `MAX_RUNWAY_MONTHS` meses sin haber cumplido el
///   umbral SWR se devuelve `Months(1200)` como **suelo** («al menos 100 años»), no `Indefinite`:
///   el infinito lo decide en exclusiva el SWR.
///
/// # Reducción exacta a `A / g`
///
/// Cuando el umbral SWR no se cumple, con rentabilidad e inflación 0 se tiene `m = m_inf = 1`,
/// luego `balance_k = A − k·g` y `g` constante. Sea `n = ⌊A/g⌋`: para todo `k ≤ n` se cumple
/// `balance_{k−1} = A − (k−1)·g ≥ g`, así que el bucle no corta; en `k = n+1` se cumple
/// `balance_n = A − n·g < g` y se devuelve `n + (A − n·g)/g = A/g`. La única división es la
/// final, de modo que el resultado es la división simple exacta (regresión sin tolerancias en
/// los tests).
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

    // Infinito ⟺ la retirada anual (grosseada) no supera el SWR. Comparación exacta sin dividir.
    // OJO: el `100` desporcentúa `swr_pct` — no confundir ni compartir con `MAX_RUNWAY_MONTHS`
    // aunque 12·100 = 1200 coincidan numéricamente.
    if annual_expense_for_swr * Decimal::from(100u32) <= balance_0 * swr_pct {
        return RunwayOutcome::Indefinite;
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

        let out = runway(
            &[(v_lento, None), (v_rapido, Some(d(10)))],
            d(2_000),
            Decimal::ZERO,
            swr(),
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
        let negativo = runway(&[(d(12_000), Some(d(-5)))], d(1_000), Decimal::ZERO, swr());
        let sin_tasa = runway(&[(d(12_000), None)], d(1_000), Decimal::ZERO, swr());
        assert_eq!(negativo, sin_tasa);
        assert_eq!(negativo, RunwayOutcome::Months(d(12)));
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

    /// Frontera EXACTA del umbral: 300.000 al 4 % = 12.000 €/año = 12 × 1.000 → `Indefinite`
    /// por igualdad (la comparación sin división lo hace posible en `Decimal`). Nota: sin
    /// rentabilidad el saldo se agotaría en 300 meses — el KPI mide tasa de retirada, no
    /// supervivencia (decisión de diseño documentada en el docstring del módulo).
    #[test]
    fn swr_threshold_exact_equality_is_indefinite() {
        let out = runway(&[(d(300_000), None)], d(1_000), Decimal::ZERO, d(4));
        assert_eq!(out, RunwayOutcome::Indefinite);
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
}
