//! **El capital necesario** (WP E7 de 5.0.0): el escalado del líquido, la bisección sobre `λ`, la
//! cifra de HOY y la curva por edad.
//!
//! Mismo arnés y misma disciplina que `tests/solve_mc.rs`: hogares pequeños **derivados a mano**,
//! con la aritmética de la predicción escrita al lado del test ANTES de correrlo
//! (`futurefin-research-methodology`, «predice el número antes de ejecutar»). Lo que aquí se prueba
//! es el SOLVER —y el escalado que le da de comer—, así que la función objetivo tiene una forma
//! conocida a propósito: un ESCALÓN en `λ`.
//!
//! **Presupuesto**: ningún test de este fichero pasa de 250 caminos, y todos salvo uno se quedan en
//! 120 meses de horizonte. La excepción es
//! `the_curve_uses_the_scaled_liquid_so_late_nodes_are_not_zero`, que necesita P9 y sus 840 meses
//! porque el fenómeno que fija —una trayectoria sin escalar que se agota antes del nodo— no existe
//! en un horizonte corto; se compensa con el mínimo de caminos. La medición de tiempos vive en
//! `tests/timing_mc.rs`, `#[ignore]` y en release.

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset, mk_asset_with_basis, p9_household, rule_remainder};
use futurefin_engine::{project_net_worth_series, InitialRateGate, ProjectionInput, TaxBracket};
use futurefin_engine_stochastic::{
    needed_capital_curve, needed_capital_today, needed_liquid_at_month, retiring_at,
    scale_liquid_assets, McConfig, ABSENT_NO_LIQUID_ASSETS,
};
use rust_decimal::Decimal;

/// Semilla fija: los tests miden un sorteo concreto, no «un sorteo».
const SEED: u64 = 20_260_906;

fn cfg(paths: u32) -> McConfig {
    McConfig {
        seed: SEED,
        paths,
        ..Default::default()
    }
}

fn vol() -> Vec<Option<f64>> {
    vec![Some(8.0)]
}

const THRESHOLD: u32 = 80;

// =================================================================================================
// Los hogares del arnés
// =================================================================================================

/// **El hogar de la PUERTA**: una sola cartera líquida al 5 % CAGR con 8 % de volatilidad, sin
/// ingreso ni gasto regulares, gasto de jubilación 2.000 €/mes, inflación 0, horizonte 120 meses y
/// puerta de tasa inicial al 4 %.
///
/// Derivado a mano ANTES de correr nada. Jubilándose en el mes 1:
///
/// ```text
///   necesidad anual íntegra = 12 · 2.000 = 24.000 €
///   tope de tasa inicial    = 4 % · L(0)
///   la puerta FALLA  ⟺  24.000 > 0,04 · L(0)  ⟺  L(0) < 600.000 €
/// ```
///
/// Esa comparación **no depende del sorteo**: `L(0)` es el estado inicial y es idéntico en todos
/// los caminos. Y F1 no compite: drenar 24.000 €/año durante 10 años de una cartera de 600.000 € al
/// 5 % no la agota en ningún camino. Por tanto `éxito(λ, k = 1)` es un **ESCALÓN** en
/// `λ_b = 600.000 / valor`, y el capital necesario de hoy es 600.000 € más lo que el redondeo hacia
/// arriba añada.
fn gated_household(liquid: Decimal) -> ProjectionInput {
    let mut input = base_input(
        120,
        Decimal::ZERO,
        Decimal::ZERO,
        vec![mk_asset(1, liquid, true, Some(Decimal::from(5)))],
        vec![rule_remainder(0)],
    );
    input.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    input.phase_plan.income_retirement_monthly = Decimal::ZERO;
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: Decimal::from(4),
        bridge: None,
    });
    input
}

/// **El hogar de la LIQUIDACIÓN**: un único activo líquido con base de coste declarada, tipo
/// PLANO del 20 % sobre la plusvalía, jubilado desde el mes 1 con un gasto de jubilación
/// deliberadamente imposible (1.000.000 €/mes).
///
/// El primer mes vende TODA su capacidad, así que `withdrawal[1]` es exactamente el neto de
/// liquidar la cartera entera: `v − 20 % · g · v` con `g = 1 − b/v`. Con un tramo único el
/// impuesto es lineal y la anualización del gross-up no cambia nada, que es justo por lo que se
/// eligió un tramo único: el número se deriva a mano sin escala marginal de por medio.
fn liquidation_household(value: Decimal, basis: Decimal) -> ProjectionInput {
    let mut input = base_input(
        12,
        Decimal::ZERO,
        Decimal::ZERO,
        vec![mk_asset_with_basis(
            1,
            value,
            true,
            Some(Decimal::ZERO),
            basis,
        )],
        vec![rule_remainder(0)],
    );
    input.taxes_enabled = true;
    input.tax_brackets = vec![TaxBracket {
        up_to: None,
        pct: Decimal::from(20),
    }];
    input.taxable_gain_ratio = Decimal::ONE;
    input.phase_plan.expense_retirement_monthly = Decimal::from(1_000_000);
    input.phase_plan.income_retirement_monthly = Decimal::ZERO;
    input
}

/// El neto que sale de los activos el mes 1 jubilándose en el mes 1.
fn month_one_net_withdrawal(input: &ProjectionInput) -> Decimal {
    let out = project_net_worth_series(&retiring_at(input, 1)).expect("el motor no falla");
    out.withdrawal[1]
}

// =================================================================================================
// El escalado
// =================================================================================================

/// **Solo el líquido se escala.** La vivienda (#143) no es el stock que el drenaje puede vender:
/// multiplicarla movería el patrimonio publicado sin mover ni un euro de la capacidad de jubilarse.
#[test]
fn scaling_the_liquid_leaves_the_house_alone() {
    let input = base_input(
        12,
        Decimal::ZERO,
        Decimal::ZERO,
        vec![
            mk_asset_with_basis(
                1,
                Decimal::from(100_000),
                true,
                Some(Decimal::ZERO),
                Decimal::from(40_000),
            ),
            // La vivienda: ilíquida, CON base declarada para que se vea que tampoco esa se toca.
            mk_asset_with_basis(
                2,
                Decimal::from(250_000),
                false,
                Some(Decimal::ONE),
                Decimal::from(200_000),
            ),
            // Un líquido SIN base declarada: sigue sin ella (su fiscalidad la gobierna el escalar
            // `taxable_gain_ratio`, que no depende del tamaño).
            mk_asset(3, Decimal::from(5_000), true, None),
        ],
        vec![rule_remainder(0)],
    );

    let scaled = scale_liquid_assets(&input, 3.0);
    assert_eq!(scaled.assets[0].value, Decimal::from(300_000));
    assert_eq!(scaled.assets[0].purchase_price, Some(Decimal::from(120_000)));
    assert_eq!(scaled.assets[1].value, Decimal::from(250_000), "la vivienda");
    assert_eq!(scaled.assets[1].purchase_price, Some(Decimal::from(200_000)));
    assert_eq!(scaled.assets[2].value, Decimal::from(15_000));
    assert_eq!(scaled.assets[2].purchase_price, None);

    // Nada más se toca: ni el gasto, ni las reglas, ni el horizonte.
    assert_eq!(scaled.expense_regular_monthly, input.expense_regular_monthly);
    assert_eq!(scaled.allocation_rules.len(), input.allocation_rules.len());
    assert_eq!(scaled.horizon_months, input.horizon_months);

    // λ = 1 es la identidad exacta.
    let same = scale_liquid_assets(&input, 1.0);
    assert_eq!(same.assets[0].value, Decimal::from(100_000));
    assert_eq!(same.assets[0].purchase_price, Some(Decimal::from(40_000)));
}

/// **La base de coste viaja con el valor, y por eso no aparece una plusvalía fantasma.**
///
/// Números escritos antes de correr, con `g = 1 − b/v` y un tipo plano del 20 %:
///
/// | cartera | `g` | neto de liquidar |
/// |---|---|---|
/// | v = 100.000, b = 40.000 (λ = 1) | 0,6 | `100.000 − 0,2·0,6·100.000` = **88.000 €** |
/// | v = 200.000, b = 80.000 (λ = 2, valor Y base) | 0,6 | `200.000 − 0,2·0,6·200.000` = **176.000 € = 2 × 88.000** |
/// | v = 200.000, b = 40.000 (λ = 2 **solo el valor**) | 0,8 | `200.000 − 0,2·0,8·200.000` = **168.000 €** |
///
/// Escalar el valor dejando la base quieta cobra **8.000 € de impuesto que no existen** y subiría
/// el capital necesario por un artefacto del método.
#[test]
fn scaling_moves_the_basis_with_the_value_so_no_phantom_gain_appears() {
    let base = liquidation_household(Decimal::from(100_000), Decimal::from(40_000));
    let net_one = month_one_net_withdrawal(&base);
    assert_eq!(net_one, Decimal::from(88_000));

    let doubled = scale_liquid_assets(&base, 2.0);
    assert_eq!(doubled.assets[0].value, Decimal::from(200_000));
    assert_eq!(doubled.assets[0].purchase_price, Some(Decimal::from(80_000)));
    let net_two = month_one_net_withdrawal(&doubled);
    assert_eq!(net_two, Decimal::from(176_000));
    assert_eq!(
        net_two,
        net_one * Decimal::from(2),
        "el neto después de impuestos escala EXACTAMENTE por λ"
    );

    // El contrafactual: la misma cartera con la base sin escalar.
    let phantom = liquidation_household(Decimal::from(200_000), Decimal::from(40_000));
    assert_eq!(
        month_one_net_withdrawal(&phantom),
        Decimal::from(168_000),
        "la plusvalía fantasma cuesta 8.000 € de impuesto inventado"
    );
}

// =================================================================================================
// El capital necesario de hoy
// =================================================================================================

/// **Hoy = la curva en el mes 1.** El envoltorio no es una segunda definición y la curva evaluada
/// en `[1]` con el MISMO presupuesto da el mismo `λ` y el mismo importe.
///
/// Predicción: con 300.000 € líquidos, `λ_b = 600.000/300.000 = 2` **exacto** (la puerta compara
/// con `>` estricto, así que la igualdad pasa) ⇒ capital necesario hoy = **600.000 €**, sin
/// redondeo que aplicar y sin deflactar (el factor de inflación en el índice 0 es 1).
#[test]
fn the_needed_capital_today_is_the_curve_at_month_one() {
    let input = gated_household(Decimal::from(300_000));
    let mc = cfg(100);

    let today = needed_capital_today(&input, &vol(), &mc, &mc, THRESHOLD).expect("no falla");
    let at_one =
        needed_liquid_at_month(&input, &vol(), &mc, &mc, THRESHOLD, 1).expect("no falla");
    assert_eq!(today, at_one, "el envoltorio es literalmente el mes 1");

    assert_eq!(today.month, 1);
    assert_eq!(today.absent_reason, None);
    assert_eq!(today.lambda, Some(2.0), "λ_b = 600.000/300.000");
    assert_eq!(today.amount_nominal, Some(Decimal::from(600_000)));
    assert_eq!(
        today.amount_today, today.amount_nominal,
        "en el mes 1 no hay nada que deflactar"
    );

    let curve = needed_capital_curve(&input, &vol(), &mc, THRESHOLD, &[1]).expect("no falla");
    assert_eq!(curve.len(), 1);
    assert_eq!(curve[0].month, 1);
    assert_eq!(curve[0].lambda, today.lambda);
    assert_eq!(curve[0].amount_nominal, today.amount_nominal);
    assert_eq!(curve[0].amount_today, today.amount_today);
}

/// **Un hogar sin activos líquidos dice por qué en vez de publicar 0 €.** Escalar cero es cero: el
/// método no puede medir, y un `0 €` se leería como «no necesitas nada», que es la respuesta
/// contraria. Ni un sorteo se gasta.
#[test]
fn a_household_with_no_liquid_assets_says_why_instead_of_publishing_zero() {
    let mut input = base_input(
        120,
        Decimal::ZERO,
        Decimal::ZERO,
        // Solo la vivienda, ilíquida.
        vec![mk_asset(4, Decimal::from(250_000), false, Some(Decimal::ONE))],
        vec![rule_remainder(0)],
    );
    input.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: Decimal::from(4),
        bridge: None,
    });
    let vols = vec![Some(8.0)];
    let mc = cfg(100);

    let out = needed_capital_today(&input, &vols, &mc, &mc, THRESHOLD).expect("no falla");
    assert_eq!(out.absent_reason, Some(ABSENT_NO_LIQUID_ASSETS));
    assert_eq!(out.lambda, None);
    assert_eq!(out.amount_nominal, None);
    assert_eq!(out.amount_today, None);
    assert_eq!(out.success_at_lambda, None);
    assert_eq!(
        (out.draws_search, out.draws_confirm),
        (0, 0),
        "no se sortea lo que no se puede escalar"
    );

    let curve = needed_capital_curve(&input, &vols, &mc, THRESHOLD, &[1, 61]).expect("no falla");
    assert_eq!(curve.len(), 2);
    assert!(curve
        .iter()
        .all(|n| n.absent_reason == Some(ABSENT_NO_LIQUID_ASSETS) && n.amount_nominal.is_none()));
}

/// **El importe publicado se redondea a cientos HACIA ARRIBA** (D4 enmendado): un capital necesario
/// redondeado a la baja quedaría por debajo del umbral que promete.
///
/// Con 333.333 € líquidos la frontera es `λ_b = 600.000/333.333 = 1,8000018`, que no cae en la
/// rejilla diádica de la bisección: el `λ` verificado deja un producto crudo de ≈ 600.016 € y el
/// publicado tiene que ser 600.100 € — múltiplo de 100, por encima del crudo y a menos de 100 € de
/// él.
#[test]
fn the_published_capital_rounds_up_to_hundreds() {
    let liquid = Decimal::from(333_333);
    let input = gated_household(liquid);
    let out = needed_capital_today(&input, &vol(), &cfg(100), &cfg(200), THRESHOLD).expect("ok");

    let lambda = out.lambda.expect("λ verificado");
    let published = out.amount_nominal.expect("importe");
    // En `k = 1` el líquido de la trayectoria escalada en el mes 0 ES `λ·L(0)`: el estado inicial,
    // antes de que ningún flujo intervenga. Es la única `k` donde el producto sirve de oráculo.
    let raw = Decimal::from_f64_retain(lambda)
        .expect("λ representable")
        .round_dp(10)
        * liquid;

    let hundred = Decimal::from(100);
    assert_eq!(
        published % hundred,
        Decimal::ZERO,
        "publicado = {published}, crudo = {raw}"
    );
    assert!(published >= raw, "publicado {published} < crudo {raw}");
    assert!(
        published - raw < hundred,
        "el redondeo añade {} €",
        published - raw
    );
    assert!(
        published >= Decimal::from(600_000),
        "por debajo de la puerta que promete: {published}"
    );
}

/// **`λ < 1` significa «ya tienes más del que necesitas».** Con 1.000.000 € líquidos la puerta se
/// abre de sobra en el mes 1 (`4 % · 1.000.000 = 40.000 € > 24.000 €`), así que la bisección baja:
/// `λ_b = 600.000/1.000.000 = 0,6` y el capital necesario queda por debajo de la cartera actual.
#[test]
fn lambda_below_one_means_you_already_have_more_than_you_need() {
    let input = gated_household(Decimal::from(1_000_000));
    let mc = cfg(100);
    let out = needed_capital_today(&input, &vol(), &mc, &mc, THRESHOLD).expect("no falla");

    let lambda = out.lambda.expect("λ verificado");
    assert!(lambda < 1.0, "λ* = {lambda}");
    assert!(lambda > 0.5, "la frontera está en 0,6: λ* = {lambda}");

    let published = out.amount_nominal.expect("importe");
    assert!(
        published < Decimal::from(1_000_000),
        "capital necesario {published} ≥ la cartera de hoy"
    );
    assert!(
        published >= Decimal::from(600_000),
        "por debajo de la puerta: {published}"
    );
}

/// **La cifra de hoy se verifica con el presupuesto de CONFIRMACIÓN**, y lo que se publica al lado
/// es esa medición —no la de búsqueda—, con su `N`, sus fallos y su cota de Wilson.
#[test]
fn needed_capital_is_verified_with_the_confirm_budget() {
    let input = gated_household(Decimal::from(300_000));
    let out = needed_liquid_at_month(&input, &vol(), &cfg(100), &cfg(250), THRESHOLD, 1)
        .expect("no falla");

    let stats = out.success_at_lambda.expect("medición publicada");
    assert_eq!(stats.paths, 250, "la medición publicada es la de confirmación");
    assert_eq!(stats.month, 1);
    assert!(
        stats.meets(THRESHOLD),
        "el λ publicado se ejecutó y cumplió: wilson_low = {}",
        stats.wilson_low
    );
    assert!(!out.capital_is_approximate);
    assert!(out.draws_confirm >= 1, "sin confirmación no hay verificación");
    assert!(
        out.draws_search >= 2,
        "búsqueda y confirmación se cuentan por separado: {} sorteos",
        out.draws_search
    );
}

// =================================================================================================
// La curva
// =================================================================================================

/// **La rejilla es la del llamante** —en su orden y con sus repeticiones— y los nodos posteriores
/// arrancan del `λ` del anterior.
///
/// El warm start se mide con la rejilla `[1, 1]`: los dos nodos resuelven EXACTAMENTE el mismo
/// problema, así que tienen que llegar al mismo `λ` y el segundo tiene que costar menos sorteos.
/// Derivado a mano: el nodo frío gasta `1 (λ=1, falla) + 1 (λ=2, cumple) + 12` = **14** sorteos; el
/// caliente arranca en 2, halva a 1 (falla) y bisecciona 8 ⇒ **10**.
#[test]
fn the_curve_is_evaluated_on_the_grid_the_caller_passes_and_warm_starts() {
    let input = gated_household(Decimal::from(300_000));
    let mc = cfg(100);

    // (a) el orden es el que se pasa, sin reordenar ni deduplicar
    let curve = needed_capital_curve(&input, &vol(), &mc, THRESHOLD, &[61, 1]).expect("ok");
    assert_eq!(
        curve.iter().map(|n| n.month).collect::<Vec<_>>(),
        vec![61, 1]
    );

    // (b) warm start
    let twice = needed_capital_curve(&input, &vol(), &mc, THRESHOLD, &[1, 1]).expect("ok");
    assert_eq!(twice.len(), 2);
    assert_eq!(twice[0].lambda, twice[1].lambda, "mismo problema, mismo λ");
    assert_eq!(twice[0].amount_nominal, twice[1].amount_nominal);
    assert!(
        twice[1].draws_search < twice[0].draws_search,
        "el nodo caliente ({}) no ahorró sorteos frente al frío ({})",
        twice[1].draws_search,
        twice[0].draws_search
    );

    // (c) la curva no confirma nada: un solo presupuesto y ningún importe «aproximado»
    assert!(twice
        .iter()
        .chain(curve.iter())
        .all(|n| n.draws_confirm == 0 && !n.capital_is_approximate));

    // (d) rejilla vacía ⇒ vector vacío (después de validar la configuración)
    assert!(needed_capital_curve(&input, &vol(), &mc, THRESHOLD, &[])
        .expect("ok")
        .is_empty());
}

/// **La curva lee el líquido del hogar ESCALADO, no `λ*·L_det`** — y por eso los nodos tardíos
/// publican una cifra en vez de una ausencia.
///
/// P9 es el caso que lo destapó (medido en `tests/timing_mc.rs`): su ingreso es plano y su gasto se
/// indexa al 2,5 % (#139), así que la trayectoria SIN escalar entra en déficit, drena la cartera y
/// llega al mes 839 **con el líquido a cero**. Con el producto `λ*·L_det(839)` el nodo del
/// horizonte valía `λ*·0 = 0 €` y salía como `no_liquid_assets`, aunque el hogar escalado sí tiene
/// cartera ahí. Con la trayectoria escalada, el nodo publica su importe.
///
/// Es el ÚNICO test de este fichero que usa P9 y sus 840 meses: el fenómeno no existe en un
/// horizonte corto. Se compensa con el mínimo de caminos que el umbral admite.
#[test]
fn the_curve_uses_the_scaled_liquid_so_late_nodes_are_not_zero() {
    let mut input = p9_household(Decimal::ZERO);
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: Decimal::new(35, 1),
        bridge: None,
    });
    // Cuenta 0 % · bonos 5 % · RV 16 % · vivienda 8 % · cripto 20 % (la cola de 70 % de la demo
    // sube el coste sin cambiar lo que este test fija).
    let vols = vec![None, Some(5.0), Some(16.0), Some(8.0), Some(20.0)];

    // La PREMISA del test: sin escalar, el líquido del mes 839 es cero.
    let unscaled = project_net_worth_series(&retiring_at(&input, 840)).expect("el motor no falla");
    assert!(
        unscaled.liquid_worth[839] <= Decimal::ZERO,
        "la premisa se ha roto: L_det(839) = {}",
        unscaled.liquid_worth[839]
    );

    let curve = needed_capital_curve(&input, &vols, &cfg(60), THRESHOLD, &[840]).expect("ok");
    assert_eq!(curve.len(), 1);
    let node = curve[0];
    assert_eq!(
        node.absent_reason, None,
        "el nodo del horizonte vuelve a ser una ausencia (λ* = {:?})",
        node.lambda
    );
    let nominal = node.amount_nominal.expect("importe nominal");
    let today = node.amount_today.expect("importe de hoy");
    assert!(nominal > Decimal::ZERO, "importe {nominal}");
    assert!(
        today < nominal,
        "839 meses al 2,5 % tienen que separar el nominal ({nominal}) del de hoy ({today})"
    );
    // Y el importe es el líquido REAL de esa trayectoria, no un producto: se reconstruye a mano.
    let lambda = node.lambda.expect("λ verificado");
    let scaled = project_net_worth_series(&retiring_at(&scale_liquid_assets(&input, lambda), 840))
        .expect("el motor no falla");
    let expected = scaled.liquid_worth[839];
    assert!(
        nominal >= expected && nominal - expected < Decimal::from(100),
        "publicado {nominal} no es el redondeo hacia arriba de {expected}"
    );
}
