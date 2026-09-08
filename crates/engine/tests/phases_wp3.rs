//! **Fases, pensión con fecha y solves** (WP3 de 5.0.0, §B.1/§B.3/§B.7 del plan de la issue #207).
//!
//! Todo lo que se afirma aquí está **predicho a mano en el comentario que lo precede**, con la
//! aritmética a la vista. Es la disciplina de `futurefin-research-methodology`: un test que
//! compara el motor consigo mismo solo pinea lo que el motor hace hoy; un test con el número
//! escrito antes de ejecutarlo comprueba que hace lo que se pidió.
//!
//! Por eso casi todos los casos van con rentabilidad 0 %, inflación 0 % y sin impuestos: no
//! porque sea realista, sino porque así **cada euro de la serie es una suma que cabe en una línea**
//! y una discrepancia señala el mes exacto. Los caminos con fiscalidad, inflación y `powd` los
//! cubren los pines dorados (`golden_pins.rs`), que son otra herramienta para otro trabajo.

#[path = "common/cases.rs"]
mod cases;

use cases::{base_input, mk_asset, mk_liab, projection_cases_all, rule_remainder};
use futurefin_engine::{
    max_extra_monthly_expense_keeping_date, project_net_worth_series, retirement_delay_months,
    BridgeCap, EngineWarning, ExpenseBasis, FireNeed, FireTarget, IncomePause, InitialRateGate,
    PartialPhase, PathFailure, PensionSchedule, Phase, ProjectionInput, RepaymentModel,
    RetirementTrigger, WithdrawalRule,
};
use rust_decimal::Decimal;

fn d(n: i64) -> Decimal {
    Decimal::from(n)
}

/// Un objetivo plano: sin impuestos, sin inflación y sin deuda, `annual_net_today/SWR` y punto.
fn flat_target(annual_net_today: i64, swr_pct: i64) -> FireTarget {
    FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: d(annual_net_today),
        },
        swr_pct: d(swr_pct),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    }
}

/// Un hogar de laboratorio: un único activo líquido al 0 %, una regla `remainder` y nada más.
fn lab(horizon: u32, income: i64, expense: i64, asset_value: i64) -> ProjectionInput {
    base_input(
        horizon,
        d(income),
        d(expense),
        vec![mk_asset(1, d(asset_value), true, Some(Decimal::ZERO))],
        vec![rule_remainder(0)],
    )
}

// =============================================================================================
// A · Pensión con fecha como INGRESO
// =============================================================================================

/// **La pensión entra en caja en un mes ACUMULANDO**, no solo jubilado (§B.1 paso 3).
///
/// Predicho: horizonte 3, ingreso 1.000 = gasto 1.000 (caja recurrente 0), pensión PLANA de 500
/// desde el índice 1. El mes `k` mira el índice `k−1`, así que:
///
/// | mes | índice | pensión | caja | activo al cierre |
/// |---|---|---|---|---|
/// | 1 | 0 | 0 | 0 | 0 |
/// | 2 | 1 | 500 | +500 | **500** |
/// | 3 | 2 | 500 | +500 | **1.000** |
///
/// Y `pension_start_month_index = 2`: el mes del BUCLE, 1-based, `start_index + 1`.
#[test]
fn a_dated_pension_is_income_while_still_accumulating() {
    let mut input = lab(3, 1_000, 1_000, 0);
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 1,
        monthly_today: d(500),
        indexed: false,
        fraction_while_partial: Decimal::ZERO,
    });
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(out.liquid_worth, vec![d(0), d(0), d(500), d(1_000)]);
    assert_eq!(out.pension_start_month_index, Some(2));
    assert_eq!(out.retirement_month_index, None, "sin objetivo no hay cruce");
    assert_eq!(out.phase_transitions, vec![(Phase::Accumulating, 0)]);
}

/// Una pensión INDEXADA se infla con el MISMO factor que el gasto del bucle (`f(k−1)`), y una
/// pensión cuyo `start_index` cae fuera del horizonte no tiene mes: `None`, no un mes inventado.
#[test]
fn an_indexed_pension_uses_the_loops_inflation_factor() {
    // Inflación 100 % anual ⇒ `f(12) = 2` exacto (`powd` enruta el año entero por `checked_powu`).
    let mut input = lab(13, 0, 0, 0);
    input.annual_inflation_percent = d(100);
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 12,
        monthly_today: d(1_000),
        indexed: true,
        fraction_while_partial: Decimal::ZERO,
    });
    let out = project_net_worth_series(&input).unwrap();
    // Solo el mes 13 (índice 12) cobra, y cobra 1.000·f(12) = 2.000.
    assert_eq!(out.liquid_worth[12], Decimal::ZERO);
    assert_eq!(out.liquid_worth[13], d(2_000));
    assert_eq!(out.pension_start_month_index, Some(13));

    let mut short = input.clone();
    short.horizon_months = 6;
    short.planning_monthly_cash_adjustment = vec![Decimal::ZERO; 6];
    assert_eq!(
        project_net_worth_series(&short).unwrap().pension_start_month_index,
        None,
        "la pensión existe en el plan pero esta simulación no llega a verla"
    );
}

// =============================================================================================
// C · Fase parcial
// =============================================================================================

/// **El mes exacto en que la media jornada conmuta ingreso Y gasto** (§B.1, D10).
///
/// Predicho: horizonte 6, ingreso regular 3.000, gasto regular 2.000, activo 0, sin objetivo.
/// Media jornada desde el mes 4 con ingreso 1.100 y `expense_basis = Retirement`, con el gasto de
/// jubilación en 1.000.
///
/// | mes | fase | ingreso | gasto | caja | activo |
/// |---|---|---|---|---|---|
/// | 1-3 | acumula | 3.000 | 2.000 | +1.000 | 1.000 / 2.000 / **3.000** |
/// | 4-6 | parcial | 1.100 | 1.000 | +100 | 3.100 / 3.200 / **3.300** |
///
/// El capital CRECE en la fase, así que `partial_phase_capital_growing` es `true` y no hay aviso.
#[test]
fn the_partial_phase_switches_income_and_expense_on_its_month() {
    let mut input = lab(6, 3_000, 2_000, 0);
    input.phase_plan.expense_retirement_monthly = d(1_000);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 4,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(
        out.liquid_worth,
        vec![d(0), d(1_000), d(2_000), d(3_000), d(3_100), d(3_200), d(3_300)]
    );
    assert_eq!(out.partial_retirement_month_index, Some(4));
    assert_eq!(
        out.phase_transitions,
        vec![(Phase::Accumulating, 0), (Phase::Partial, 4)]
    );
    assert!(out.partial_phase_capital_growing);
    assert!(out.warnings.is_empty());
    // Ningún mes vendió nada: la fase parcial va en superávit.
    assert!(out.withdrawal.iter().all(|w| w.is_zero()));
}

/// La misma fase con `expense_basis = Regular` se queda con el gasto de siempre y **come capital**:
/// 1.100 − 2.000 = −900 €/mes vendidos de la cartera, sin techo (la regla de retirada gobierna la
/// jubilación, no la media jornada).
///
/// Predicho: activo 3.000 al entrar en el mes 4 ⇒ 2.100 / 1.200 / **300**. Aviso
/// `PartialPhaseCapitalShrinking` y `partial_phase_capital_growing = false`.
#[test]
fn a_partial_phase_that_eats_capital_warns_and_sells_without_a_ceiling() {
    let mut input = lab(6, 3_000, 2_000, 0);
    input.phase_plan.expense_retirement_monthly = d(1_000);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 4,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Regular,
    });
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(
        out.liquid_worth,
        vec![d(0), d(1_000), d(2_000), d(3_000), d(2_100), d(1_200), d(300)]
    );
    assert_eq!(out.withdrawal[4], d(900));
    assert!(!out.partial_phase_capital_growing);
    assert_eq!(
        out.warnings,
        vec![EngineWarning::PartialPhaseCapitalShrinking]
    );
    assert!(
        out.withdrawal_shortfall.iter().all(|s| s.is_zero()),
        "sin techo no hay recorte: el hogar gasta lo declarado"
    );
}

/// La media jornada cobra la FRACCIÓN declarada de la pensión (D8), y eso se ve en la CAJA.
///
/// Predicho: gasto parcial 2.000, ingreso 1.100, pensión 1.200 al 50 % ⇒ 600. La caja de la fase
/// es `1.100 + 600 − 2.000 = −300 €/mes`, que sale de la cartera.
///
/// (La lectura `partial_gap_target`, que capitalizaba ese hueco a `300·12/0,04` = 90.000 €, se
/// retiró en E4 junto con el resto del objetivo como criterio.)
#[test]
fn the_partial_phase_collects_its_share_of_the_pension() {
    let mut input = lab(4, 3_000, 2_000, 0);
    input.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: d(2_000),
            pension_monthly: Decimal::ZERO,
        },
        ..flat_target(0, 4)
    });
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.crossing_is_reading_only = true;
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 3,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    input.phase_plan.pension = Some(PensionSchedule {
        start_index: 0,
        monthly_today: d(1_200),
        indexed: false,
        fraction_while_partial: Decimal::new(5, 1),
    });
    let out = project_net_worth_series(&input).unwrap();

    // Meses 1-2 (acumulando): 3.000 + 1.200 − 2.000 = +2.200. Meses 3-4 (parcial):
    // 1.100 + 600 − 2.000 = −300, vendidos de la cartera.
    assert_eq!(
        out.liquid_worth,
        vec![d(0), d(2_200), d(4_400), d(4_100), d(3_800)]
    );
}

// =============================================================================================
// D · `crossing_is_reading_only` (D17)
// =============================================================================================

/// **El cruce deja de jubilar y se queda en lectura.**
///
/// Predicho: 1.000.000 € líquidos, objetivo 600.000 (24.000/0,04) ⇒ se cruza en el mes 1.
/// - Con la bandera: NO se jubila; ingreso 3.000 − gasto 2.000 = +1.000 €/mes ⇒
///   `liquid(12) = 1.012.000`, `retirement_month_index = None`, cruce anotado en el mes 1.
/// - Sin la bandera: jubilado en el mes 1, ingreso de jubilación 0 − gasto 2.000 ⇒ 12 ventas de
///   2.000 ⇒ `liquid(12) = 976.000`.
#[test]
fn a_reading_only_crossing_does_not_retire_anyone() {
    let mut input = lab(12, 3_000, 2_000, 1_000_000);
    input.fire_target = Some(flat_target(24_000, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(120);
    input.phase_plan.crossing_is_reading_only = true;

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.retirement_month_index, None);
    assert_eq!(
        out.liquid_crossing_month_index,
        Some(1),
        "el cruce SÍ se anota: es la lectura que el chart necesita"
    );
    assert_eq!(out.liquid_worth[12], d(1_012_000));
    assert_eq!(out.phase_transitions, vec![(Phase::Accumulating, 0)]);

    let mut retiring = input.clone();
    retiring.phase_plan.crossing_is_reading_only = false;
    let out2 = project_net_worth_series(&retiring).unwrap();
    assert_eq!(out2.retirement_month_index, Some(1));
    assert_eq!(out2.liquid_worth[12], d(976_000));
}

/// **Jubilarse por edad con el líquido corto ya no es un AVISO: es un fallo del camino** (E1).
///
/// El mismo hogar que hasta E1 emitía `retire_at_age_underfunded` comparando `L(R−1)` con el
/// objetivo de perpetuidad. Ahora el criterio es el que la literatura usa —la tasa inicial— y el
/// resultado no es una etiqueta al lado de la curva: el camino FALLA, y su fallo es lo que el
/// sorteo cuenta para decidir la fecha.
///
/// Predicho a mano: ingreso 3.000 − gasto 2.000 ⇒ +1.000 €/mes sobre 100.000 ⇒ `L(5) = 105.000`.
/// Jubilado en el mes 6 con gasto de 2.000 e ingreso 0 ⇒ necesidad ordinaria anual 24.000 €.
/// Tope al 4 %: `0,04 × 105.000 = 4.200 €/año` ⇒ 24.000 > 4.200 ⇒ falla en el mes 6.
/// **Sin puerta, el mismo hogar no falla por nada** y no emite un solo aviso.
#[test]
fn retiring_by_age_with_a_short_liquid_fails_the_initial_rate_gate() {
    let mut input = lab(12, 3_000, 2_000, 100_000);
    input.fire_target = Some(flat_target(24_000, 4)); // objetivo 600.000
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(6);
    input.phase_plan.crossing_is_reading_only = true;
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: d(4),
        bridge: None,
    });

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.retirement_month_index, Some(6));
    assert_eq!(out.liquid_crossing_month_index, None, "nunca se cruza");
    assert_eq!(out.liquid_worth[5], d(105_000), "la aritmética del predicho");
    assert_eq!(out.failure_month_index, Some(6));
    assert_eq!(out.failure_kind, Some(PathFailure::InitialRateExceeded));
    assert!(
        out.warnings.is_empty(),
        "el aviso de D17 se retiró: el veredicto es el fallo del camino"
    );

    // Sin puerta no hay comparación, y por tanto no hay fallo: es la semántica de 4.15.0 y la
    // razón de que `pins-4.15.json` no se mueva.
    let mut ungated = input.clone();
    ungated.phase_plan.initial_rate = None;
    let out2 = project_net_worth_series(&ungated).unwrap();
    assert_eq!(out2.failure_month_index, None);
    assert_eq!(out2.failure_kind, None);
    assert_eq!(
        out2.liquid_worth, out.liquid_worth,
        "la puerta DIAGNOSTICA: no mueve un euro de la simulación"
    );
}

// =============================================================================================
// D bis · Puerta de tasa inicial y fallo por camino (E1, correcciones C1/C2)
// =============================================================================================

/// **La tasa inicial se juzga en `R` y NUNCA se vuelve a juzgar** (C1).
///
/// Es la corrección medida por el panel adversarial: comprobar el SWR mes a mes sobre el saldo
/// vivo convierte cualquier bajada del mercado en un fallo retroactivo, y en la demo ponía la
/// fecha válida en la edad de la pensión.
///
/// Predicho: 700.000 € al 0 %, +1.000 €/mes hasta jubilarse en el mes 2 ⇒ `L(1) = 701.000`.
/// Necesidad ordinaria anual 24.000 €; tope al 4 % = 28.040 € ⇒ pasa la puerta. Después el hogar
/// drena 2.000 €/mes los meses 2 a 120 (119 meses) ⇒ `L(120) = 701.000 − 238.000 = 463.000`, muy
/// por debajo de los 600.000 que el 4 % exigiría — y aun así el camino NO falla.
#[test]
fn the_initial_rate_gate_fires_in_r_only_and_never_after() {
    let mut input = lab(120, 3_000, 2_000, 700_000);
    input.phase_plan.expense_retirement_monthly = d(2_000);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(2);
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: d(4),
        bridge: None,
    });

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.retirement_month_index, Some(2));
    assert_eq!(out.liquid_worth[1], d(701_000));
    assert_eq!(out.liquid_worth[120], d(463_000));
    assert!(
        out.liquid_worth[120] < d(600_000),
        "el líquido cae por debajo de lo que el 4 % exigiría: 24.000/0,04 = 600.000"
    );
    assert_eq!(
        out.failure_month_index, None,
        "la tasa INICIAL no se recomprueba: un mes malo no jubila retroactivamente a nadie"
    );

    // Un euro menos de capital de partida y la puerta sí ata en R: 600.000 + 1.000 del mes 1 =
    // 601.000 ⇒ tope 24.040 ≥ 24.000, pasa; con 598.000 ⇒ 599.000 ⇒ tope 23.960 < 24.000, falla.
    let mut short = input.clone();
    short.assets[0].value = d(598_000);
    let out2 = project_net_worth_series(&short).unwrap();
    assert_eq!(out2.liquid_worth[1], d(599_000));
    assert_eq!(out2.failure_month_index, Some(2));
    assert_eq!(out2.failure_kind, Some(PathFailure::InitialRateExceeded));
}

/// **El puente sube el tope solo si la pensión llega a tiempo** (C2).
///
/// Predicho: 400.000 € líquidos, jubilado en el mes 1 con gasto 2.000 e ingreso 0 ⇒ necesidad
/// ordinaria anual 24.000 €. Tope al SWR (4 %) = 16.000 € ⇒ ata. Tope del puente (8 %) = 32.000 €
/// ⇒ no ata. Con `max_years = 5` (60 meses) el puente aplica si la pensión entra en caja como
/// mucho en el mes 61: `start_index = 48` ⇒ mes 49 ⇒ aplica; `start_index = 72` ⇒ mes 73 ⇒ ya no.
#[test]
fn the_bridge_cap_applies_only_if_the_pension_is_within_max_years() {
    let build = |pension_start_index: u32| {
        let mut input = lab(12, 0, 2_000, 400_000);
        input.phase_plan.expense_retirement_monthly = d(2_000);
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        input.phase_plan.pension = Some(PensionSchedule {
            start_index: pension_start_index,
            monthly_today: d(1_500),
            indexed: false,
            fraction_while_partial: Decimal::ZERO,
        });
        input.phase_plan.initial_rate = Some(InitialRateGate {
            swr_pct: d(4),
            bridge: Some(BridgeCap {
                max_pct: d(8),
                max_years: 5,
            }),
        });
        project_net_worth_series(&input).unwrap()
    };

    let close = build(48);
    assert_eq!(
        close.failure_month_index, None,
        "la pensión llega en el mes 49, dentro de los 5 años: rige el tope del puente (8 %)"
    );

    let far = build(72);
    assert_eq!(
        far.failure_month_index,
        Some(1),
        "la pensión llega en el mes 73, fuera de la ventana: rige el SWR (4 %)"
    );
    assert_eq!(far.failure_kind, Some(PathFailure::InitialRateExceeded));

    // Y sin puente declarado, la pensión cercana no regala tope: el SWR ata igual.
    let mut plain = lab(12, 0, 2_000, 400_000);
    plain.phase_plan.expense_retirement_monthly = d(2_000);
    plain.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
    plain.phase_plan.pension = Some(PensionSchedule {
        start_index: 48,
        monthly_today: d(1_500),
        indexed: false,
        fraction_while_partial: Decimal::ZERO,
    });
    plain.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: d(4),
        bridge: None,
    });
    let out = project_net_worth_series(&plain).unwrap();
    assert_eq!(out.failure_month_index, Some(1));
    assert_eq!(out.failure_kind, Some(PathFailure::InitialRateExceeded));
}

/// **F3 mira la necesidad ORDINARIA, no el déficit de caja** (C1): una cuota de hipoteca no
/// dispara «la regla se queda por debajo de tu gasto».
///
/// Predicho: 600.000 € líquidos, jubilado en el mes 1, gasto 1.000 e ingreso 0 ⇒ necesidad
/// ORDINARIA 1.000 €/mes. Hipoteca sin intereses de 1.500 €/mes ⇒ el déficit de CAJA es 2.500.
/// La regla `percent_of_balance` al 4 % permite `0,04 × 600.000/12 = 2.000 €/mes` brutos (sin
/// impuestos, netos también): por encima de la necesidad ordinaria (1.000) y por debajo del
/// déficit de caja (2.500).
///
/// - No hay fallo: el gasto de vivir está cubierto de sobra.
/// - Y el recorte de la regla SÍ existe (500 €/mes): la hipoteca ata la venta sin ser un fallo.
///
/// El control: subir el gasto de jubilación a 2.500 € —sin tocar la hipoteca— sí dispara F3.
///
/// Las dos ramas se deciden en el mes 1, que es `R` (`AtMonth(1)`): desde C10 ese es el ÚNICO mes
/// en que F3 se juzga, así que este test mide qué MAGNITUD compara —la ordinaria, no la de caja—
/// y `f3_fires_only_in_the_first_retired_month_never_after` mide CUÁNDO.
#[test]
fn f3_ignores_the_mortgage_and_looks_at_the_ordinary_need() {
    let build = |expense_retirement: i64| {
        let mut input = lab(6, 0, 1_000, 600_000);
        input.phase_plan.expense_retirement_monthly = d(expense_retirement);
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
        input.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance { pct: d(4) };
        input.liabilities = vec![mk_liab(
            d(100_000),
            d(1_500),
            None,
            RepaymentModel::FixedPayments,
            None,
        )];
        project_net_worth_series(&input).unwrap()
    };

    let covered = build(1_000);
    assert_eq!(
        covered.failure_month_index, None,
        "la regla permite 2.000 y el gasto ordinario es 1.000: la cuota no es hambre"
    );
    assert_eq!(
        covered.withdrawal[1],
        d(2_000),
        "se vende el techo de la regla, que la necesidad de caja (2.500) supera"
    );
    assert_eq!(
        covered.withdrawal_shortfall[1],
        d(500),
        "el recorte de la regla existe —lo causa la cuota— y NO es un fallo"
    );

    let starved = build(2_500);
    assert_eq!(
        starved.failure_month_index,
        Some(1),
        "ahora el GASTO ordinario (2.500) sí supera lo que la regla permite (2.000)"
    );
    assert_eq!(starved.failure_kind, Some(PathFailure::RuleBelowNeed));
}

/// **F3 se juzga en `R` y NUNCA se vuelve a juzgar** (C10) — el gemelo exacto de
/// `the_initial_rate_gate_fires_in_r_only_and_never_after`, y por la misma razón.
///
/// Mes a mes, F3 no medía la salud del plan: medía una BARRERA. Con una regla por saldo el
/// permitido sigue al líquido, así que basta un mes flojo para que el permitido cruce por debajo
/// del gasto y el camino quede marcado para siempre — sobre 840 meses la probabilidad de tocar
/// esa barrera alguna vez tiende a 1 por la varianza, no por el plan. Medido sobre la demo
/// sintética («3,5 % del saldo» + `rule_is_spend`): el capital necesario hoy salía **2,52 M€**
/// frente a los 620 k€ de `fixed_real`, los 67 fallos de 2.500 caminos eran TODOS F3 y el primero
/// caía siempre antes de la pensión. Con F3 solo en `R`, el mismo hogar pide 860 k€.
///
/// **La regla POR SALDO no deja de recortar** — solo deja de ser un fracaso: el recorte sigue
/// publicándose mes a mes en `withdrawal_shortfall`, que es informativo por contrato.
///
/// Predicho, con `pct = 6 %` (⇒ permitido = `L(k−1)/200`), jubilación forzada en el mes **3** —
/// para que la marca sea `R` y no «el mes 1»—, 600.000 € al 0 %, ingreso 3.000 y gasto regular
/// 2.000 (superávit 1.000 que la regla `remainder` reinvierte):
///
/// | mes | `L(k−1)` | permitido | necesidad | venta | recorte | `L(k)` |
/// |---|---|---|---|---|---|---|
/// | 1 | 600.000 | — (acumulando) | — | — | 0 | 601.000 |
/// | 2 | 601.000 | — (acumulando) | — | — | 0 | 602.000 |
/// | 3 = `R` | 602.000 | **3.010** | 3.000 | 3.000 | 0 | 599.000 |
/// | 4 | 599.000 | 2.995 | 3.000 | 2.995 | **5** | 596.005 |
///
/// El mes 4 es exactamente el caso que el modelo viejo marcaba como `rule_below_need`: hoy es un
/// recorte de 5 € y el camino **no falla**. El control es la desigualdad al revés: con 3.011 € de
/// gasto la regla ya no llega EN `R` (3.010 < 3.011) y ahí sí falla, en el mes 3.
#[test]
fn f3_fires_only_in_the_first_retired_month_never_after() {
    let build = |expense_retirement: i64| {
        let mut input = lab(24, 3_000, 2_000, 600_000);
        input.phase_plan.expense_retirement_monthly = d(expense_retirement);
        input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(3);
        input.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance { pct: d(6) };
        project_net_worth_series(&input).unwrap()
    };

    // (a) La regla CUBRE en `R` y se queda corta después: el camino no falla.
    let out = build(3_000);
    assert_eq!(out.retirement_month_index, Some(3), "`R` es el mes 3");
    assert_eq!(out.liquid_worth[2], d(602_000), "dos meses de superávit");
    assert_eq!(out.withdrawal[3], d(3_000), "el permitido (3.010) no ata en `R`");
    assert_eq!(out.withdrawal_shortfall[3], Decimal::ZERO);
    assert_eq!(out.liquid_worth[3], d(599_000));
    // El mes 4 SÍ recorta —y el modelo viejo lo habría publicado como `rule_below_need`—…
    assert_eq!(out.withdrawal[4], d(2_995), "el permitido baja a 2.995 y ata");
    assert_eq!(out.withdrawal_shortfall[4], d(5));
    assert_eq!(out.liquid_worth[4], d(596_005));
    assert!(
        out.withdrawal_shortfall[5] > Decimal::ZERO,
        "y sigue recortando los meses siguientes: el recorte no desaparece, deja de ser un fallo"
    );
    // …y aun así el camino NO falla: después de `R` el único motivo vivo es F1.
    assert_eq!(
        out.failure_month_index, None,
        "un recorte posterior a `R` es una LECTURA (`withdrawal_shortfall`), no un fracaso"
    );
    assert_eq!(out.failure_kind, None);
    assert_eq!(
        out.uncovered_deficit_total,
        Decimal::ZERO,
        "y no falta un euro de gasto: la cartera fundó todo lo que la regla dejó vender"
    );

    // (b) La inversa: la regla se queda corta YA en `R` (3.010 < 3.011) ⇒ F3, en el mes 3.
    let starved = build(3_011);
    assert_eq!(starved.failure_month_index, Some(3), "el fallo cae EN `R`");
    assert_eq!(starved.failure_kind, Some(PathFailure::RuleBelowNeed));
    assert_eq!(
        starved.withdrawal_shortfall[3],
        d(1),
        "1 € de recorte en `R` basta: la comparación es estricta"
    );
}

/// **Durante la media jornada solo puede fallar F1** (supuesto S1).
///
/// Las reglas de retirada se anclan en `L(R−1)`, que en la fase parcial todavía no existe, y la
/// tasa inicial es una propiedad de la fecha de jubilación TOTAL. Así que una fase parcial que se
/// come el capital falla por lo único que puede: quedarse sin cartera.
///
/// Predicho: 2.000 € líquidos, fase parcial desde el mes 1 con ingreso 500 y gasto (base de
/// jubilación) 2.000 ⇒ déficit de 1.500 €/mes. Mes 1: vende 1.500, quedan 500. Mes 2: necesita
/// 1.500 y solo hay 500 ⇒ 1.000 € sin cubrir ⇒ F1 en el mes 2.
///
/// Con una regla al 4 % —que permitiría `0,04 × 2.000/12 = 6,67 €/mes`, ridícula frente a los
/// 1.500 de necesidad— F3 dispararía en el mes 1 si aplicara durante la fase parcial. No aplica:
/// el fallo es del mes 2 y es F1.
#[test]
fn during_the_partial_phase_only_f1_can_fire() {
    let mut input = lab(6, 3_000, 2_000, 2_000);
    input.phase_plan.expense_retirement_monthly = d(2_000);
    // Jubilación total fuera del horizonte: la fase parcial es toda la simulación.
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(999);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 1,
        income_monthly: d(500),
        expense_basis: ExpenseBasis::Retirement,
    });
    input.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance { pct: d(4) };
    input.phase_plan.initial_rate = Some(InitialRateGate {
        swr_pct: d(4),
        bridge: None,
    });

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.partial_retirement_month_index, Some(1));
    assert_eq!(out.retirement_month_index, None, "nunca se jubila del todo");
    assert_eq!(out.liquid_worth[1], d(500));
    assert_eq!(out.unmet_need[2], d(1_000));
    assert_eq!(
        out.failure_month_index,
        Some(2),
        "F3 habría disparado en el mes 1 si la regla aplicara en media jornada; no aplica"
    );
    assert_eq!(out.failure_kind, Some(PathFailure::PortfolioDepleted));
    assert!(
        out.withdrawal_shortfall.iter().all(|v| v.is_zero()),
        "la fase parcial no pasa por la regla: no hay techo que recorte"
    );
}

/// **F1 gana el mes en que coincide con F3** (prioridad F1 > F2 > F3).
///
/// Los dos motivos describen el mismo mes desde dos sitios distintos —«la regla te deja sacar
/// menos de lo que gastas» y «la cartera no ha podido dar ni eso»— y el segundo es el que manda:
/// recortar el nivel de vida es una decisión; quedarse sin dinero, no.
///
/// **La coincidencia tiene que caer en `R`** (C10): desde que F3 solo se juzga en el primer mes
/// jubilado, un mes posterior no puede cumplir los dos y la prioridad no se ejercitaría. Por eso
/// la jubilación se fuerza en el mes **5**, que es justo el mes en que la pausa de ingresos deja
/// al hogar sin nómina — así el mismo mes es `R`, es F3 y es F1.
///
/// Predicho, sin reglas de asignación (el superávit no se reinvierte, así que el saldo se queda
/// quieto en 500 €) y con `guardrails` anclada en `L(R−1) = L(4) = 500` al 2.400 % anual ⇒
/// `W_R = 500 × 2.400/1.200 = 1.000 €/mes`, constante (sin IPC ni revisión antes de doce meses):
///
/// - Meses 1–4: acumulando, ingreso 3.000 > gasto 1.500 ⇒ ni venta ni fallo posible.
/// - Mes 5 = `R`: la pausa de ingresos pone el ingreso a 0 ⇒ necesidad ordinaria 1.500 > 1.000
///   permitidos ⇒ F3 se cumple; y la venta persigue 1.000 sobre una cartera de 500 ⇒ 500 € sin
///   fundar ⇒ F1 también. El latch guarda F1.
#[test]
fn f1_wins_the_month_it_coincides_with_f3() {
    let mut input = base_input(
        8,
        d(3_000),
        d(1_500),
        vec![mk_asset(1, d(500), true, Some(Decimal::ZERO))],
        vec![], // sin cascada: el superávit no vuelve a la cartera
    );
    input.phase_plan.expense_retirement_monthly = d(1_500);
    input.phase_plan.income_retirement_monthly = d(3_000);
    // `R` = 5, el mismo mes en que arranca la pausa: es donde los dos motivos coinciden.
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(5);
    input.phase_plan.withdrawal = WithdrawalRule::Guardrails {
        pct: d(2_400),
        band_pct: d(20),
        adjust_pct: d(10),
    };
    input.phase_plan.income_pause = Some(IncomePause {
        from_month: 5,
        months: 4,
        income_fraction: Decimal::ZERO,
    });

    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.liquid_worth[4], d(500), "nada se vende ni se aporta");
    assert_eq!(out.retirement_month_index, Some(5), "`R` es el mes de la pausa");
    assert_eq!(
        out.failure_month_index,
        Some(5),
        "el primer mes en que la pausa deja al hogar sin ingreso"
    );
    assert_eq!(
        out.failure_kind,
        Some(PathFailure::PortfolioDepleted),
        "F1 gana a F3 el mes en que los dos se cumplen"
    );
    // La prueba de que F3 TAMBIÉN se cumplía ese mes: la regla permitía 1.000 (recorte de 500
    // sobre una necesidad de 1.500) y la cartera solo pudo dar 500.
    assert_eq!(out.withdrawal_shortfall[5], d(500));
    assert_eq!(out.withdrawal[5], d(500));
    assert_eq!(out.unmet_need[5], d(500));
}

/// **Sin puerta declarada, ningún camino falla por tasa inicial** — la semántica de 4.15.0, que
/// es la que `pins-4.15.json` fotografía.
///
/// Se comprueba sobre la batería ENTERA del motor, no sobre un caso elegido: ninguno de los casos
/// de 4.15.0 declara puerta, así que ninguno puede fallar por F2 por mucho que se jubile con el
/// líquido bajo.
#[test]
fn without_a_gate_no_path_fails_by_initial_rate() {
    for case in projection_cases_all() {
        assert!(
            case.input.phase_plan.initial_rate.is_none(),
            "{}: la batería de 4.15.0 no declara puertas",
            case.name
        );
        let out = project_net_worth_series(&case.input).unwrap();
        assert_ne!(
            out.failure_kind,
            Some(PathFailure::InitialRateExceeded),
            "{}: sin puerta no se puede fallar por tasa inicial",
            case.name
        );
    }
}

// =============================================================================================
// E · Techo de aportación y caja disponible
// =============================================================================================

/// **Identidad contable del mes con techo**: `sobrante = invertido + disponible`, y el disponible
/// NO es patrimonio.
///
/// Predicho: ingreso 5.000, gasto 3.000 ⇒ sobrante 2.000; techo 1.200 ⇒ 1.200 invertidos y 800
/// disponibles cada uno de los 10 meses. `liquid(10) = 12.000`, `disposable_cash_total = 8.000`,
/// y el patrimonio NO incluye esos 8.000.
#[test]
fn a_contribution_cap_splits_the_surplus_and_the_rest_leaves_the_balance() {
    let mut input = lab(10, 5_000, 3_000, 0);
    input.phase_plan.contribution_cap_monthly = Some(d(1_200));
    let out = project_net_worth_series(&input).unwrap();

    assert_eq!(out.liquid_worth[10], d(12_000));
    assert_eq!(out.net_worth[10], d(12_000), "el disponible no es patrimonio");
    assert_eq!(out.disposable_cash_total, d(8_000));
    for k in 1..=10usize {
        assert_eq!(out.disposable_cash[k], d(800), "mes {k}");
        let invested = out.liquid_worth[k] - out.liquid_worth[k - 1];
        assert_eq!(
            invested + out.disposable_cash[k],
            d(2_000),
            "identidad del mes {k}: sobrante = invertido + disponible"
        );
    }
    assert_eq!(out.disposable_cash[0], Decimal::ZERO);

    // Sin techo, la serie es cero mes a mes y el total también.
    let plain = project_net_worth_series(&lab(10, 5_000, 3_000, 0)).unwrap();
    assert!(plain.disposable_cash.iter().all(|v| v.is_zero()));
    assert_eq!(plain.disposable_cash_total, Decimal::ZERO);
    assert_eq!(plain.liquid_worth[10], d(20_000));
}

/// El corte de coast es un techo de 0 desde su mes, y manda sobre el techo constante.
#[test]
fn stopping_contributions_beats_the_constant_cap() {
    let mut input = lab(10, 5_000, 3_000, 0);
    input.phase_plan.contribution_cap_monthly = Some(d(1_200));
    input.phase_plan.contributions_stop_month = Some(6);
    let out = project_net_worth_series(&input).unwrap();
    // Meses 1-5 aportan 1.200; del 6 en adelante, nada.
    assert_eq!(out.liquid_worth[5], d(6_000));
    assert_eq!(out.liquid_worth[10], d(6_000));
    assert_eq!(out.disposable_cash[6], d(2_000));
}

// =============================================================================================
// F · Solves (§B.7)
// =============================================================================================

/// **Cuánto más puedo gastar sin mover la fecha** (P8.b).
///
/// Predicho: cartera 5.000, sobrante 1.000 €/mes, objetivo 10.000 (400/0,04). Base: `liquid(k) =
/// 5.000 + 1.000k`, cruza 10.000 en `k = 5`, así que se jubila en el **mes 6**; el techo de
/// tolerancia es el mes 7, o sea `liquid(6) ≥ 10.000`:
///
/// ```text
/// 5.000 + 6·(1.000 − e) ≥ 10.000  ⇔  e ≤ 1.000/6 = 166,666… €/mes
/// ```
///
/// La bisección sobre `[0, 1.000]` con 24 halvings resuelve a menos de `1.000/2²⁴ ≈ 6·10⁻⁵`.
#[test]
fn the_extra_expense_that_keeps_the_date_is_one_sixth_of_the_headroom() {
    let mut input = lab(24, 3_000, 2_000, 5_000);
    input.fire_target = Some(flat_target(400, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);

    let baseline = project_net_worth_series(&input).unwrap();
    assert_eq!(baseline.retirement_month_index, Some(6), "la fecha base");

    let extra = max_extra_monthly_expense_keeping_date(&input)
        .unwrap()
        .unwrap();
    let expected = d(1_000) / d(6);
    assert!(
        (extra - expected).abs() < Decimal::new(1, 3),
        "166,666… €/mes predicho, obtenido {extra}"
    );

    // Sin fecha base no hay fecha que conservar: `None`, no un 0.
    let mut no_date = input.clone();
    no_date.fire_target = None;
    assert_eq!(max_extra_monthly_expense_keeping_date(&no_date).unwrap(), None);
}

/// **Cuánto retrasa una pausa de ingresos** (P8.c).
///
/// Predicho sobre el mismo hogar (cartera 5.000, +1.000 €/mes, objetivo 10.000, base = mes 6):
/// una pausa de 2 meses a fracción 0 desde el mes 2 convierte dos `+1.000` en dos `−2.000`, un
/// vuelco de 6.000 € = **6 meses** a 1.000 €/mes.
///
/// | mes | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 |
/// |---|---|---|---|---|---|---|---|---|---|---|---|
/// | líquido | 6.000 | 4.000 | 2.000 | 3.000 | 4.000 | 5.000 | 6.000 | 7.000 | 8.000 | 9.000 | 10.000 |
///
/// El cruce se decide contra el cierre anterior, así que se jubila en el **mes 12**: 12 − 6 = 6.
#[test]
fn an_income_pause_delays_retirement_by_six_months() {
    let mut input = lab(24, 3_000, 2_000, 5_000);
    input.fire_target = Some(flat_target(400, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);

    let delay = retirement_delay_months(
        &input,
        IncomePause {
            from_month: 2,
            months: 2,
            income_fraction: Decimal::ZERO,
        },
    )
    .unwrap();

    assert_eq!(delay.baseline_month_index, Some(6));
    assert_eq!(delay.paused_month_index, Some(12));
    assert_eq!(delay.delay_months, Some(6));
}

/// Una pausa que empuja la jubilación FUERA del horizonte no devuelve un retraso enorme: devuelve
/// `None`, porque «no se jubila» no es un número de meses.
#[test]
fn a_pause_that_pushes_retirement_past_the_horizon_has_no_delay_number() {
    let mut input = lab(8, 3_000, 2_000, 5_000);
    input.fire_target = Some(flat_target(400, 4));
    input.phase_plan.expense_retirement_monthly = d(2_000);

    let delay = retirement_delay_months(
        &input,
        IncomePause {
            from_month: 2,
            months: 3,
            income_fraction: Decimal::ZERO,
        },
    )
    .unwrap();
    assert_eq!(delay.baseline_month_index, Some(6));
    assert_eq!(delay.paused_month_index, None);
    assert_eq!(delay.delay_months, None);
}

// =============================================================================================
// G · Invariante de §C — el mes del ingreso ES el mes de la jubilación
// =============================================================================================

/// **Invariante de comportamiento (§C, hallazgo B4)**: el mes en que el ingreso conmuta al de
/// jubilación es exactamente `retirement_month_index`, y el mes en que conmuta al de media
/// jornada es exactamente `partial_retirement_month_index`. Se comprueba sobre la SERIE, no sobre
/// el enum: si algún día las fases y los importes se separan, esto lo caza.
#[test]
fn the_phase_readings_agree_with_the_cash_flow_they_describe() {
    let mut input = lab(24, 3_000, 2_000, 0);
    input.phase_plan.expense_retirement_monthly = d(2_500);
    input.phase_plan.income_retirement_monthly = d(500);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 5,
        income_monthly: d(2_400),
        expense_basis: ExpenseBasis::Regular,
    });
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(10);
    let out = project_net_worth_series(&input).unwrap();

    // Δ líquido del mes k (sin rentabilidad ni ventas parciales) = caja del mes.
    let delta = |k: usize| out.liquid_worth[k] - out.liquid_worth[k - 1];
    // Acumulando: 3.000 − 2.000 = +1.000.
    assert_eq!(delta(4), d(1_000));
    // Parcial (gasto REGULAR): 2.400 − 2.000 = +400, desde el mes 5.
    assert_eq!(delta(5), d(400));
    assert_eq!(delta(9), d(400));
    // Jubilado: 500 − 2.500 = −2.000, desde el mes 10.
    assert_eq!(delta(10), d(-2_000));

    assert_eq!(out.partial_retirement_month_index, Some(5));
    assert_eq!(out.retirement_month_index, Some(10));
    assert_eq!(
        out.phase_transitions,
        vec![
            (Phase::Accumulating, 0),
            (Phase::Partial, 5),
            (Phase::Retired, 10)
        ]
    );
}

/// Una media jornada declarada DESPUÉS de la jubilación **no ocurre** (las fases son monótonas), y
/// entonces no se publica su mes: el chart no puede pintar una fase que la simulación no vivió.
#[test]
fn a_partial_phase_after_retirement_never_happens() {
    let mut input = lab(12, 3_000, 2_000, 0);
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(3);
    input.phase_plan.partial = Some(PartialPhase {
        start_month: 8,
        income_monthly: d(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    let out = project_net_worth_series(&input).unwrap();
    assert_eq!(out.partial_retirement_month_index, None);
    assert!(!out.partial_phase_capital_growing);
    assert_eq!(
        out.phase_transitions,
        vec![(Phase::Accumulating, 0), (Phase::Retired, 3)]
    );
}

/// **La cota de búsqueda de los solves NO es el sobrante del mes 1**, y este test es la regresión
/// de esa decisión (tomada en WP3 con la medición de P9 delante, ver `search_ceiling` en
/// `crates/engine/src/solve.rs`).
///
/// P9 es el hogar realista de la batería: su neto recurrente del mes 1 son **500 €/mes**, pero su
/// caja mensual crece muy por encima cuando los pasivos se extinguen y los «Próximos» entran.
/// Medido a 600 meses: con un techo de 500 €/mes `líquido(599)` se queda en 91.444 € frente a los
/// 725.197 € de la cascada real, así que una cota de 500 € recortaría cualquier respuesta que
/// viva por encima.
///
/// E4 se llevó los dos solves que biseccionaban sobre un objetivo, pero **no la cota**: la sigue
/// usando [`max_extra_monthly_expense_keeping_date`], y por ahí se comprueba. Con la jubilación
/// FORZADA en el mes 600 la fecha no depende del gasto, así que la respuesta es la cota entera —
/// y tiene que superar de largo los 500 €/mes del mes 1.
#[test]
fn the_solve_ceiling_is_the_max_monthly_surplus_not_the_first_months_headroom() {
    let mut input = projection_cases_all()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 en la batería")
        .input;
    input.phase_plan.crossing_is_reading_only = true;
    input.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(600);

    let extra = max_extra_monthly_expense_keeping_date(&input)
        .unwrap()
        .expect("P9 se jubila por edad en el mes 600: hay fecha que conservar");
    assert!(
        extra > d(500),
        "la cota tiene que superar el neto recurrente del mes 1 (500 €): {extra}"
    );
}
