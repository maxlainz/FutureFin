//! **Los reproductores de la segunda revisión adversarial (D20, 2026-09-03)**, convertidos en
//! regresiones.
//!
//! Cada test de este fichero fija UNA de las correcciones del pase, con los números medidos
//! antes y después en el propio doc. Viven juntos —y no repartidos por los módulos— porque lo que
//! los une no es el área del motor sino su procedencia: si alguien deshace una de estas
//! decisiones, aquí es donde se entera de cuál era el hallazgo y cuánto costaba.
//!
//! | test | hallazgo | qué fijaba |
//! |---|---|---|
//! | `an_exact_landing_that_covers_every_later_need_is_not_a_depletion` | #2 | `>=` sobre el aterrizaje exacto |
//! | `the_binding_allowance_is_a_cut_on_the_mixed_path_too` | #3 | el techo que no ataba en la vía mixta |
//! | `guardrails_under_a_binding_allowance_classify_on_both_paths` | #3 | el mismo, con parámetros que la API acepta |
//! | `rule_is_spend_funds_the_month_surplus_first` | #4 | comprar y vender el mismo fondo el mismo mes |
//! | `a_bridge_discount_too_negative_is_a_typed_error_not_a_panic` | #1 | `powd` sin `checked` |
//! | `the_partial_gap_target_needs_a_partial_phase_that_happened` | #9 | objetivo de una fase que no se vivió |

use chrono::NaiveDate;
use futurefin_engine::{
    project_net_worth_series, AllocationKind, AllocationRule, EngineError, FireNeed, FireTarget,
    PartialPhase, PensionSchedule, PhasePlan, ProjectionInput, RetirementTrigger, SimAsset,
    SpendMode, TargetBasis, TaxBracket, WithdrawalRule,
};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("literal decimal válido")
}

/// Escala del ahorro española 2025-26, la misma de `common/cases.rs`.
fn es() -> Vec<TaxBracket> {
    [
        (Some("6000"), "19"),
        (Some("50000"), "21"),
        (Some("200000"), "23"),
        (Some("300000"), "27"),
        (None, "30"),
    ]
    .into_iter()
    .map(|(up_to, pct)| TaxBracket {
        up_to: up_to.map(dec),
        pct: dec(pct),
    })
    .collect()
}

fn asset(i: u128, value: &str, basis: Option<&str>, liquid: bool, rate: &str) -> SimAsset {
    SimAsset {
        id: Uuid::from_u128(i),
        value: dec(value),
        purchase_price: basis.map(dec),
        is_liquid: liquid,
        expected_annual_return_percent: Some(dec(rate)),
    }
}

fn remainder_to(target_index: usize) -> AllocationRule {
    AllocationRule {
        target_index,
        kind: AllocationKind::Remainder,
        amount: None,
        cap: None,
    }
}

fn base(horizon: u32, income: &str, expense: &str, assets: Vec<SimAsset>) -> ProjectionInput {
    ProjectionInput {
        ref_date: NaiveDate::from_ymd_opt(2026, 9, 1).expect("fecha válida"),
        horizon_months: horizon,
        annual_inflation_percent: Decimal::ZERO,
        tax_brackets: es(),
        taxes_enabled: true,
        taxable_gain_ratio: Decimal::ONE,
        income_regular_monthly: dec(income),
        expense_regular_monthly: dec(expense),
        assets,
        allocation_rules: vec![remainder_to(0)],
        liabilities: Vec::new(),
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        phase_plan: PhasePlan::classic(Decimal::ZERO, dec(expense)),
        fire_target: None,
    }
}

// =================================================================================================
// Hallazgo #2 — el aterrizaje exacto no es un agotamiento
// =================================================================================================

/// **La cartera que se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto
/// posterior no se ha agotado: ha cumplido.**
///
/// El hogar: 2.000 €/mes de gasto, pensión de 2.500 € desde el índice 120, objetivo puente sin
/// descuento ⇒ `T(0) = 120 × 2.000 = 240.000 €` exactos. Con exactamente ese capital al 0 %, la
/// venta del mes 120 consume el último euro y desde el 121 la pensión paga sola: 500 €/mes de
/// sobra durante 120 meses ⇒ 60.000 € al final del horizonte, y **ni un euro de necesidad sin
/// cubrir**.
///
/// Hasta el pase de correcciones el motor publicaba `assets_depleted_month_index = Some(120)`
/// porque el predicado era `venta_bruta >= drenable` y el aterrizaje exacto cae del lado del
/// `>=`. Peor: la misma entrada daba `None` en `Decimal` y `Some(120)` en `f64` —el tipo sobre el
/// que corre cada camino de Monte Carlo—, así que el mismo plan salía «arruinado» en el fan chart
/// y «perfecto» en la línea.
///
/// Un euro menos de capital SÍ es un agotamiento, y el test lo comprueba: el discriminante es la
/// necesidad sin cubrir, no el filo de la comparación.
#[test]
fn an_exact_landing_that_covers_every_later_need_is_not_a_depletion() {
    let build = |capital: &str| {
        let mut input = base(
            240,
            "0",
            "2000",
            vec![asset(1, capital, Some("0"), true, "0")],
        );
        input.taxes_enabled = false;
        input.tax_brackets = Vec::new();
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        input.phase_plan.target_basis = TargetBasis::BridgeToPension;
        input.phase_plan.bridge_discount_annual_pct = Decimal::ZERO;
        input.phase_plan.pension = Some(PensionSchedule {
            start_index: 120,
            monthly_today: dec("2500"),
            indexed: true,
            fraction_while_partial: Decimal::ZERO,
        });
        input.fire_target = Some(FireTarget {
            need: FireNeed::ExpenseMinusPension {
                expense_monthly: dec("2000"),
                pension_monthly: Decimal::ZERO,
            },
            swr_pct: dec("4"),
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::ZERO,
            debt_payments_remaining: Vec::new(),
        });
        project_net_worth_series(&input).expect("simula")
    };

    // (a) El aterrizaje EXACTO: cartera a cero en el mes 120 y nada sin cubrir después.
    let exact = build("240000");
    assert_eq!(
        exact.assets_depleted_month_index, None,
        "vaciar la cartera el mes en que la pensión toma el relevo no es agotarla"
    );
    assert_eq!(exact.uncovered_deficit_total, Decimal::ZERO);
    assert_eq!(exact.liquid_worth[120], Decimal::ZERO, "sí se vacía");
    assert_eq!(
        exact.liquid_worth[240],
        dec("60000"),
        "y desde el 121 la pensión deja 500 €/mes"
    );

    // (b) Un euro menos SÍ deja necesidad sin cubrir, y entonces el mes se publica.
    let short = build("239999");
    assert_eq!(short.assets_depleted_month_index, Some(120));
    assert_eq!(short.uncovered_deficit_total, Decimal::ONE);

    // (c) Y un céntimo más ni se vacía ni se agota.
    let spare = build("240000.01");
    assert_eq!(spare.assets_depleted_month_index, None);
    assert_eq!(spare.uncovered_deficit_total, Decimal::ZERO);
}

// =================================================================================================
// Hallazgo #3 — el techo de la regla que no ataba en la vía mixta
// =================================================================================================

/// Dos hogares con el MISMO valor total, la MISMA base de coste total y la MISMA necesidad, que
/// solo difieren en cómo está repartida la base: uniforme (`g = 0,5` en los dos activos) o mixta
/// (`g = 0` y `g = 1`).
fn b1(mixed: bool, mode: SpendMode) -> ProjectionInput {
    let assets = if mixed {
        vec![
            asset(1, "500", Some("500"), true, "0"),
            asset(2, "500", Some("0"), true, "0"),
        ]
    } else {
        vec![
            asset(1, "500", Some("250"), true, "0"),
            asset(2, "500", Some("250"), true, "0"),
        ]
    };
    let mut input = base(4, "0", "2000", assets);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
    // `pct = 1440` da `allowed_gross = 1.200 €`: POR ENCIMA de todo lo vendible (1.000) y por
    // DEBAJO del bruto que la necesidad pediría (2.223,46). Ese es exactamente el hueco del bug.
    // No es un valor que la API acepte (`MAX_WITHDRAWAL_PCT = 20`): es el caso mínimo que aísla
    // la rama, y `guardrails_under_a_binding_allowance_classify_on_both_paths` lo repite con
    // parámetros reales.
    input.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance { pct: dec("1440") };
    input.phase_plan.spend_mode = mode;
    input
}

/// **Un techo de retirada que la cartera no llega a cubrir sigue siendo un RECORTE, no una deuda
/// del hogar** — también cuando la venta va por el paseo mixto.
///
/// La regla autoriza vender 1.200 € brutos; la cartera solo tiene 1.000 y los vende enteros. De
/// los 2.000 € de necesidad, una parte la rechazó la REGLA (recorte, informativo) y otra no la
/// pudieron fundar los ACTIVOS (descubierto, que resta patrimonio). La vía escalar lo repartía
/// bien; la mixta decidía si el techo ataba comparando contra `dd.gross_monthly` —que el paseo ya
/// había recortado a la capacidad— y por tanto **descartaba el techo en silencio**: los 916 € que
/// la regla había rechazado se contabilizaban como descubierto y salían del patrimonio.
///
/// Con la venta byte a byte idéntica en los dos hogares (los dos vacían la cartera y obtienen
/// 905 €), el patrimonio publicado difería en 916 €.
///
/// # Por qué los dos hogares NO tienen que dar el mismo número
///
/// Solo coinciden en lo que se vende. El neto de un techo POR ENCIMA de la capacidad se tasa con
/// la `g` MARGINAL —la del último tramo con material, que es el euro siguiente que se vendería—,
/// y ahí los dos hogares son distintos de verdad: el uniforme tiene `g = 0,5` en todo, el mixto
/// tiene el tramo barato ya agotado y `g = 1` en el margen. De ahí los 21 € que quedan (937 vs
/// 916 de recorte, 158 vs 179 de descubierto). No es un residuo del bug: es la misma asimetría
/// que ya existe cuando la venta es parcial (vender 500 € netea 452,50 en el uniforme y 500 en el
/// mixto).
#[test]
fn the_binding_allowance_is_a_cut_on_the_mixed_path_too() {
    for mode in [SpendMode::Ceiling, SpendMode::RuleIsSpend] {
        let uniform = project_net_worth_series(&b1(false, mode)).expect("simula");
        let mixed = project_net_worth_series(&b1(true, mode)).expect("simula");

        // La VENTA es la misma en los dos: mismo neto obtenido, cartera vacía, mismo mes.
        assert_eq!(uniform.withdrawal[1], dec("905.0000"));
        assert_eq!(mixed.withdrawal[1], dec("905.00"));
        assert_eq!(uniform.assets_depleted_month_index, Some(1));
        assert_eq!(mixed.assets_depleted_month_index, Some(1));

        // Y el reparto entre recorte y descubierto ya existe en las DOS vías. Antes: recorte 0 y
        // descubierto 1.095 en la mixta (todo deuda), contra 916 / 179 en la uniforme.
        assert_eq!(uniform.withdrawal_shortfall[1], dec("916.0000"));
        assert_eq!(uniform.uncovered_deficit_total, dec("179.0000"));
        assert_eq!(mixed.withdrawal_shortfall[1], dec("937.00"));
        assert_eq!(mixed.uncovered_deficit_total, dec("158.00"));

        // El patrimonio publicado ya no se lleva el recorte por delante: la brecha entre los dos
        // hogares baja de 916 € a los 21 € de la `g` marginal.
        assert_eq!(uniform.net_worth[4], dec("-179.0000"));
        assert_eq!(mixed.net_worth[4], dec("-158.00"));

        // Y las tres magnitudes cierran contra la necesidad, en los dos.
        for out in [&uniform, &mixed] {
            assert_eq!(
                out.withdrawal[1] + out.withdrawal_shortfall[1] + out.unmet_need[1],
                dec("2000"),
                "retirada + recorte + descubierto = necesidad"
            );
        }
    }
}

/// El mismo hallazgo con **parámetros que la API sí acepta**: guardarraíles 4 / 20 / 10 sobre una
/// cartera que se desploma un 95 % anual. En el mes 19 el techo (500 €) está por encima de todo
/// lo vendible (367,69 €) y por debajo de la necesidad (2.000 €) — el hueco exacto del bug.
#[test]
fn guardrails_under_a_binding_allowance_classify_on_both_paths() {
    let build = |mixed: bool| {
        let assets = if mixed {
            vec![
                asset(1, "180000", Some("100000"), true, "-95"),
                asset(2, "20000", Some("0"), true, "-95"),
            ]
        } else {
            vec![
                asset(1, "180000", Some("90000"), true, "-95"),
                asset(2, "20000", Some("10000"), true, "-95"),
            ]
        };
        let mut input = base(60, "0", "2000", assets);
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        input.phase_plan.withdrawal = WithdrawalRule::Guardrails {
            pct: dec("4"),
            band_pct: dec("10"),
            adjust_pct: dec("25"),
        };
        project_net_worth_series(&input).expect("simula")
    };
    let uniform = build(false);
    let mixed = build(true);

    // Vía uniforme: el techo recorta 1.500 € y los activos dejan 132,31 € sin fundar.
    assert_eq!(
        uniform.withdrawal_shortfall[19],
        dec("1500.0000000000000000000000000")
    );
    assert_eq!(uniform.withdrawal[19].round_dp(4), dec("367.6884"));
    assert_eq!(uniform.unmet_need[19].round_dp(4), dec("132.3116"));

    // Vía mixta: ANTES publicaba recorte 0 y 1.674,80 € de descubierto — todo deuda del hogar.
    // Ahora recorta de verdad, con la `g` marginal poniendo precio al techo que no cabe.
    assert!(
        mixed.withdrawal_shortfall[19] > dec("1500"),
        "el techo tiene que atar también aquí: {}",
        mixed.withdrawal_shortfall[19]
    );
    assert!(
        mixed.unmet_need[19] < dec("200"),
        "y el descubierto ya no se come el recorte: {}",
        mixed.unmet_need[19]
    );
    for out in [&uniform, &mixed] {
        assert_eq!(
            out.withdrawal[19] + out.withdrawal_shortfall[19] + out.unmet_need[19],
            dec("2000"),
            "las tres magnitudes cierran contra la necesidad"
        );
    }
}

// =================================================================================================
// Hallazgo #4 — `rule_is_spend` no compra y vende el mismo fondo el mismo mes
// =================================================================================================

/// **El gasto que la regla ordena se paga primero con la caja del mes.**
///
/// El hogar: 1 M€ en un fondo con 500 k€ de base (`g = 0,5`), jubilado desde el mes 1, ingreso
/// 5.000 €, gasto 2.000 € ⇒ **3.000 € de superávit todos los meses**, y una regla
/// `percent_of_balance` al 4 % en modo `rule_is_spend` que ordena gastar ~2.993 € netos.
///
/// Hasta el pase de correcciones el mes hacía las dos cosas: la cascada metía los 3.000 € en el
/// fondo y acto seguido la venta sacaba 3.333 € brutos del MISMO fondo para financiar el gasto de
/// la regla. El hecho económico —gastar 2.993 € que la nómina ya había puesto sobre la mesa— no
/// mueve un euro de cartera, pero el ida y vuelta realiza plusvalía: **3.991,72 €/año de
/// impuesto**, ×10,7 el coste real. Ahora el gasto sale de la caja, la venta es 0 y el impuesto
/// también.
#[test]
fn rule_is_spend_funds_the_month_surplus_first() {
    let mut input = base(
        12,
        "5000",
        "2000",
        vec![asset(1, "1000000", Some("500000"), true, "0")],
    );
    input.phase_plan = PhasePlan::classic(dec("5000"), dec("2000"));
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
    input.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance { pct: dec("4") };
    input.phase_plan.spend_mode = SpendMode::RuleIsSpend;
    let out = project_net_worth_series(&input).expect("simula");

    // El gasto de la regla del mes 1: 4 %/12 de 1 M€ bruto = 3.333,33, neto 2.993,34.
    assert_eq!(
        out.withdrawal[1].round_dp(4),
        dec("2993.3357"),
        "la regla gasta lo mismo que antes: lo que cambia es de dónde sale"
    );
    assert_eq!(
        out.withdrawal_excess[1], out.withdrawal[1],
        "sin necesidad, todo el gasto de la regla es sobrante"
    );
    assert_eq!(out.withdrawal_shortfall[1], Decimal::ZERO);

    // **El impuesto anual es CERO**: el superávit (3.000 €) cubre el gasto (2.993,34 €) y no se
    // vende nada. La identidad del hogar lo cierra: 1.000.000 + 12×3.000 − Σ gastado = NW(12).
    let spent: Decimal = out.withdrawal.iter().copied().sum();
    let tax = dec("1000000") + dec("36000") - spent - out.net_worth[12];
    // Medido: **0,0029 €** al año, contra 3.991,72 antes. No es cero exacto porque el sobrante
    // que sí se queda invertido (≈7 €/mes) engorda el saldo y, en los últimos meses, el 4 % de la
    // regla se pasa por unos céntimos del superávit: ahí sí hay una venta, minúscula y real.
    assert!(
        tax.abs() < dec("0.01"),
        "el ida y vuelta ya no realiza plusvalía; impuesto anual medido = {tax} € (antes: 3.991,72)"
    );
    assert!(
        out.net_worth[12] > dec("1000000"),
        "y el patrimonio sube: {} (antes 996.072,52)",
        out.net_worth[12]
    );
    // La cartera no recibe la aportación que iba a salir el mismo mes: la base de coste no crece.
    assert_eq!(
        out.contributed_capital[12],
        dec("500078.53481539090402724015004"),
        "solo entra el sobrante que de verdad se queda invertido"
    );
}

// =================================================================================================
// Hallazgo #1 — el descuento del puente ya no panica
// =================================================================================================

/// **Un descuento de puente demasiado negativo es un error TIPADO, no un pánico.**
///
/// `bridge_discount_annual_pct` no se escribe: se deriva de la rentabilidad esperada ponderada de
/// los activos líquidos, y esa solo está acotada por `> −100`. Con `d` muy negativo el factor
/// `q(j) = (1+d/100)^{j/12}` se hunde hacia 0, el término descontado explota y `powd` —o la suma
/// sufijo, o el producto de la evaluación— se sale del rango de `Decimal`. Eso PANICABA, y salía
/// como un 500 opaco de `/v1/projection/series`.
///
/// La lectura suelta (`fire_target_at_month_index_with_plan`) degrada a la perpetuidad sobre la
/// necesidad íntegra, que es la misma degradación declarada de `p > MAX_BRIDGE_MONTHS`; la
/// SIMULACIÓN falla en voz alta, porque publicar ese objetivo sería publicar un plan distinto del
/// configurado.
#[test]
fn a_bridge_discount_too_negative_is_a_typed_error_not_a_panic() {
    let build = |d: &str, p: u32| {
        let mut input = base(
            240,
            "0",
            "2000",
            vec![asset(1, "500000", Some("0"), true, "0")],
        );
        input.taxes_enabled = false;
        input.tax_brackets = Vec::new();
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        input.phase_plan.target_basis = TargetBasis::BridgeToPension;
        input.phase_plan.bridge_discount_annual_pct = dec(d);
        input.phase_plan.pension = Some(PensionSchedule {
            start_index: p,
            monthly_today: dec("1200"),
            indexed: false,
            fraction_while_partial: Decimal::ZERO,
        });
        input.fire_target = Some(FireTarget {
            need: FireNeed::ExpenseMinusPension {
                expense_monthly: dec("2000"),
                pension_monthly: Decimal::ZERO,
            },
            swr_pct: dec("4"),
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::ZERO,
            debt_payments_remaining: Vec::new(),
        });
        input
    };

    // El caso que panicaba en `powd` («Pow overflowed»): pensión a 20 años, d = −99 %.
    assert!(matches!(
        project_net_worth_series(&build("-99", 240)),
        Err(EngineError::BridgeDiscountOverflow)
    ));
    // Y el que panicaba en la suma sufijo («Addition overflowed»): pensión a 70 años, d = −60 %.
    // Es el alcanzable de verdad: un solo activo líquido al −60 % con el descuento por defecto.
    assert!(matches!(
        project_net_worth_series(&build("-60", 840)),
        Err(EngineError::BridgeDiscountOverflow)
    ));
    // Un descuento razonable sigue simulando.
    assert!(project_net_worth_series(&build("5", 240)).is_ok());
    // Y la LECTURA suelta nunca panica: degrada a la perpetuidad sobre la necesidad íntegra.
    let degraded = build("-99", 240);
    let t = futurefin_engine::fire_target_at_month_index_with_plan(
        degraded.fire_target.as_ref(),
        &degraded.phase_plan,
        0,
    );
    assert_eq!(
        t,
        futurefin_engine::fire_target_at_month_index(degraded.fire_target.as_ref(), 0),
        "la lectura degradada ES el objetivo de 4.15.0 sin pensión"
    );
}

// =================================================================================================
// Hallazgo #9 — el objetivo del hueco necesita una fase parcial que ocurriera
// =================================================================================================

/// **`partial_gap_target` solo existe si la media jornada se llegó a vivir.**
///
/// El hogar declara media jornada a partir del mes 60, pero con 598.000 € y 6.000 €/mes de
/// superávit cruza su número FIRE (600.000 €) en el mes 2 y se jubila del todo **58 meses antes**.
/// La fase parcial no ocurre —`partial_retirement_month_index` es `None` y `phase_transitions` no
/// la menciona— y sin embargo el objetivo del hueco se publicaba: 270.000 €, calculados de la
/// fase DECLARADA. Su gemelo `partial_phase_capital_growing` ya se gateaba así.
#[test]
fn the_partial_gap_target_needs_a_partial_phase_that_happened() {
    let mut input = base(
        120,
        "8000",
        "2000",
        vec![asset(1, "598000", Some("598000"), true, "0")],
    );
    input.taxes_enabled = false;
    input.tax_brackets = Vec::new();
    input.phase_plan = PhasePlan::classic(Decimal::ZERO, dec("2000"));
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 60,
        income_monthly: dec("1100"),
        expense_basis: futurefin_engine::ExpenseBasis::Retirement,
    });
    input.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: dec("2000"),
            pension_monthly: Decimal::ZERO,
        },
        swr_pct: dec("4"),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    let out = project_net_worth_series(&input).expect("simula");

    assert_eq!(out.retirement_month_index, Some(2), "cruza en el mes 2");
    assert_eq!(
        out.partial_retirement_month_index, None,
        "la fase no ocurre"
    );
    assert_eq!(
        out.partial_gap_target, None,
        "y su objetivo tampoco: antes publicaba 270.000 € de una fase que nadie vivió"
    );
    assert!(!out.partial_phase_capital_growing, "el gemelo ya lo hacía");

    // Con la fase parcial de verdad (sin cruce que la adelante), el objetivo vuelve a existir.
    let mut lived = input.clone();
    lived.assets = vec![asset(1, "1000", Some("1000"), true, "0")];
    lived.income_regular_monthly = dec("2500");
    let out = project_net_worth_series(&lived).expect("simula");
    assert_eq!(out.partial_retirement_month_index, Some(60));
    assert!(
        out.partial_gap_target.is_some(),
        "vivida la fase, el hueco vuelve a ser una lectura legítima"
    );
}
