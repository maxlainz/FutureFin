//! Rendimiento neto anual esperado del patrimonio: lo que la cartera renta en un año según las
//! rentabilidades esperadas configuradas, **menos** lo que cuestan los intereses de la deuda,
//! sobre el patrimonio neto.
//!
//! Módulo puro (sin I/O, sin reloj, sin `f64`). Único consumidor: `GET /v1/summary`.

use rust_decimal::Decimal;

/// Rendimiento anual esperado del patrimonio neto, en **porcentaje** (5 = 5 %/año).
///
/// Los dos campos salen exactos del cálculo: el redondeo de publicación lo aplica la capa API
/// (mismo criterio que `runway_months`), nunca este módulo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetReturn {
    /// Rendimiento nominal: `100 · (Σ activos·r − Σ principales·apr) / patrimonio_neto`.
    pub nominal_pct: Decimal,
    /// El mismo rendimiento descontada la inflación, por **división de factores** (Fisher), no
    /// por resta: `100 · ((1 + nominal/100)/(1 + inflación/100) − 1)`.
    pub real_pct: Decimal,
}

/// Rendimiento neto anual esperado sobre el patrimonio neto.
///
/// `assets` son pares `(valor_actual, rentabilidad_anual_%)` y `liabilities` pares
/// `(principal, TIN_%)`; en ambos, `None` (rentabilidad o TIN sin configurar) cuenta como **0 %**
/// — no excluye la fila, que sigue pesando en el patrimonio neto del denominador. El caller es
/// quien decide qué filas entran: en `GET /v1/summary` son TODOS los activos del scope y los
/// pasivos **no vencidos**, es decir, exactamente los sumandos de `net_worth`.
///
/// # Modelo
///
/// - **Numerador en euros/año**: `Σ vₐ·rₐ/100 − Σ pₗ·aprₗ/100`. Los intereses de la deuda son un
///   lastre real del patrimonio, así que restan; una hipoteca al 3 % contrapesa la parte de la
///   cartera que renta ese 3 %.
/// - **Denominador**: el patrimonio neto `Σ vₐ − Σ pₗ`. Es lo que de verdad es tuyo, y por eso el
///   apalancamiento **amplifica** el resultado en ambos sentidos.
/// - **Sin patrimonio neto positivo no hay métrica**: con `NW ≤ 0` el cociente cambia de signo o
///   diverge y dejaría de significar «rendimiento». Se devuelve `None` en vez de un número que
///   se leería al revés.
/// - **La cifra real se obtiene dividiendo factores, no restando puntos**: `(1+n)/(1+i) − 1`. La
///   resta `n − i` es una aproximación que se desvía justo cuando más importa (tasas altas).
/// - **Es una expectativa, no un realizado**: sale de las rentabilidades que el usuario configuró
///   por activo, no del histórico. Y no incluye aportaciones: mide la cartera, no el ahorro.
///
/// Las tasas negativas se aceptan tal cual (una rentabilidad esperada de −3 % resta); no hay
/// composición mensual aquí — es una tasa anual sobre saldos actuales.
pub fn net_return_percentages(
    assets: &[(Decimal, Option<Decimal>)],
    liabilities: &[(Decimal, Option<Decimal>)],
    annual_inflation_percent: Decimal,
) -> Option<NetReturn> {
    let hundred = Decimal::from(100u32);

    let assets_total: Decimal = assets.iter().map(|(v, _)| *v).sum();
    let liabilities_total: Decimal = liabilities.iter().map(|(p, _)| *p).sum();
    let net_worth = assets_total - liabilities_total;
    if net_worth <= Decimal::ZERO {
        return None;
    }

    let asset_yield: Decimal = assets
        .iter()
        .map(|(v, r)| *v * r.unwrap_or(Decimal::ZERO) / hundred)
        .sum();
    let debt_cost: Decimal = liabilities
        .iter()
        .map(|(p, apr)| *p * apr.unwrap_or(Decimal::ZERO) / hundred)
        .sum();

    let nominal_pct = (asset_yield - debt_cost) * hundred / net_worth;

    // `(1 + n/100)/(1 + i/100)` reescrito como `(100 + n)/(100 + i)`: una división menos y el
    // mismo número. La inflación llega clampada a ≥ 0 desde la instalación, así que el
    // denominador es ≥ 100; la guarda es defensiva (el JSONB no se revalida al leer).
    let inflation_factor = hundred + annual_inflation_percent;
    let real_pct = if inflation_factor <= Decimal::ZERO {
        nominal_pct
    } else {
        ((hundred + nominal_pct) / inflation_factor - Decimal::ONE) * hundred
    };

    Some(NetReturn {
        nominal_pct,
        real_pct,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    /// Ponderación por valor: 100.000 al 5 % + 300.000 al 1 % sobre 400.000 € sin deuda da
    /// (5.000 + 3.000)/400.000 = **2 %**, no la media aritmética de las tasas (3 %).
    #[test]
    fn weights_by_value_not_by_asset_count() {
        let out = net_return_percentages(
            &[(d(100_000), Some(d(5))), (d(300_000), Some(d(1)))],
            &[],
            Decimal::ZERO,
        )
        .expect("NW positivo");
        assert_eq!(out.nominal_pct, d(2));
        // Sin inflación, real == nominal exacto.
        assert_eq!(out.real_pct, d(2));

        let media_simple = d(3);
        assert_ne!(out.nominal_pct, media_simple);
    }

    /// El interés de la deuda RESTA y el denominador es el patrimonio NETO:
    /// 100.000 al 5 % (=5.000) − 60.000 al 3 % (=1.800) = 3.200 sobre 40.000 → **8 %**.
    /// El apalancamiento amplifica: la misma cartera sin deuda rendiría 5 %.
    #[test]
    fn liability_interest_drags_and_leverage_amplifies() {
        let out = net_return_percentages(
            &[(d(100_000), Some(d(5)))],
            &[(d(60_000), Some(d(3)))],
            Decimal::ZERO,
        )
        .expect("NW positivo");
        assert_eq!(out.nominal_pct, d(8));
    }

    /// Deuda cara sobre cartera floja ⇒ rendimiento **negativo**, no cero.
    /// 100.000 al 1 % (=1.000) − 50.000 al 8 % (=4.000) = −3.000 sobre 50.000 → **−6 %**.
    #[test]
    fn expensive_debt_yields_negative_return() {
        let out = net_return_percentages(
            &[(d(100_000), Some(d(1)))],
            &[(d(50_000), Some(d(8)))],
            Decimal::ZERO,
        )
        .expect("NW positivo");
        assert_eq!(out.nominal_pct, d(-6));
    }

    /// `None` en la tasa cuenta como **0 %**, no excluye la fila: el activo sin rentabilidad
    /// configurada sigue pesando en el denominador y por tanto DILUYE.
    /// 100.000 al 5 % + 100.000 sin tasa = 5.000 sobre 200.000 → **2,5 %**.
    /// Ídem un pasivo sin TIN: pesa en el patrimonio neto y no cuesta nada.
    #[test]
    fn missing_rate_counts_as_zero_and_still_weighs() {
        let sin_tasa = net_return_percentages(
            &[(d(100_000), Some(d(5))), (d(100_000), None)],
            &[],
            Decimal::ZERO,
        )
        .expect("NW positivo");
        assert_eq!(sin_tasa.nominal_pct, Decimal::new(25, 1));

        // 100.000 al 5 % (=5.000) − 50.000 sin TIN (=0) sobre 50.000 → 10 %.
        let pasivo_sin_apr = net_return_percentages(
            &[(d(100_000), Some(d(5)))],
            &[(d(50_000), None)],
            Decimal::ZERO,
        )
        .expect("NW positivo");
        assert_eq!(pasivo_sin_apr.nominal_pct, d(10));
    }

    /// Una rentabilidad esperada negativa resta de verdad:
    /// 50.000 al 6 % (=3.000) + 50.000 al −4 % (=−2.000) = 1.000 sobre 100.000 → **1 %**.
    #[test]
    fn negative_expected_return_subtracts() {
        let out = net_return_percentages(
            &[(d(50_000), Some(d(6))), (d(50_000), Some(d(-4)))],
            &[],
            Decimal::ZERO,
        )
        .expect("NW positivo");
        assert_eq!(out.nominal_pct, d(1));
    }

    /// Patrimonio neto ≤ 0 ⇒ métrica ausente, no un número con el signo cambiado.
    #[test]
    fn non_positive_net_worth_has_no_metric() {
        // Deuda mayor que los activos.
        assert!(net_return_percentages(
            &[(d(50_000), Some(d(5)))],
            &[(d(80_000), Some(d(3)))],
            Decimal::ZERO
        )
        .is_none());
        // Exactamente 0: el cociente divergiría.
        assert!(net_return_percentages(
            &[(d(50_000), Some(d(5)))],
            &[(d(50_000), Some(d(3)))],
            Decimal::ZERO
        )
        .is_none());
        // Sin nada registrado.
        assert!(net_return_percentages(&[], &[], Decimal::ZERO).is_none());
    }

    /// Con inflación 0 el real es EXACTAMENTE el nominal (la división es por 1, sin residuo).
    #[test]
    fn zero_inflation_leaves_real_equal_to_nominal() {
        let out = net_return_percentages(&[(d(100_000), Some(d(7)))], &[], Decimal::ZERO)
            .expect("NW positivo");
        assert_eq!(out.real_pct, out.nominal_pct);
        assert_eq!(out.real_pct, d(7));
    }

    /// El real se obtiene DIVIDIENDO factores, no restando puntos: con nominal 7 % e inflación
    /// 2 %, `1,07/1,02 − 1 = 4,90196…%`, estrictamente **menor** que el 5 % de la resta simple.
    #[test]
    fn real_divides_factors_instead_of_subtracting_points() {
        let out = net_return_percentages(&[(d(100_000), Some(d(7)))], &[], d(2))
            .expect("NW positivo");

        let esperado = (Decimal::new(107, 2) / Decimal::new(102, 2) - Decimal::ONE) * d(100);
        assert_eq!(out.real_pct, esperado);

        let resta_simple = d(5);
        assert!(
            out.real_pct < resta_simple,
            "la división de factores debe quedar por debajo de la resta, obtenido {}",
            out.real_pct
        );
        assert_eq!(out.real_pct.round_dp(4), Decimal::new(49020, 4));
    }

    /// Caso trabajado de la documentación: 100.000 al 5 % + 50.000 al 0 %, hipoteca de 60.000 al
    /// 3 %, inflación 2 %. Numerador 5.000 − 1.800 = 3.200; patrimonio neto 90.000.
    /// Nominal = 3,5555…% (3,5556 publicado); real = 1,52505…% (1,5251 publicado).
    #[test]
    fn worked_example_matches_the_documented_figures() {
        let out = net_return_percentages(
            &[(d(100_000), Some(d(5))), (d(50_000), Some(Decimal::ZERO))],
            &[(d(60_000), Some(d(3)))],
            d(2),
        )
        .expect("NW positivo");

        assert_eq!(out.nominal_pct, d(3_200) * d(100) / d(90_000));
        assert_eq!(out.nominal_pct.round_dp(4), Decimal::new(35556, 4));
        assert_eq!(out.real_pct.round_dp(4), Decimal::new(15251, 4));
    }
}
