//! **6.0.0 F1c — Monte Carlo con `DeficitPolicy::SinkOnly` (D-9).** El éxito de un camino es no
//! tener NINGÚN mes de gasto sin cubrir en todo el horizonte, en cualquier fase. Con volatilidad
//! 0 todos los caminos son el determinista, así que el éxito tiene que ser 0 o 1 y coincidir con
//! el veredicto del camino `f64` (y del `Decimal`).

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset_with_basis, projection_cases_6_0_sales, rule_remainder};
use futurefin_engine::{project_net_worth_series, DeficitPolicy, PathFailure};
use futurefin_engine_stochastic::{project_percentile_bands, simulate_f64, McConfig};
use rust_decimal::Decimal;

fn config() -> McConfig {
    McConfig {
        seed: 7,
        paths: 40,
        threads: Some(1),
        ..McConfig::default()
    }
}

/// Predicción: S2 (sumidero agotado en el mes 35 en ACUMULACIÓN) ⇒ los 40 caminos fallan:
/// éxito 0, `uncovered_free_probability` 0 y los 40 fallos en la casilla de F1.
#[test]
fn zero_volatility_sink_only_failure_is_certain() {
    let s2 = projection_cases_6_0_sales().remove(1);
    assert_eq!(s2.input.deficit_policy, DeficitPolicy::SinkOnly);
    let det = simulate_f64(&s2.input).unwrap();
    assert_eq!(det.first_uncovered_month, Some(35));
    assert_eq!(det.failure_kind, Some(PathFailure::UncoveredMonth));
    assert_eq!(
        project_net_worth_series(&s2.input)
            .unwrap()
            .failure_month_index,
        Some(35)
    );

    let vols = vec![None; s2.input.assets.len()];
    let mc = project_percentile_bands(&s2.input, &vols, &config()).unwrap();
    assert_eq!(mc.deficit_policy, DeficitPolicy::SinkOnly);
    assert_eq!(mc.success_probability, 0.0);
    assert_eq!(mc.uncovered_free_probability, 0.0);
    assert_eq!(mc.failures_by_kind, [40, 0]);
}

/// Predicción: el mismo hogar con un sumidero de 200.000 € (déficit 1.000 €/mes × 120 meses =
/// 120.000) nunca deja un mes sin cubrir ⇒ éxito 1 y `uncovered_free_probability` 1.
#[test]
fn zero_volatility_sink_only_success_is_certain() {
    let mut input = base_input(
        120,
        Decimal::from(2_000),
        Decimal::from(3_000),
        vec![mk_asset_with_basis(
            1,
            Decimal::from(200_000),
            true,
            None,
            Decimal::from(200_000),
        )],
        vec![rule_remainder(0)],
    );
    input.deficit_policy = DeficitPolicy::SinkOnly;
    let det = simulate_f64(&input).unwrap();
    assert_eq!(det.failure_month_index, None);
    let mc = project_percentile_bands(&input, &[None], &config()).unwrap();
    assert_eq!(mc.success_probability, 1.0);
    assert_eq!(mc.uncovered_free_probability, 1.0);
    assert_eq!(mc.failures_by_kind, [0, 0]);
    // Decisión del owner: en SinkOnly la puerta de tasa inicial no firma. Con una puerta SWR 1 %
    // que el plan incumple de largo (12·1.000 > 1 % · 200.000) y un gasto jubilado de 1.000 €/mes
    // que el sumidero cubre (120.000 < 200.000), el éxito sigue siendo 1 (= ningún mes sin
    // cubrir); en LegacyDrain el mismo plan falla en el mes 1 con InitialRateExceeded.
    input.phase_plan = futurefin_engine::PhasePlan::forced_at(
        1,
        Decimal::ZERO,
        Decimal::from(1_000),
        Decimal::ZERO,
    );
    input.income_regular_monthly = Decimal::ZERO;
    input.phase_plan.initial_rate = Some(futurefin_engine::InitialRateGate {
        swr_pct: Decimal::ONE,
        bridge: None,
    });
    let mc = project_percentile_bands(&input, &[None], &config()).unwrap();
    assert_eq!(mc.success_probability, 1.0);
    assert_eq!(mc.success_probability, mc.uncovered_free_probability);
    input.deficit_policy = DeficitPolicy::LegacyDrain;
    let lg = project_percentile_bands(&input, &[None], &config()).unwrap();
    assert_eq!(lg.success_probability, 0.0);
    assert_eq!(lg.failures_by_kind, [0, 40]);
}

/// Predicción: en `LegacyDrain` el campo nuevo es una lectura y no mueve `success_probability`:
/// S2 en legacy no falla (acumulando, puerta de fase) pero su lectura de meses sin cubrir sí.
/// Legacy vende el fondo no sumidero, así que el primer mes sin cubrir llega más tarde o nunca;
/// lo que se exige es coherencia con el camino determinista.
#[test]
fn legacy_reads_uncovered_months_without_changing_success() {
    let mut s2 = projection_cases_6_0_sales().remove(1);
    s2.input.deficit_policy = DeficitPolicy::LegacyDrain;
    let det = simulate_f64(&s2.input).unwrap();
    assert_eq!(det.failure_month_index, None);
    let vols = vec![None; s2.input.assets.len()];
    let mc = project_percentile_bands(&s2.input, &vols, &config()).unwrap();
    assert_eq!(mc.success_probability, 1.0);
    let expected = if det.first_uncovered_month.is_some() {
        0.0
    } else {
        1.0
    };
    assert_eq!(mc.uncovered_free_probability, expected);
}
