//! **Mes de inicio de activos y pasivos** (6.0.0 F1a, RFC Planes §3 «Motor»).
//!
//! Rejilla: `start_month = s` es el mes `s` del BUCLE (1-based). El ítem existe desde la apertura
//! del mes `s`, así que en la serie publicada (índice `i` = cierre del mes `i`, índice 0 = hoy)
//! aparece por primera vez en el índice `s − 1`. `None`, `Some(0)` y `Some(1)` son «existe hoy».
//!
//! Cada test declara su predicción ANTES de los asserts (futurefin-research-methodology): las
//! cifras están calculadas a mano o por construcción (desplazamiento exacto de una serie legacy).

#[path = "common/cases.rs"]
mod cases;

use cases::{
    base_input, d, mk_asset, mk_liab, p9_household, projection_cases_all, rule_fixed,
    rule_remainder,
};
use futurefin_engine::{
    first_month_allocation, liability_amortization_schedule, project_net_worth_series,
    AllocationSkipReason, ProjectionInput, RepaymentModel,
};
use rust_decimal::Decimal;

fn eur(n: i64) -> Decimal {
    Decimal::from(n)
}

/// 10.000 € líquidos al 0 %, ingreso 1.000 €/mes, gasto 0, todo el sobrante a la cuenta.
fn cash_household(horizon: u32) -> ProjectionInput {
    base_input(
        horizon,
        eur(1_000),
        Decimal::ZERO,
        vec![mk_asset(1, eur(10_000), true, None)],
        vec![rule_remainder(0)],
    )
}

/// (a) Un activo con `start_month = 7`: la serie es la del hogar SIN el activo hasta el índice
/// 5, y en el índice 6 (apertura del mes 7) salta EXACTAMENTE su valor, sin caja.
///
/// Predicción: sin el activo `NW[i] = 10.000 + 1.000·i`. Con la casa de 250.000 € al 0 %:
/// `NW[5] = 15.000` (igual), `NW[6] = 16.000 + 250.000 = 266.000`, `NW[24] = 284.000`.
#[test]
fn dated_asset_appears_at_series_index_start_minus_one_and_jumps_by_its_value() {
    let without = project_net_worth_series(&cash_household(24)).unwrap();

    let mut input = cash_household(24);
    let mut house = mk_asset(2, eur(250_000), false, None);
    house.start_month = Some(7);
    input.assets.push(house);
    let with = project_net_worth_series(&input).unwrap();

    for i in 0..6 {
        assert_eq!(with.net_worth[i], without.net_worth[i], "índice {i}");
        assert_eq!(with.per_asset_series[1][i], Decimal::ZERO, "índice {i}");
        assert_eq!(with.liquid_worth[i], without.liquid_worth[i], "índice {i}");
    }
    for i in 6..=24 {
        assert_eq!(
            with.net_worth[i] - without.net_worth[i],
            eur(250_000),
            "índice {i}: el salto es exactamente el valor, ni más ni menos"
        );
        // Ilíquida: el líquido no cambia.
        assert_eq!(with.liquid_worth[i], without.liquid_worth[i], "índice {i}");
    }
    assert_eq!(with.net_worth[5], eur(15_000));
    assert_eq!(with.net_worth[6], eur(266_000));
    assert_eq!(with.net_worth[24], eur(284_000));
    // La aparición no genera caja: la cuenta corriente es la misma serie.
    assert_eq!(with.per_asset_series[0], without.per_asset_series[0]);
    // La base de coste entra con el activo (sin `purchase_price` ⇒ 0): el capital aportado no
    // cambia.
    assert_eq!(with.contributed_capital, without.contributed_capital);
}

/// (a, crecimiento) Desde que aparece, el activo crece igual que uno que existiera desde hoy:
/// `serie_fechada[6 + j] == serie_legacy[j]` BIT A BIT (mismas multiplicaciones, en el mismo
/// orden), con el legacy simulado sobre `24 − 6 = 18` meses.
///
/// Predicción: `250.000 · 1,12^(18/12) ≈ 296.324` en el índice 24.
#[test]
fn dated_asset_then_grows_exactly_like_one_that_existed_from_today() {
    let mut dated = base_input(
        24,
        Decimal::ZERO,
        Decimal::ZERO,
        vec![mk_asset(1, eur(250_000), false, Some(eur(12)))],
        vec![rule_remainder(0)],
    );
    dated.assets[0].start_month = Some(7);
    let dated_out = project_net_worth_series(&dated).unwrap();

    let legacy = base_input(
        18,
        Decimal::ZERO,
        Decimal::ZERO,
        vec![mk_asset(1, eur(250_000), false, Some(eur(12)))],
        vec![rule_remainder(0)],
    );
    let legacy_out = project_net_worth_series(&legacy).unwrap();

    for i in 0..6 {
        assert_eq!(dated_out.per_asset_series[0][i], Decimal::ZERO);
    }
    for j in 0..=18 {
        assert_eq!(
            format!("{:?}", dated_out.per_asset_series[0][6 + j]),
            format!("{:?}", legacy_out.per_asset_series[0][j]),
            "j = {j}: mismo valor y misma escala"
        );
    }
    let last = dated_out.per_asset_series[0][24];
    println!("activo fechado al 12 % en el índice 24: {last}");
    assert!((last - d(29_632_400, 2)).abs() < eur(10), "obtenido {last}");
}

/// La cascada NO aporta a un activo que aún no existe: la regla se salta y su parte sigue
/// cascada abajo. Desde el mes de inicio, la regla vuelve a aportar.
///
/// Predicción (fijo 500 → casa fechada en el mes 7, resto → cuenta): meses 1–6 la cuenta recibe
/// 1.000; desde el 7, 500 y 500. Cuenta[6] = 16.000; casa[6] = 250.000; índice 24: casa =
/// 259.000, cuenta = 25.000; el patrimonio total es el mismo de (a): 284.000.
#[test]
fn allocation_rule_to_a_not_yet_existing_asset_is_skipped_and_flows_down() {
    let mut house = mk_asset(2, eur(250_000), false, None);
    house.start_month = Some(7);
    let input = base_input(
        24,
        eur(1_000),
        Decimal::ZERO,
        vec![mk_asset(1, eur(10_000), true, None), house],
        vec![rule_fixed(1, eur(500), None), rule_remainder(0)],
    );
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.per_asset_series[0][6], eur(16_000));
    assert_eq!(out.per_asset_series[1][5], Decimal::ZERO);
    assert_eq!(out.per_asset_series[1][6], eur(250_000));
    assert_eq!(out.per_asset_series[1][7], eur(250_500));
    assert_eq!(out.per_asset_series[0][7], eur(16_500));
    assert_eq!(out.per_asset_series[1][24], eur(259_000));
    assert_eq!(out.per_asset_series[0][24], eur(25_000));
    assert_eq!(out.net_worth[24], eur(284_000));
    assert_eq!(out.unallocated_savings_total, Decimal::ZERO);

    // El mes 1 resuelto (la explicación de la cascada) dice lo mismo que el bucle.
    let first = first_month_allocation(&input).unwrap();
    assert_eq!(first.per_asset, vec![eur(1_000), Decimal::ZERO]);
    assert_eq!(
        first.rules[0].skipped_reason,
        Some(AllocationSkipReason::InvalidTarget)
    );
    assert_eq!(first.rules[1].amount_resolved, eur(1_000));
}

/// Préstamo francés de 100.000 € al 3 % con cuota 965,61 (la exacta a 120 meses es 965,6074…).
fn french_loan() -> futurefin_engine::ProjectionLiabilityInput {
    mk_liab(
        eur(100_000),
        d(96_561, 2),
        Some(eur(3)),
        RepaymentModel::French,
        None,
    )
}

/// (b) Un pasivo firmado en el mes 13: cero deuda y cero cuotas antes; desde el 13, la serie es
/// la del mismo préstamo firmado hoy desplazada 12 meses — y por tanto el interés total es el
/// mismo.
///
/// Predicción: `NW[0..=11] = 0`; `NW[12] = −100.000` (la firma resta el principal, sin caja);
/// `unmet[13] = 965,61` (la primera cuota, sin activos que la cubran); `NW[12 + j] ==
/// legacy[j]` exacto; pago en 120 cuotas (última parcial) ⇒ interés ≈ `120·965,6074 − 100.000 ≈
/// 15.872,9 €`; extinción en el mes 120 del legacy y 132 del fechado.
#[test]
fn dated_liability_is_the_same_loan_shifted() {
    let horizon = 150;
    let shift = 12;
    let mut dated = base_input(horizon, Decimal::ZERO, Decimal::ZERO, vec![], vec![]);
    let mut loan = french_loan();
    loan.start_month = Some(shift + 1);
    dated.liabilities = vec![loan.clone()];
    let dated_out = project_net_worth_series(&dated).unwrap();

    let mut legacy = base_input(
        horizon - shift,
        Decimal::ZERO,
        Decimal::ZERO,
        vec![],
        vec![],
    );
    legacy.liabilities = vec![french_loan()];
    let legacy_out = project_net_worth_series(&legacy).unwrap();

    for i in 0..shift as usize {
        assert_eq!(dated_out.net_worth[i], Decimal::ZERO, "índice {i}");
    }
    for k in 1..=shift as usize {
        assert_eq!(dated_out.unmet_need[k], Decimal::ZERO, "mes {k}: sin cuota");
    }
    assert_eq!(dated_out.net_worth[shift as usize], -eur(100_000));
    assert_eq!(dated_out.unmet_need[shift as usize + 1], d(96_561, 2));
    for j in 0..=(horizon - shift) as usize {
        assert_eq!(
            format!("{:?}", dated_out.net_worth[shift as usize + j]),
            format!("{:?}", legacy_out.net_worth[j]),
            "j = {j}"
        );
    }
    // Sin activos, toda la caja de la deuda es descubierto: total pagado idéntico.
    assert_eq!(
        dated_out.uncovered_deficit_total,
        legacy_out.uncovered_deficit_total
    );

    // El calendario de amortización: empieza en el mes 13 y es fila a fila el legacy.
    let sch_dated = liability_amortization_schedule(&loan, dated.ref_date, horizon);
    let sch_legacy = liability_amortization_schedule(&french_loan(), dated.ref_date, horizon);
    assert_eq!(sch_dated.months.len(), sch_legacy.months.len());
    assert_eq!(sch_dated.months[0].month_index, shift + 1);
    for (a, b) in sch_dated.months.iter().zip(&sch_legacy.months) {
        assert_eq!(a.month_index, b.month_index + shift);
        assert_eq!(a.payment, b.payment);
        assert_eq!(a.interest_accrued, b.interest_accrued);
        assert_eq!(a.closing_principal, b.closing_principal);
    }
    assert_eq!(sch_dated.total_interest, sch_legacy.total_interest);
    assert_eq!(
        sch_dated.payoff_month_index,
        sch_legacy.payoff_month_index.map(|m| m + shift)
    );
    println!(
        "interés total: {} — extinción legacy {:?}, fechado {:?}",
        sch_legacy.total_interest, sch_legacy.payoff_month_index, sch_dated.payoff_month_index
    );
    assert_eq!(sch_legacy.payoff_month_index, Some(120));
    assert!((sch_legacy.total_interest - d(1_587_290, 2)).abs() < eur(2));
    // Interés del calendario = total pagado − principal (identidad de la renta). No bit a bit:
    // las dos sumas acumulan 120 términos por caminos distintos y difieren en el dígito 28.
    let paid_minus_principal = legacy_out.uncovered_deficit_total - eur(100_000);
    assert!(
        (sch_legacy.total_interest - paid_minus_principal).abs() < Decimal::new(1, 20),
        "{} vs {paid_minus_principal}",
        sch_legacy.total_interest
    );
}

/// (c) `start_month` = `None`, `Some(0)` y `Some(1)` son el mismo hogar, bit a bit (mismo
/// `Debug`, que es lo que distingue la escala de un `Decimal`), en TODA la batería de 4.15.0 y
/// en el hogar P9.
#[test]
fn start_at_first_month_or_zero_reproduces_legacy_bit_for_bit() {
    let mut cases: Vec<(String, ProjectionInput)> = projection_cases_all()
        .into_iter()
        .map(|c| (c.name.to_string(), c.input))
        .collect();
    cases.push(("p9".into(), p9_household(eur(20_000))));
    assert!(cases.len() > 10);
    for (name, input) in cases {
        let legacy = project_net_worth_series(&input);
        for start in [Some(0), Some(1)] {
            let mut alt = input.clone();
            for a in &mut alt.assets {
                a.start_month = start;
            }
            for l in &mut alt.liabilities {
                l.start_month = start;
            }
            let out = project_net_worth_series(&alt);
            assert_eq!(
                format!("{legacy:?}"),
                format!("{out:?}"),
                "{name} con start_month = {start:?}"
            );
            assert_eq!(
                format!("{:?}", first_month_allocation(&input)),
                format!("{:?}", first_month_allocation(&alt)),
                "{name}: mes 1"
            );
        }
    }
}

/// El caso de la puerta de degeneración (D1) hace lo que dice: la casa y la hipoteca aparecen en
/// el índice 36 (mes 37), la herencia en el 99 (mes 100), y nada fechado existe antes.
///
/// Predicción: `per_asset[1][35] = 0`, `per_asset[1][36] = 250.000` exacto (aparece sin crecer);
/// `per_asset[2][98] = 0`, `per_asset[2][99] = 50.000`; el salto de patrimonio entre 35 y 36 es
/// `+250.000 − 200.000 = +50.000` más el ahorro de un mes, y el de 98→99 incluye `+50.000`.
#[test]
fn degeneration_case_d1_materialises_on_schedule() {
    let case = cases::projection_cases_6_0_dated()
        .into_iter()
        .next()
        .unwrap();
    let out = project_net_worth_series(&case.input).unwrap();
    assert_eq!(out.per_asset_series[1][35], Decimal::ZERO);
    assert_eq!(out.per_asset_series[1][36], eur(250_000));
    assert_eq!(out.per_asset_series[2][98], Decimal::ZERO);
    assert_eq!(out.per_asset_series[2][99], eur(50_000));
    let jump_36 = out.net_worth[36] - out.net_worth[35];
    let jump_99 = out.net_worth[99] - out.net_worth[98];
    println!(
        "D1: ΔNW[35→36] = {jump_36}, ΔNW[98→99] = {jump_99}, agotamiento = {:?}, \
         descubierto = {}",
        out.assets_depleted_month_index, out.uncovered_deficit_total
    );
    assert!(jump_36 > eur(50_000) && jump_36 < eur(52_000), "{jump_36}");
    assert!(jump_99 > eur(50_000), "{jump_99}");
}
