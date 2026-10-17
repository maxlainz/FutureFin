//! **Déficit por el sumidero (D-29) y ventas explícitas (D-10/D-21)** — 6.0.0 F1c, `sales.rs`.
//!
//! Cada test declara su predicción ANTES de los asserts (futurefin-research-methodology). Los
//! hogares son de laboratorio: rentabilidades nulas e inflación 0 salvo donde se mide la
//! indexación, para que cada euro de la salida se pueda reconstruir a mano.

#[path = "common/cases.rs"]
mod cases;

use cases::{
    base_input, mk_asset, mk_asset_with_basis, projection_cases_6_0_sales, rule_fixed,
    rule_remainder,
};
use futurefin_engine::{
    build_asset_sale_orders, first_month_allocation, gross_up_monthly,
    inflation_factor_at_month_index, project_net_worth_series, AssetSaleKind, AssetSaleSpec,
    DeficitPolicy, EngineError, InitialRateGate, PathFailure, PhasePlan, ProjectionInput,
    SaleTaxMode, TaxBracket,
};
use rust_decimal::Decimal;
use std::str::FromStr;

fn eur(n: i64) -> Decimal {
    Decimal::from(n)
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn flat_21() -> Vec<TaxBracket> {
    vec![TaxBracket {
        up_to: None,
        pct: eur(21),
    }]
}

fn assert_close(got: Decimal, want: Decimal, tol: &str, what: &str) {
    assert!((got - want).abs() <= dec(tol), "{what}: {got} vs {want}");
}

/// Un hogar con un único activo sumidero (`rule_remainder(0)`), sin rentabilidad ni inflación.
fn one_sink(
    horizon: u32,
    income: i64,
    expense: i64,
    value: i64,
    purchase: i64,
    policy: DeficitPolicy,
) -> ProjectionInput {
    let mut input = base_input(
        horizon,
        eur(income),
        eur(expense),
        vec![mk_asset_with_basis(
            1,
            eur(value),
            true,
            None,
            eur(purchase),
        )],
        vec![rule_remainder(0)],
    );
    input.tax_brackets = flat_21();
    input.taxes_enabled = true;
    input.deficit_policy = policy;
    input
}

// ---------------------------------------------------------------------------------------------
// Legacy intacto
// ---------------------------------------------------------------------------------------------

/// Predicción: el default es `LegacyDrain` y sin ventas los apéndices de F1c salen vacíos.
/// (La bit-identidad de las cifras la pinean `golden_pins.rs` sin regenerar.)
#[test]
fn default_is_legacy_and_publishes_empty_appendices() {
    let input = one_sink(12, 1_000, 3_000, 100_000, 100_000, DeficitPolicy::default());
    assert_eq!(input.deficit_policy, DeficitPolicy::LegacyDrain);
    let out = project_net_worth_series(&input).unwrap();
    assert!(out.deficit_coverage_net.is_empty());
    assert!(out.deficit_coverage_tax.is_empty());
    assert!(out.explicit_sale_net.is_empty());
    assert!(out.explicit_sale_tax.is_empty());
    assert!(out.explicit_sale_exhausted.is_empty());
    assert_eq!(out.first_uncovered_month, None);
}

// ---------------------------------------------------------------------------------------------
// SinkOnly
// ---------------------------------------------------------------------------------------------

/// Sumidero de efectivo (precio de compra = valor ⇒ g = 0), déficit 2.000 €/mes.
/// Predicción: cobertura = 2.000 EXACTO cada mes, impuesto 0, valor tras 12 meses = 76.000,
/// ningún mes sin cubrir y ningún fallo.
#[test]
fn sink_only_cash_sink_covers_the_deficit_tax_free() {
    let input = one_sink(12, 1_000, 3_000, 100_000, 100_000, DeficitPolicy::SinkOnly);
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.deficit_coverage_net.len(), 13);
    for k in 1..=12 {
        assert_eq!(out.deficit_coverage_net[k], eur(2_000), "mes {k}");
        assert_eq!(out.deficit_coverage_tax[k], Decimal::ZERO, "mes {k}");
        assert_eq!(out.unmet_need[k], Decimal::ZERO, "mes {k}");
    }
    assert_eq!(out.per_asset_series[0][12], eur(76_000));
    assert_eq!(out.first_uncovered_month, None);
    assert_eq!(out.failure_month_index, None);
}

/// Sumidero con 50 % de plusvalía latente (valor 100.000, compra 50.000 ⇒ g = 0,5) y un tramo
/// plano del 21 %. Para un neto N = 2.000: G·(1 − 0,21·0,5) = N ⇒ G = 2.000/0,895 =
/// 2.234,636871508379888268156424… €, impuesto ≈ 234,64 €. Predicción: el impuesto publicado es
/// EXACTAMENTE `gross_up_monthly(2.000, tramos, g)` − 2.000 (la función del motor) y está a menos
/// de 1e-20 € de la forma cerrada; el valor del mes 1 es 100.000 − G.
#[test]
fn sink_only_taxes_the_sink_gain_with_the_engine_gross_up() {
    let input = one_sink(3, 1_000, 3_000, 100_000, 50_000, DeficitPolicy::SinkOnly);
    let out = project_net_worth_series(&input).unwrap();
    let g_engine = gross_up_monthly(eur(2_000), &flat_21(), true, dec("0.5"));
    let g_closed = eur(2_000) / dec("0.895");
    assert_close(
        g_engine,
        g_closed,
        "0.00000000000000000001",
        "forma cerrada",
    );
    assert_eq!(out.deficit_coverage_net[1], eur(2_000));
    assert_eq!(out.deficit_coverage_tax[1], g_engine - eur(2_000));
    assert_close(
        out.deficit_coverage_tax[1],
        dec("234.636871508379888268"),
        "0.000000000000000001",
        "impuesto",
    );
    assert_eq!(out.per_asset_series[0][1], eur(100_000) - g_engine);
    // La base baja en proporción ⇒ g sigue en 0,5 (hasta el redondeo): el mes 2 tributa igual.
    assert_close(
        out.deficit_coverage_tax[2],
        out.deficit_coverage_tax[1],
        "0.0000000000000001",
        "mes 2",
    );
}

/// Sumidero de 10.000 € sin plusvalía, déficit 3.000 €/mes, hogar ACUMULANDO (sin objetivo FIRE
/// ni jubilación forzada). Predicción: meses 1–3 cubiertos (quedan 1.000), el 4 cubre 1.000 y
/// deja 2.000 sin cubrir, el 5 deja 3.000. `first_uncovered_month = 4` y el camino FALLA en el 4
/// con `UncoveredMonth` aunque la fase sea de acumulación. En `LegacyDrain` el mismo hogar no
/// falla (puerta de fase), pero la lectura `first_uncovered_month` también es 4.
#[test]
fn sink_only_exhausted_sink_is_a_failure_even_while_accumulating() {
    let input = one_sink(8, 1_000, 4_000, 10_000, 10_000, DeficitPolicy::SinkOnly);
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.retirement_month_index, None, "nunca se jubila");
    assert_eq!(out.deficit_coverage_net[3], eur(3_000));
    assert_eq!(out.deficit_coverage_net[4], eur(1_000));
    assert_eq!(out.unmet_need[4], eur(2_000));
    assert_eq!(out.unmet_need[5], eur(3_000));
    assert_eq!(out.first_uncovered_month, Some(4));
    assert_eq!(out.failure_month_index, Some(4));
    assert_eq!(out.failure_kind, Some(PathFailure::UncoveredMonth));
    assert_eq!(out.failure_kind.unwrap().code(), "uncovered_month");

    let mut legacy = input.clone();
    legacy.deficit_policy = DeficitPolicy::LegacyDrain;
    let lo = project_net_worth_series(&legacy).unwrap();
    assert_eq!(lo.failure_month_index, None, "legacy: fallo solo jubilado");
    assert_eq!(
        lo.first_uncovered_month,
        Some(4),
        "legacy: la lectura existe"
    );
}

/// Casa ilíquida + fondo líquido NO sumidero + cuenta sumidero de 5.000 €, déficit 2.000 €/mes.
/// Predicción (SinkOnly): la cuenta cubre los meses 1–2, el 3 cubre 1.000 y deja 1.000 sin
/// cubrir; el fondo (50.000) y la casa (300.000) NO se tocan ningún mes. En `LegacyDrain` el
/// drenaje sí vende el fondo desde el mes 3 (líquido, empate de rentabilidad ⇒ por índice).
#[test]
fn sink_only_never_touches_non_sink_assets() {
    let mut input = base_input(
        12,
        eur(1_000),
        eur(3_000),
        vec![
            mk_asset(1, eur(5_000), true, None),
            mk_asset(2, eur(50_000), true, None),
            mk_asset(3, eur(300_000), false, None),
        ],
        vec![rule_remainder(0)],
    );
    input.deficit_policy = DeficitPolicy::SinkOnly;
    let out = project_net_worth_series(&input).unwrap();
    assert!(out.per_asset_series[1].iter().all(|v| *v == eur(50_000)));
    assert!(out.per_asset_series[2].iter().all(|v| *v == eur(300_000)));
    assert_eq!(out.unmet_need[3], eur(1_000));
    assert_eq!(out.first_uncovered_month, Some(3));

    input.deficit_policy = DeficitPolicy::LegacyDrain;
    let lo = project_net_worth_series(&input).unwrap();
    assert_eq!(lo.per_asset_series[1][3], eur(49_000));
    assert_eq!(
        lo.per_asset_series[2][12],
        eur(300_000),
        "la casa va la última"
    );
}

/// Decisión del owner (D-10): un sumidero ILÍQUIDO no se vende para cubrir un déficit. Casa
/// ilíquida de 300.000 € como destino del sobrante, déficit 1.000 €/mes. Predicción (SinkOnly):
/// la casa sigue en 300.000 todos los meses, el mes 1 deja 1.000 € sin cubrir y el camino falla
/// en el 1 con `UncoveredMonth`.
#[test]
fn sink_only_never_sells_an_illiquid_sink() {
    let mut input = base_input(
        6,
        eur(1_000),
        eur(2_000),
        vec![mk_asset(1, eur(300_000), false, None)],
        vec![rule_remainder(0)],
    );
    input.deficit_policy = DeficitPolicy::SinkOnly;
    let out = project_net_worth_series(&input).unwrap();
    assert!(out.per_asset_series[0].iter().all(|v| *v == eur(300_000)));
    assert_eq!(out.deficit_coverage_net[1], Decimal::ZERO);
    assert_eq!(out.unmet_need[1], eur(1_000));
    assert_eq!(out.first_uncovered_month, Some(1));
    assert_eq!(out.failure_month_index, Some(1));
    assert_eq!(out.failure_kind, Some(PathFailure::UncoveredMonth));
}

/// Decisión del owner: en SinkOnly el ÚNICO fallo es el mes sin cubrir; la puerta de tasa
/// inicial no firma. Jubilado desde el mes 1 sin ingresos, gasto 3.000 €/mes, sumidero líquido de
/// 100.000 y puerta SWR 4 %: 36.000 > 4.000 ⇒ Predicción: Legacy falla en el mes 1 con
/// `InitialRateExceeded`; SinkOnly no falla (el sumidero cubre los 12 meses).
#[test]
fn sink_only_ignores_the_initial_rate_gate() {
    let mut input = base_input(
        12,
        Decimal::ZERO,
        eur(3_000),
        vec![mk_asset(1, eur(100_000), true, None)],
        vec![rule_remainder(0)],
    );
    let mut plan = PhasePlan::forced_at(1, Decimal::ZERO, eur(3_000), Decimal::ZERO);
    plan.initial_rate = Some(InitialRateGate {
        swr_pct: eur(4),
        bridge: None,
    });
    input.phase_plan = plan;
    let legacy = project_net_worth_series(&input).unwrap();
    assert_eq!(legacy.failure_month_index, Some(1));
    assert_eq!(legacy.failure_kind, Some(PathFailure::InitialRateExceeded));

    input.deficit_policy = DeficitPolicy::SinkOnly;
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.failure_month_index, None);
    assert_eq!(out.failure_kind, None);
    assert_eq!(out.per_asset_series[0][12], eur(64_000));
}

/// Sin regla `Remainder` sin tope no hay sumidero: todo déficit queda sin cubrir desde el mes 1.
#[test]
fn sink_only_without_sink_leaves_every_deficit_uncovered() {
    let mut input = one_sink(4, 1_000, 2_000, 50_000, 50_000, DeficitPolicy::SinkOnly);
    input.allocation_rules = vec![];
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.first_uncovered_month, Some(1));
    assert_eq!(out.unmet_need[1], eur(1_000));
    assert_eq!(out.per_asset_series[0][4], eur(50_000));
}

// ---------------------------------------------------------------------------------------------
// Ventas explícitas
// ---------------------------------------------------------------------------------------------

/// Hogar con superávit (ingreso 4.000, gasto 3.000 indexado al 2 %), cuenta sumidero 10.000 y
/// fondo 200.000 con compra 100.000 (g = 0,5), tramo plano 21 %, inflación 2 %. Venta de
/// 2.000 €/mes netos de hoy del fondo desde el mes 13, indexada.
fn sale_household(tax_mode: SaleTaxMode<Decimal>, kind: AssetSaleKind<Decimal>) -> ProjectionInput {
    let mut input = base_input(
        24,
        eur(4_000),
        eur(3_000),
        vec![
            mk_asset_with_basis(1, eur(10_000), true, None, eur(10_000)),
            mk_asset_with_basis(2, eur(200_000), true, None, eur(100_000)),
        ],
        vec![rule_remainder(0)],
    );
    input.annual_inflation_percent = eur(2);
    input.tax_brackets = flat_21();
    input.taxes_enabled = true;
    input.asset_sales = build_asset_sale_orders(
        &[AssetSaleSpec {
            asset_index: 1,
            start_month: 13,
            end_month: None,
            kind,
            keeps_purchasing_power: true,
            tax_mode,
        }],
        24,
        input.annual_inflation_percent,
    );
    input
}

const MONTHLY_2000: AssetSaleKind<Decimal> = AssetSaleKind::MonthlyNet {
    amount_today: Decimal::from_parts(2_000, 0, 0, false, 0),
};

/// Predicción (Auto): meses 1–12 sin venta (0); en el 13, `f(13) = 1,02` exacto ⇒ N = 2.040,
/// G = gross_up_monthly(2.040, 21 %, 0,5) ≈ 2.040/0,895 = 2.279,33 €, impuesto G − N ≈ 239,33,
/// fondo = 200.000 − G. En el 14, N = 2.000·f(14). El neto entra en caja (superávit) y la
/// cascada lo manda al sumidero: el sumidero sube N además del sobrante ordinario.
#[test]
fn explicit_monthly_sale_auto_indexed() {
    for policy in [DeficitPolicy::LegacyDrain, DeficitPolicy::SinkOnly] {
        let mut input = sale_household(SaleTaxMode::Auto, MONTHLY_2000);
        input.deficit_policy = policy;
        let out = project_net_worth_series(&input).unwrap();
        for k in 1..=12 {
            assert_eq!(
                out.explicit_sale_net[k],
                Decimal::ZERO,
                "{policy:?} mes {k}"
            );
        }
        let g13 = gross_up_monthly(eur(2_040), &flat_21(), true, dec("0.5"));
        assert_eq!(out.explicit_sale_net[13], eur(2_040), "{policy:?}");
        assert_eq!(out.explicit_sale_tax[13], g13 - eur(2_040), "{policy:?}");
        assert_close(
            g13,
            eur(2_040) / dec("0.895"),
            "0.00000000000000000001",
            "G13",
        );
        assert_eq!(
            out.per_asset_series[1][13],
            eur(200_000) - g13,
            "{policy:?}"
        );
        let n14 = eur(2_000) * inflation_factor_at_month_index(eur(2), 13);
        assert_eq!(out.explicit_sale_net[14], n14, "{policy:?}");
        assert!(out.explicit_sale_exhausted.is_empty());
        // El sumidero recibe el sobrante ordinario + el neto: 4.000 − 3.000·f(13) + 2.040.
        let surplus13 = eur(4_000) - eur(3_000) * dec("1.02") + eur(2_040);
        assert_eq!(
            out.per_asset_series[0][13] - out.per_asset_series[0][12],
            surplus13,
            "{policy:?}"
        );
    }
}

/// Predicción (Exempt): impuesto 0 todos los meses, bruto = neto ⇒ fondo(13) = 200.000 − 2.040.
#[test]
fn explicit_monthly_sale_exempt_pays_no_tax() {
    let out = project_net_worth_series(&sale_household(SaleTaxMode::Exempt, MONTHLY_2000)).unwrap();
    assert!(out.explicit_sale_tax.iter().all(|t| t.is_zero()));
    assert_eq!(out.per_asset_series[1][13], eur(200_000) - eur(2_040));
}

/// Predicción (ManualPct 10 %): G = 2.040/0,9 = 2.266,66… ⇒ impuesto = G − N = G·0,1 = 226,66…
#[test]
fn explicit_monthly_sale_manual_pct_pays_pct_of_gross() {
    let out = project_net_worth_series(&sale_household(
        SaleTaxMode::ManualPct { pct: eur(10) },
        MONTHLY_2000,
    ))
    .unwrap();
    let g = eur(2_040) / dec("0.9");
    assert_close(
        out.explicit_sale_tax[13],
        g * dec("0.1"),
        "0.0000000000000000001",
        "10 %",
    );
    assert_close(
        out.explicit_sale_tax[13],
        dec("226.666666666666666667"),
        "0.000000000000000001",
        "cifra",
    );
    assert_eq!(out.explicit_sale_net[13], eur(2_040));
}

/// Predicción (ManualAmount 75 € por mes): bruto = N + 75, impuesto 75 cada mes con venta.
#[test]
fn explicit_monthly_sale_manual_amount_is_a_nominal_tax_per_month() {
    let out = project_net_worth_series(&sale_household(
        SaleTaxMode::ManualAmount { tax: eur(75) },
        MONTHLY_2000,
    ))
    .unwrap();
    for k in 13..=24 {
        assert_eq!(out.explicit_sale_tax[k], eur(75), "mes {k}");
    }
    assert_eq!(
        out.per_asset_series[1][13],
        eur(200_000) - eur(2_040) - eur(75)
    );
}

/// Predicción (Whole, Auto): en el mes 13 se vende el fondo entero, G = 200.000; la venta es UN
/// hecho del año ⇒ impuesto = 21 % · (0,5 · 200.000) = 21.000, neto 179.000. Fondo = 0 desde el
/// 13; el neto entra en caja y va al sumidero.
#[test]
fn explicit_whole_sale_empties_the_asset() {
    let out =
        project_net_worth_series(&sale_household(SaleTaxMode::Auto, AssetSaleKind::Whole)).unwrap();
    assert_eq!(out.explicit_sale_tax[13], eur(21_000));
    assert_eq!(out.explicit_sale_net[13], eur(179_000));
    for k in 13..=24 {
        assert_eq!(out.per_asset_series[1][k], Decimal::ZERO, "mes {k}");
    }
}

/// Venta mensual exenta de 50.000 €/mes de un fondo de 120.000 sin rentabilidad. Predicción:
/// meses 1–2 venden 50.000 (quedan 20.000), el 3 vende los 20.000 y la orden queda AGOTADA
/// (`(1, 3)`), el 4 no vende nada. Un registro por activo.
#[test]
fn explicit_sale_stops_when_the_asset_runs_out() {
    let mut input = base_input(
        6,
        eur(3_000),
        eur(3_000),
        vec![
            mk_asset(1, eur(1_000), true, None),
            mk_asset(2, eur(120_000), true, None),
        ],
        vec![rule_remainder(0)],
    );
    input.asset_sales = build_asset_sale_orders(
        &[AssetSaleSpec {
            asset_index: 1,
            start_month: 1,
            end_month: None,
            kind: AssetSaleKind::MonthlyNet {
                amount_today: eur(50_000),
            },
            keeps_purchasing_power: false,
            tax_mode: SaleTaxMode::Exempt,
        }],
        6,
        Decimal::ZERO,
    );
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.explicit_sale_net[2], eur(50_000));
    assert_eq!(out.explicit_sale_net[3], eur(20_000));
    assert_eq!(out.explicit_sale_net[4], Decimal::ZERO);
    assert_eq!(out.explicit_sale_exhausted, vec![(1, 3)]);
    // El patrimonio no cambia por vender sin impuesto: el neto acaba en el sumidero.
    assert_eq!(out.net_worth[6], eur(121_000));
    // El mes 1 lo ve también `first_month_allocation`: la venta está dentro de `base_cash`.
    let fma = first_month_allocation(&input).unwrap();
    assert_eq!(fma.explicit_sales_net, eur(50_000));
    assert_eq!(fma.base_cash, eur(50_000));
}

/// Una orden fuera de rango es un error tipado, en el bucle y en el mes 1.
#[test]
fn invalid_sale_orders_are_rejected() {
    let mut input = sale_household(SaleTaxMode::Auto, MONTHLY_2000);
    input.asset_sales.orders[0].asset_index = 7;
    assert_eq!(
        project_net_worth_series(&input).unwrap_err(),
        EngineError::InvalidAssetSales
    );
    assert_eq!(
        first_month_allocation(&input).unwrap_err(),
        EngineError::InvalidAssetSales
    );
}

/// Los casos de la batería S1–S3 corren y cumplen la identidad del mes en `SinkOnly`:
/// cobertura + no cubierto = déficit neto. Predicción: S2 (sumidero 30.000, déficit 1.000, 400
/// exentos/mes de 1 a 12) cubre hasta el mes 34 y el 35 es el primero sin cubrir (200 €).
#[test]
fn battery_cases_run_and_s2_fails_where_predicted() {
    let cases = projection_cases_6_0_sales();
    for c in &cases {
        project_net_worth_series(&c.input).unwrap();
    }
    let s2 = project_net_worth_series(&cases[1].input).unwrap();
    assert_eq!(s2.deficit_coverage_net[12], eur(600));
    assert_eq!(s2.deficit_coverage_net[34], eur(1_000));
    assert_eq!(s2.unmet_need[35], eur(200));
    assert_eq!(s2.first_uncovered_month, Some(35));
    assert_eq!(s2.failure_kind, Some(PathFailure::UncoveredMonth));
    let after_sales = s2.per_asset_series[1][12];
    assert!(
        s2.per_asset_series[1]
            .iter()
            .skip(13)
            .all(|v| *v > after_sales),
        "D-10: tras sus ventas el fondo no sumidero solo crece, aunque haya meses sin cubrir"
    );
}

// ---------------------------------------------------------------------------------------------
// Regresiones de la revisión adversarial de F1c (B1–B4)
// ---------------------------------------------------------------------------------------------

/// Hogar de B1: ingreso 3.000, gasto 2.000 (ahorro 1.000/mes), fondo líquido 10.000 sumidero
/// (`Remainder` sin tope), casa ilíquida 300.000, techo de aportación 500 €/mes, venta `Whole`
/// exenta de la casa en `sale_month`. Sin rentabilidades ni inflación.
fn b1_household(sale_month: u32, policy: DeficitPolicy) -> ProjectionInput {
    let mut input = base_input(
        6,
        eur(3_000),
        eur(2_000),
        vec![
            mk_asset(1, eur(10_000), true, None),
            mk_asset(2, eur(300_000), false, None),
        ],
        vec![rule_remainder(0)],
    );
    input.phase_plan.contribution_cap_monthly = Some(eur(500));
    input.deficit_policy = policy;
    input.asset_sales = build_asset_sale_orders(
        &[AssetSaleSpec {
            asset_index: 1,
            start_month: sale_month,
            end_month: None,
            kind: AssetSaleKind::Whole,
            keeps_purchasing_power: false,
            tax_mode: SaleTaxMode::Exempt,
        }],
        6,
        Decimal::ZERO,
    );
    input
}

/// B1: el neto de una venta explícita NO pasa por el techo de aportación — es un cambio de
/// composición del patrimonio, no ahorro nuevo.
///
/// Predicción (derivación a mano): cada mes el ahorro es 3.000 − 2.000 = 1.000; el techo deja
/// invertir 500 y 500 son `disposable` (salen del balance). NW(0) = 310.000, NW(1) = 310.500,
/// NW(2) = 311.000. En el mes 3 la casa se vende entera exenta: neto 300.000 que entra ENTERO en
/// la cascada junto a los 500 invertibles ⇒ fondo = 11.000 + 300.500 = 311.500, casa = 0,
/// NW(3) = 311.500 exacto (antes del arreglo: 11.500, con disposable[3] = 300.500), y
/// disposable[3] = 500 (solo el ahorro recortado). En las dos políticas (no hay déficit).
#[test]
fn b1_explicit_sale_proceeds_bypass_the_contribution_cap() {
    for policy in [DeficitPolicy::LegacyDrain, DeficitPolicy::SinkOnly] {
        let out = project_net_worth_series(&b1_household(3, policy)).unwrap();
        assert_eq!(out.net_worth[0], eur(310_000), "{policy:?}");
        assert_eq!(out.net_worth[1], eur(310_500), "{policy:?}");
        assert_eq!(out.net_worth[2], eur(311_000), "{policy:?}");
        assert_eq!(out.net_worth[3], eur(311_500), "{policy:?}");
        assert_eq!(out.per_asset_series[0][3], eur(311_500), "{policy:?}");
        assert_eq!(out.per_asset_series[1][3], Decimal::ZERO, "{policy:?}");
        assert_eq!(out.disposable_cash[3], eur(500), "{policy:?}");
        assert_eq!(out.net_worth[6], eur(313_000), "{policy:?}");
        assert_eq!(out.disposable_cash_total, eur(3_000), "{policy:?}");
    }
}

/// B1 en `first_month_allocation`: la venta en el mes 1 ⇒ el pool es 500 + 300.000 y
/// `disposable` es 500 (antes: 300.500). Predicción: per_asset[0] = 300.500.
#[test]
fn b1_first_month_allocation_also_bypasses_the_cap() {
    for policy in [DeficitPolicy::LegacyDrain, DeficitPolicy::SinkOnly] {
        let fma = first_month_allocation(&b1_household(1, policy)).unwrap();
        assert_eq!(fma.explicit_sales_net, eur(300_000), "{policy:?}");
        assert_eq!(fma.base_cash, eur(301_000), "{policy:?}");
        assert_eq!(fma.disposable, eur(500), "{policy:?}");
        assert_eq!(fma.per_asset[0], eur(300_500), "{policy:?}");
        let out = project_net_worth_series(&b1_household(1, policy)).unwrap();
        assert_eq!(out.per_asset_series[0][1], fma.per_asset[0] + eur(10_000));
    }
}

/// B2: en `SinkOnly`, `assets_depleted_month_index` es «el sumidero se agotó».
/// Predicción: (a) sin sumidero (sin reglas) ⇒ None aunque todo mes quede sin cubrir; (b)
/// sumidero ilíquido (= sin sumidero) ⇒ None; (c) sumidero de 10.000 con déficit 3.000/mes ⇒
/// llega a 0 en el mes 4 ⇒ Some(4).
#[test]
fn b2_sink_only_depletion_means_the_sink_ran_out() {
    let mut no_sink = one_sink(4, 1_000, 2_000, 50_000, 50_000, DeficitPolicy::SinkOnly);
    no_sink.allocation_rules = vec![];
    let out = project_net_worth_series(&no_sink).unwrap();
    assert_eq!(out.first_uncovered_month, Some(1));
    assert_eq!(out.assets_depleted_month_index, None);

    let mut illiquid = base_input(
        6,
        eur(1_000),
        eur(2_000),
        vec![mk_asset(1, eur(300_000), false, None)],
        vec![rule_remainder(0)],
    );
    illiquid.deficit_policy = DeficitPolicy::SinkOnly;
    let out = project_net_worth_series(&illiquid).unwrap();
    assert_eq!(out.first_uncovered_month, Some(1));
    assert_eq!(out.assets_depleted_month_index, None);

    let out = project_net_worth_series(&one_sink(
        8,
        1_000,
        4_000,
        10_000,
        10_000,
        DeficitPolicy::SinkOnly,
    ))
    .unwrap();
    assert_eq!(out.assets_depleted_month_index, Some(4));
}

/// B3 (a): una orden que agota su activo se detiene PARA SIEMPRE aunque la cascada lo rellene.
/// Cuenta sumidero 1.000 (activo 0), fondo 120.000 (activo 1) que recibe una regla fija de
/// 5.000/mes antes del `Remainder`; ingreso = gasto (ahorro 0); venta mensual exenta de 50.000
/// del fondo desde el mes 1. Predicción: mes 1 fondo 120.000 − 50.000 + 5.000 = 75.000; mes 2
/// 30.000; mes 3 vende los 30.000 (agotada `(1, 3)`) y la cascada le devuelve 5.000; meses 4–6
/// sin venta (antes: vendía los 5.000 rellenados cada mes) ⇒ fondo = 5.000 y neto vendido 0.
#[test]
fn b3_exhausted_order_never_resumes_after_a_refill() {
    let mut input = base_input(
        6,
        eur(3_000),
        eur(3_000),
        vec![
            mk_asset(1, eur(1_000), true, None),
            mk_asset(2, eur(120_000), true, None),
        ],
        vec![rule_fixed(1, eur(5_000), None), rule_remainder(0)],
    );
    input.asset_sales = build_asset_sale_orders(
        &[AssetSaleSpec {
            asset_index: 1,
            start_month: 1,
            end_month: None,
            kind: AssetSaleKind::MonthlyNet {
                amount_today: eur(50_000),
            },
            keeps_purchasing_power: false,
            tax_mode: SaleTaxMode::Exempt,
        }],
        6,
        Decimal::ZERO,
    );
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.per_asset_series[1][1], eur(75_000));
    assert_eq!(out.per_asset_series[1][2], eur(30_000));
    assert_eq!(out.explicit_sale_net[3], eur(30_000));
    assert_eq!(out.per_asset_series[1][3], eur(5_000));
    for k in 4..=6 {
        assert_eq!(out.explicit_sale_net[k], Decimal::ZERO, "mes {k}");
    }
    assert_eq!(out.per_asset_series[1][6], eur(5_000));
    assert_eq!(out.explicit_sale_exhausted, vec![(1, 3)]);
    assert_eq!(out.net_worth[6], eur(121_000));
}

/// B3 (b): una orden anterior al mes de inicio del activo (F1a) se salta SIN marcar
/// agotamiento. Fondo de 20.000 que empieza en el mes 4; venta mensual exenta de 1.000 desde el
/// mes 1. Predicción: meses 1–3 sin venta, mes 4 en adelante 1.000/mes, `exhausted` vacío
/// (antes: `(1, 1)`), fondo(6) = 17.000.
#[test]
fn b3_orders_before_the_asset_exists_are_skipped_not_exhausted() {
    let mut fund = mk_asset(2, eur(20_000), true, None);
    fund.start_month = Some(4);
    let mut input = base_input(
        6,
        eur(3_000),
        eur(3_000),
        vec![mk_asset(1, eur(1_000), true, None), fund],
        vec![rule_remainder(0)],
    );
    input.asset_sales = build_asset_sale_orders(
        &[AssetSaleSpec {
            asset_index: 1,
            start_month: 1,
            end_month: None,
            kind: AssetSaleKind::MonthlyNet {
                amount_today: eur(1_000),
            },
            keeps_purchasing_power: false,
            tax_mode: SaleTaxMode::Exempt,
        }],
        6,
        Decimal::ZERO,
    );
    let out = project_net_worth_series(&input).unwrap();
    for k in 1..=3 {
        assert_eq!(out.explicit_sale_net[k], Decimal::ZERO, "mes {k}");
    }
    for k in 4..=6 {
        assert_eq!(out.explicit_sale_net[k], eur(1_000), "mes {k}");
    }
    assert!(out.explicit_sale_exhausted.is_empty());
    assert_eq!(out.per_asset_series[1][6], eur(17_000));
    let fma = first_month_allocation(&input).unwrap();
    assert_eq!(fma.explicit_sales_net, Decimal::ZERO);
}

/// B4: `ManualPct` fuera de `[0, 100)` y `ManualAmount` negativo se rechazan en la validación
/// de la entrada, en cualquier tipo de venta. Predicción: `Whole` + 100 % (antes: impuesto =
/// valor, activo destruido), `MonthlyNet` + 150 %, −1 % y −5 € ⇒ `InvalidAssetSales` en el bucle
/// y en el mes 1; 99,99 % y 0 € pasan.
#[test]
fn b4_invalid_manual_tax_is_rejected() {
    let bad = [
        (
            AssetSaleKind::Whole,
            SaleTaxMode::ManualPct { pct: eur(100) },
        ),
        (MONTHLY_2000, SaleTaxMode::ManualPct { pct: eur(150) }),
        (
            AssetSaleKind::Whole,
            SaleTaxMode::ManualPct { pct: eur(-1) },
        ),
        (MONTHLY_2000, SaleTaxMode::ManualAmount { tax: eur(-5) }),
    ];
    for (kind, mode) in bad {
        let mut input = sale_household(mode, kind);
        // La validación no depende del mes: también con la venta en el mes 1.
        input.asset_sales.orders[0].month = 1;
        assert_eq!(
            project_net_worth_series(&input).unwrap_err(),
            EngineError::InvalidAssetSales,
            "{kind:?} {mode:?}"
        );
        assert_eq!(
            first_month_allocation(&input).unwrap_err(),
            EngineError::InvalidAssetSales,
            "{kind:?} {mode:?}"
        );
    }
    for mode in [
        SaleTaxMode::ManualPct { pct: dec("99.99") },
        SaleTaxMode::ManualAmount { tax: Decimal::ZERO },
    ] {
        project_net_worth_series(&sale_household(mode, AssetSaleKind::Whole)).unwrap();
    }
}
