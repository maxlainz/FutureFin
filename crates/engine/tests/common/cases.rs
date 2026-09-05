//! Batería ÚNICA de casos del motor. **Una sola definición, dos consumidores:**
//!
//! - `audit_dump.rs` — vuelca en CSV las series de los casos L1–L6, P1–P6 y P13 para que un
//!   oráculo EXTERNO las compare mes a mes. Su salida es un contrato: no crece sin declararlo
//!   (P13 entró en WP1a de 5.0.0 con el arreglo de la issue #208).
//! - `golden_pins.rs` — canonicaliza y hashea TODOS los casos (L1–L6 y P1–P13) contra el pin de
//!   4.15.0, la red de seguridad de bit-identidad del refactor 5.0.0.
//!
//! El módulo vivía dentro de `audit_dump.rs` hasta WP0 de 5.0.0. Se extrajo porque dos baterías
//! escritas por separado divergen en silencio al primer caso que alguien «mejora» en un solo
//! lado, y entonces el pin deja de pinear lo que el dump vuelca.
//!
//! **Determinismo:** `ref_date()` es fija (2026-09-01), no hay RNG, no hay reloj y todos los
//! horizontes son ≤ 840 (`MAX_LIABILITY_SCHEDULE_MONTHS`). Todo el dinero es `Decimal` — el
//! freezer de `crates/engine/src/lib.rs` no escanea `tests/`, pero la norma es la misma.
//!
//! **Orden:** `projection_cases_all()` = `projection_cases_audit()` ++ `projection_cases_extended()`,
//! y `golden_pins.rs` tiene un test que lo comprueba: si alguien reordena la batería de auditoría,
//! el CSV cambia de forma sin que el hash lo delate.
//!
//! **Dos baterías desde WP2 de 5.0.0.** `projection_cases_all()` es EXACTAMENTE lo que
//! `pins-4.15.json` hashea y por eso **no crece**; los casos nuevos (reglas de retirada, techo
//! numérico de #209) viven en `projection_cases_5_0()`, que solo consumen el pin aditivo
//! `pins-5.0-outputs.json` y el CSV (`projection_cases_dumped()`). Un caso añadido al primer
//! conjunto obligaría a regenerar el pin que existe justamente para no moverse.

#![allow(dead_code)]

use chrono::NaiveDate;
use futurefin_engine::{
    debt_payments_remaining_series, AllocationCap, AllocationKind, AllocationRule,
    EarlyRepaymentEffect, ExpenseBasis, FireNeed, FireTarget, IncomePause, PartialPhase,
    PensionSchedule, PhasePlan, ProjectionInput, ProjectionLiabilityInput, RepaymentModel,
    RetirementTrigger, SimAsset, SpendMode, TargetBasis, TaxBracket, WithdrawalRule,
};
use rust_decimal::Decimal;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------------
// Constructores compartidos
// ---------------------------------------------------------------------------------------------

pub fn d(mantissa: i64, scale: u32) -> Decimal {
    Decimal::new(mantissa, scale)
}

pub fn ref_date() -> NaiveDate {
    // Fecha fija: el dump debe ser byte-estable entre ejecuciones.
    NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
}

pub fn mk_asset(id: u128, value: Decimal, liquid: bool, rate: Option<Decimal>) -> SimAsset {
    SimAsset {
        id: Uuid::from_u128(id),
        value,
        purchase_price: None,
        is_liquid: liquid,
        expected_annual_return_percent: rate,
    }
}

/// Igual que [`mk_asset`] pero con **base de coste declarada**: es lo que hace que el motor
/// derive `g_i = 1 − b_i/v_i` por activo (#178) en vez de aplicar el escalar
/// `taxable_gain_ratio`. Sin esto no hay forma de ejercitar la vía MIXTA del drenaje.
pub fn mk_asset_with_basis(
    id: u128,
    value: Decimal,
    liquid: bool,
    rate: Option<Decimal>,
    purchase_price: Decimal,
) -> SimAsset {
    SimAsset {
        id: Uuid::from_u128(id),
        value,
        purchase_price: Some(purchase_price),
        is_liquid: liquid,
        expected_annual_return_percent: rate,
    }
}

pub fn mk_liab(
    principal: Decimal,
    payment: Decimal,
    apr: Option<Decimal>,
    model: RepaymentModel,
    end_after_months: Option<u32>,
) -> ProjectionLiabilityInput {
    let payment_end = end_after_months.map(|n| {
        // Último día del mes n (1-based) contando desde ref_date.
        let first = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        first
            .checked_add_months(chrono::Months::new(n))
            .unwrap()
            .pred_opt()
            .unwrap()
    });
    ProjectionLiabilityInput {
        principal,
        monthly_payment: payment,
        payment_end,
        repayment_model: model,
        apr_percent: apr,
        min_payment_pct: None,
        min_payment_eur: None,
        extra_principal_monthly: Decimal::ZERO,
        extra_principal_lump_sums: vec![],
        early_repayment_fee_pct: None,
        early_repayment_effect: Default::default(),
    }
}

pub fn base_input(
    horizon: u32,
    income: Decimal,
    expense: Decimal,
    assets: Vec<SimAsset>,
    rules: Vec<AllocationRule>,
) -> ProjectionInput {
    ProjectionInput {
        ref_date: ref_date(),
        horizon_months: horizon,
        annual_inflation_percent: Decimal::ZERO,
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        income_regular_monthly: income,
        expense_regular_monthly: expense,
        assets,
        allocation_rules: rules,
        liabilities: vec![],
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        // El plan de 4.15.0 tal cual: jubilación por cruce, `fixed_real`/`ceiling`, sin fase
        // parcial ni pensión con fecha y sin retirada extra. Los casos que necesitan otra cosa
        // (P10) lo mutan campo a campo, igual que antes mutaban los cuatro campos sueltos.
        phase_plan: PhasePlan::classic(Decimal::ZERO, expense),
        fire_target: None,
    }
}

pub fn rule_fixed(target: usize, amount: Decimal, cap: Option<AllocationCap>) -> AllocationRule {
    AllocationRule {
        target_index: target,
        kind: AllocationKind::Fixed,
        amount: Some(amount),
        cap,
    }
}

pub fn rule_percent(target: usize, pct: Decimal, cap: Option<AllocationCap>) -> AllocationRule {
    AllocationRule {
        target_index: target,
        kind: AllocationKind::Percent,
        amount: Some(pct),
        cap,
    }
}

pub fn rule_remainder(target: usize) -> AllocationRule {
    AllocationRule {
        target_index: target,
        kind: AllocationKind::Remainder,
        amount: None,
        cap: None,
    }
}

/// Escala del ahorro española 2025-26 — la MISMA que `default_es_tax_brackets()` de
/// `apps/api/src/handlers/installation.rs` (19/21/23/27/30 % en 6k/50k/200k/300k). Copiada a
/// propósito: el motor no depende del crate de la API, y un caso de pin que use otra escala
/// dejaría de pinear el camino que producción ejecuta.
pub fn es_tax_brackets_2025_26() -> Vec<TaxBracket> {
    vec![
        TaxBracket {
            up_to: Some(Decimal::from(6_000u32)),
            pct: Decimal::from(19u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(50_000u32)),
            pct: Decimal::from(21u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(200_000u32)),
            pct: Decimal::from(23u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(300_000u32)),
            pct: Decimal::from(27u32),
        },
        TaxBracket {
            up_to: None,
            pct: Decimal::from(30u32),
        },
    ]
}

// ---------------------------------------------------------------------------------------------
// Casos
// ---------------------------------------------------------------------------------------------

/// **El hogar de P9**, con el saldo de la cuenta corriente (activo 0) como ÚNICO eje.
///
/// 840 meses, 5 activos, cascada de 3 reglas con tope, 2 pasivos (uno francés con TIN y otro sin
/// interés que vence), planning flows con signo en dos tramos, objetivo FIRE con pensión,
/// impuestos ES, inflación 2,5 % y término finito de deuda.
///
/// Dos instancias, y el eje NO es decorativo:
/// - `20_000` ⇒ `P9_hogar_realista`, el caso grande y el que mide `timing.rs`.
/// - `8_000` ⇒ `P13_cash8k_denormal_g`, la **regresión de la issue #208**: con 8.000 € el −15.000
///   del mes 24 deja la cuenta al borde, y como la cuenta va al 0 % y la cascada la alimenta, su
///   base de coste queda pegada al valor (cada euro aportado sube las dos), el drenaje conserva
///   `b/v` y un 0 % no vuelve a abrir hueco: la plusvalía relativa `g = 1 − b/v` se queda
///   DENORMAL (≈1e-27). Hasta 4.15.0 el solver mixto calculaba `(techo_del_tramo − base) / g` con
///   `/` y esta proyección PANICABA en el mes 138 con «Division overflowed» — un 400 `task_panic`
///   opaco en producción. Desde WP1a de 5.0.0 el tope va por `checked_div` y los 840 meses corren.
///   Con 12.000 y 15.000 panicaba igual; con 20.000 (P9) no, y por eso el bug sobrevivió a WP0.
///
/// Los dos casos comparten TODO lo demás — un segundo hogar escrito a mano habría divergido al
/// primer retoque, y entonces P13 dejaría de ser «P9 con menos caja».
pub fn p9_household(checking_account_value: Decimal) -> ProjectionInput {
    let mut p9 = base_input(
        840,
        Decimal::from(4_200),
        Decimal::from(2_600),
        vec![
            // 0 — cuenta corriente, 0 %: destino del `remainder` (sumidero). ES EL EJE del par
            //     P9/P13 (ver el doc de arriba): al 0 % su base de coste no se despega nunca del
            //     valor, así que es el activo que fabrica la `g` denormal de #208.
            mk_asset(1, checking_account_value, true, None),
            // 1 — fondo de bonos 3 %: destino de la regla fija con tope Amount(20.000).
            mk_asset(2, Decimal::from(12_000), true, Some(Decimal::from(3))),
            // 2 — fondo de RV 6,5 % CON base de coste declarada (30.000 sobre 40.000 ⇒ g = 0,25).
            mk_asset_with_basis(
                3,
                Decimal::from(40_000),
                true,
                Some(d(65, 1)),
                Decimal::from(30_000),
            ),
            // 3 — vivienda 1 %, ILÍQUIDA: pesa en `net_worth` pero no en `liquid_worth`, así que
            //     el cruce FIRE (#143) no la cuenta y el drenaje solo la toca al final.
            mk_asset(4, Decimal::from(250_000), false, Some(Decimal::ONE)),
            // 4 — cripto 12 %: la cola de rentabilidad alta, última en el orden de drenaje.
            mk_asset(5, Decimal::from(5_000), true, Some(Decimal::from(12))),
        ],
        vec![
            rule_fixed(
                1,
                Decimal::from(300),
                Some(AllocationCap::Amount(Decimal::from(20_000))),
            ),
            rule_percent(2, Decimal::from(60), None),
            rule_remainder(0),
        ],
    );
    p9.annual_inflation_percent = d(25, 1);
    p9.tax_brackets = es_tax_brackets_2025_26();
    p9.taxes_enabled = true;
    p9.taxable_gain_ratio = Decimal::ONE;
    p9.phase_plan.income_retirement_monthly = Decimal::from(900);
    p9.phase_plan.expense_retirement_monthly = Decimal::from(2_300);
    p9.liabilities = vec![
        // Hipoteca francesa: 180.000 € al TIN 2,9 %, cuota 900 €, sin fecha de fin declarada.
        mk_liab(
            Decimal::from(180_000),
            Decimal::from(900),
            Some(d(29, 1)),
            RepaymentModel::French,
            None,
        ),
        // Préstamo al consumo sin interés que VENCE a los 30 meses.
        mk_liab(
            Decimal::from(6_000),
            Decimal::from(200),
            None,
            RepaymentModel::FixedPayments,
            Some(30),
        ),
    ];
    // Planning flows: el índice `i` del vector es el mes `i+1` del bucle.
    //   · −15.000 € en el mes 24 (una entrada de coche) ⇒ índice 23.
    //   · +800 €/mes del mes 36 al 72 inclusive (un alquiler temporal) ⇒ índices 35..=71.
    p9.planning_monthly_cash_adjustment[23] = Decimal::from(-15_000);
    for i in 35..=71 {
        p9.planning_monthly_cash_adjustment[i] = Decimal::from(800);
    }
    p9.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(2_300),
            pension_monthly: Decimal::from(900),
        },
        swr_pct: d(35, 1),
        tax_brackets: es_tax_brackets_2025_26(),
        taxes_enabled: true,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: d(25, 1),
        // Término finito de deuda (#142) calculado con la MISMA función que el handler.
        debt_payments_remaining: debt_payments_remaining_series(&p9.liabilities, ref_date()),
    });
    p9
}

/// Un calendario de amortización a volcar/hashear, con el horizonte con el que se pide.
pub struct LiabCase {
    pub name: &'static str,
    pub liab: ProjectionLiabilityInput,
    pub horizon: u32,
}

/// Una proyección a volcar/hashear.
pub struct ProjCase {
    pub name: &'static str,
    pub input: ProjectionInput,
}

/// Batería de calendarios de amortización (casos L*). Orden = orden del CSV de `audit_dump`.
pub fn liability_cases() -> Vec<LiabCase> {
    vec![
        // L1: préstamo que VENCE con saldo vivo — 50.000 € al TIN 6 %, cuota 500 €, plan de 60 meses.
        LiabCase {
            name: "L1_venc_saldo_vivo",
            liab: mk_liab(
                Decimal::from(50_000),
                Decimal::from(500),
                Some(Decimal::from(6)),
                RepaymentModel::French,
                Some(60),
            ),
            horizon: 840,
        },
        // L2: cuota por debajo del interés — 20.000 € al TIN 24 %, cuota 300 € (interés mes 1 = 400 €).
        LiabCase {
            name: "L2_cuota_bajo_interes",
            liab: mk_liab(
                Decimal::from(20_000),
                Decimal::from(300),
                Some(Decimal::from(24)),
                RepaymentModel::French,
                None,
            ),
            horizon: 120,
        },
        // L3: revolving — 5.000 € al 21 % (tal cual lo teclearía el usuario desde una TAE cotizada),
        // cuota fija 150 €.
        LiabCase {
            name: "L3_revolving",
            liab: mk_liab(
                Decimal::from(5_000),
                Decimal::from(150),
                Some(Decimal::from(21)),
                RepaymentModel::Revolving,
                None,
            ),
            horizon: 840,
        },
        // L4: pasivo SIN TIN con modelo francés — degeneración deliberada a fixed_payments.
        LiabCase {
            name: "L4_sin_tin",
            liab: mk_liab(
                Decimal::from(100_000),
                Decimal::from(500),
                None,
                RepaymentModel::French,
                None,
            ),
            horizon: 840,
        },
        // L5: caso patrón contra cuadro externo — 100.000 € al TIN 3 %, cuota de anualidad para
        // n = 278 meses: M = P·i/(1−(1+i)^−n) con i = 0,0025 → 499,51 € (redondeada a céntimo).
        LiabCase {
            name: "L5_bde_100k_3pct",
            liab: mk_liab(
                Decimal::from(100_000),
                d(49_951, 2),
                Some(Decimal::from(3)),
                RepaymentModel::French,
                None,
            ),
            horizon: 840,
        },
        // L6: interest_only con cuota declarada ≠ interés real — 100.000 € al TIN 4 % (interés real
        // 333,33 €/mes) pero cuota declarada 200 €.
        LiabCase {
            name: "L6_interest_only_200",
            liab: mk_liab(
                Decimal::from(100_000),
                Decimal::from(200),
                Some(Decimal::from(4)),
                RepaymentModel::InterestOnly,
                None,
            ),
            horizon: 120,
        },
    ]
}

/// Batería de proyecciones que `audit_dump` vuelca (casos P1–P6 y P13). **Este orden ES el CSV.**
///
/// P13 se sumó en WP1a de 5.0.0 con el arreglo de la issue #208: el CSV creció en un caso y el
/// oráculo externo tiene que enterarse (la cota la vigila
/// `golden_pins.rs::the_audit_battery_is_the_ordered_prefix_of_the_pinned_battery`).
pub fn projection_cases_audit() -> Vec<ProjCase> {
    let mut out = Vec::new();

    // P1: déficit crónico — ingreso 1.000, gasto 2.500, 30.000 € líquidos al 0 %.
    // La realidad: agotamiento en el mes 20 y desde ahí el NW cae 1.500 €/mes (deuda implícita).
    out.push(ProjCase {
        name: "P1_deficit_cronico",
        input: base_input(
            60,
            Decimal::from(1_000),
            Decimal::from(2_500),
            vec![mk_asset(1, Decimal::from(30_000), true, None)],
            vec![],
        ),
    });

    // P2: FIRE alcanzado en el mes 0 — NW inicial 900.000 ≥ target 800.000.
    let mut p2 = base_input(
        24,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(900_000), true, None)],
        vec![],
    );
    p2.phase_plan.income_retirement_monthly = Decimal::ZERO;
    p2.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p2.fire_target = Some(FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: Decimal::from(800_000),
        },
        swr_pct: Decimal::from(100u32),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: d(25, 1),
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P2_fire_mes0",
        input: p2,
    });

    // P3: superávit post-jubilación — pensión 2.500 vs gasto 2.000 tras cruzar un target bajo;
    // el sobrante de 500 €/mes se acumula al 0 % durante décadas.
    let mut p3 = base_input(
        480,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(190_000),
            true,
            Some(Decimal::from(3)),
        )],
        vec![],
    );
    p3.phase_plan.income_retirement_monthly = Decimal::from(2_500);
    p3.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p3.fire_target = Some(FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: Decimal::from(200_000),
        },
        swr_pct: Decimal::from(100u32),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P3_superavit_jubilacion",
        input: p3,
    });

    // P4: retornos en el borde −100 % / −101 % — factor 0, jamás negativo.
    out.push(ProjCase {
        name: "P4_ret_menos100",
        input: base_input(
            6,
            Decimal::ZERO,
            Decimal::ZERO,
            vec![
                mk_asset(1, Decimal::from(10_000), true, Some(Decimal::from(-100))),
                mk_asset(2, Decimal::from(10_000), true, Some(Decimal::from(-101))),
            ],
            vec![],
        ),
    });

    // P5: caso patrón de unidades — ingreso 3.000 / gasto 2.000 nominales planos 30 años,
    // activo 10.000 al 7 %. Para confrontar contra un oráculo con flujos indexados al IPC.
    out.push(ProjCase {
        name: "P5_flat_nominal_30y",
        input: base_input(
            360,
            Decimal::from(3_000),
            Decimal::from(2_000),
            vec![mk_asset(
                1,
                Decimal::from(10_000),
                true,
                Some(Decimal::from(7)),
            )],
            // 4.12.1: P5 gana el sumidero — sin él, P3/P5/P6 dejaban de ejercitar la cascada a
            // la vez y el arnés perdía su única cobertura de superávit invertido.
            vec![rule_remainder(0)],
        ),
    });

    // P6: el préstamo L1 dentro de una proyección a 30 años — tras vencer el plan en el mes 60,
    // el saldo vivo queda CONGELADO restando al patrimonio hasta el final.
    let mut p6 = base_input(
        360,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(10_000), true, None)],
        vec![],
    );
    p6.liabilities = vec![mk_liab(
        Decimal::from(50_000),
        Decimal::from(500),
        Some(Decimal::from(6)),
        RepaymentModel::French,
        Some(60),
    )];
    out.push(ProjCase {
        name: "P6_venc_saldo_vivo_proj",
        input: p6,
    });

    // -----------------------------------------------------------------------------------------
    // P13: el hogar de P9 con 8.000 € en la cuenta corriente en vez de 20.000 — la **regresión
    //      de la issue #208** (pánico «Division overflowed» en `gross_up_mixed_monthly`).
    //
    // Está en la batería de AUDITORÍA, no en la extendida, a propósito: la `g` denormal nace del
    // par (cuenta al 0 % alimentada por la cascada) × (venta fuerte) y atraviesa el gross-up
    // mixto por tramos progresivos — justo la aritmética que un oráculo externo debe poder
    // reproducir mes a mes. Un pin interno solo diría «no panica»; el CSV dice «y estos son los
    // números». El mecanismo completo, en el doc de [`p9_household`].
    // -----------------------------------------------------------------------------------------
    out.push(ProjCase {
        name: "P13_cash8k_denormal_g",
        input: p9_household(Decimal::from(8_000)),
    });

    out
}

/// Casos P7–P12: los caminos que el refactor 5.0.0 va a tocar y que P1–P6 NO ejercitaban.
///
/// **No entran en el CSV de `audit_dump`** (su formato es un contrato con el oráculo externo);
/// existen para que el pin cubra el motor entero, no solo la mitad que el oráculo mira.
pub fn projection_cases_extended() -> Vec<ProjCase> {
    let mut out = Vec::new();

    // -----------------------------------------------------------------------------------------
    // P7: jubilado desde el mes 0 con pensión plana, impuestos ES y gasto indexado al 2,5 %.
    //
    // Camino cubierto: `FireNeed::ExpenseMinusPension` + gross-up por tramos + latch de
    // jubilación en el mes 0 (cruce contra el patrimonio LÍQUIDO) + drenaje con impuestos por la
    // vía ESCALAR (ningún activo declara `purchase_price` ⇒ g uniforme = `taxable_gain_ratio`).
    //
    // PREDICCIÓN (a mano, antes de correr nada):
    //   · need anual neta con f(0)=1: (2.000 − 800)·12 = 14.400 €.
    //   · gross-up ES: el primer tramo (19 % hasta 6.000) deja neto 6.000−1.140 = 4.860 €; los
    //     9.540 € netos que faltan salen del tramo del 21 %: 9.540/0,79 = 12.075,9493... ⇒
    //     G = 6.000 + 12.075,9493... = 18.075,9493670886... €.
    //   · objetivo(0) = G / 0,035 = 516.455,6962025316... € → 516.455,70 € a dos decimales.
    //     Líquido inicial 600.000 € ≥ objetivo ⇒ JUBILADO en el mes 0.
    //   · caja del mes 1 (f(0)=1, sin deuda ni planning): 800 − 2.000 = **−1.200 €**.
    // Ambas se afirman en `golden_pins.rs::p7_and_p9_are_anchored_by_hand_derived_numbers`.
    // -----------------------------------------------------------------------------------------
    let mut p7 = base_input(
        360,
        // Irrelevante por construcción (jubilado desde el mes 0), pero deliberadamente ≠ pensión:
        // si el latch dejara de dispararse, el hash se movería en vez de pasar de largo.
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(600_000), true, Some(d(35, 1)))],
        vec![],
    );
    p7.annual_inflation_percent = d(25, 1);
    p7.tax_brackets = es_tax_brackets_2025_26();
    p7.taxes_enabled = true;
    p7.taxable_gain_ratio = Decimal::ONE;
    p7.phase_plan.income_retirement_monthly = Decimal::from(800);
    p7.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p7.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(2_000),
            pension_monthly: Decimal::from(800),
        },
        swr_pct: d(35, 1),
        tax_brackets: es_tax_brackets_2025_26(),
        taxes_enabled: true,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: d(25, 1),
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P7_jubilado_pension_impuestos",
        input: p7,
    });

    // -----------------------------------------------------------------------------------------
    // P8: déficit crónico con `g` MIXTA — la única vía que ejercita `gross_up_mixed_monthly`.
    //
    // Tres activos líquidos con tres `g` distintas y > 0 valor:
    //   · 0 — fondo BAJO EL AGUA: v 10.000, coste 12.000 ⇒ g = clamp(1 − 1,2) = 0.
    //   · 1 — fondo con 50 % de plusvalía: v 20.000, coste 10.000 ⇒ g = 0,5.
    //   · 2 — sin `purchase_price` ⇒ g = el ESCALAR `taxable_gain_ratio` = 1.
    // Orden de drenaje (líquidos, menor rentabilidad primero): 0 (2 %), 1 (5 %), 2 (8 %) — el
    // mismo que el solver mixto usa para montar sus tramos.
    //
    // El déficit (500 − 2.000 = −1.500 €/mes) vacía los 45.000 € en un par de años: a partir de
    // ahí no queda activo con valor > 0, la `g` vuelve a ser uniforme y el caso ejercita también
    // la vía escalar y el descubierto NETO acumulado. Dos caminos en un solo hash.
    // -----------------------------------------------------------------------------------------
    let mut p8 = base_input(
        120,
        Decimal::from(500),
        Decimal::from(2_000),
        vec![
            mk_asset_with_basis(
                1,
                Decimal::from(10_000),
                true,
                Some(Decimal::from(2)),
                Decimal::from(12_000),
            ),
            mk_asset_with_basis(
                2,
                Decimal::from(20_000),
                true,
                Some(Decimal::from(5)),
                Decimal::from(10_000),
            ),
            mk_asset(3, Decimal::from(15_000), true, Some(Decimal::from(8))),
        ],
        vec![],
    );
    p8.tax_brackets = es_tax_brackets_2025_26();
    p8.taxes_enabled = true;
    p8.taxable_gain_ratio = Decimal::ONE;
    out.push(ProjCase {
        name: "P8_drenaje_g_mixta",
        input: p8,
    });

    // -----------------------------------------------------------------------------------------
    // P9: «hogar realista» — el caso GRANDE, y también el que mide el arnés de tiempos
    //     (`timing.rs`). 840 meses, 5 activos, cascada de 3 reglas con tope, 2 pasivos (uno
    //     francés con TIN y otro sin interés que vence), planning flows con signo en dos tramos,
    //     objetivo FIRE con pensión, impuestos ES, inflación 2,5 % y término finito de deuda.
    //
    // PREDICCIÓN del mes 1 (a mano, antes de correr nada):
    //   · Servicio de deuda: francés 180.000 € al 2,9 % ⇒ interés = 180.000 · 0,029/12 =
    //     **435,00 €** exactos; saldo con interés 180.435 € > cuota ⇒ caja = 900 €.
    //     `fixed_payments` 6.000 € con plan vivo ⇒ caja = min(200, 6.000) = 200 €.
    //     TOTAL = **1.100 €**.
    //   · Gasto del mes 1 con f(0) = 1: **2.600 €** (lo recién tecleado no se mueve).
    //   · Neto recurrente = 4.200 − 2.600 − 1.100 = **500 €**; planning[0] = 0 ⇒ caja = 500 €.
    //   · Cascada: fija 300 → bonos (hueco 20.000 − 12.000 = 8.000) ⇒ 300; queda 200.
    //     60 % de lo que queda ⇒ 120 → renta variable; queda 80. Resto ⇒ 80 a la cuenta.
    //     `per_asset = [80, 300, 120, 0, 0]`, `leftover = 0`.
    // Todo ello se afirma en `golden_pins.rs::p7_and_p9_are_anchored_by_hand_derived_numbers`.
    // -----------------------------------------------------------------------------------------
    let p9 = p9_household(Decimal::from(20_000));
    out.push(ProjCase {
        name: "P9_hogar_realista",
        input: p9,
    });

    // -----------------------------------------------------------------------------------------
    // P10: jubilación FORZADA por el trigger `AtMonth` del `PhasePlan` (no por cruce de objetivo)
    //      con `extra_monthly_withdrawal > 0` — los antiguos `retirement_start_month` /
    //      `retirement_monthly_withdrawal`. El motor lo soporta desde siempre y la API
    //      **nunca** lo rellena — es decir, ningún test de integración lo cubre: si el refactor
    //      lo rompiese, no habría red fuera de este pin.
    //
    // Meses 1–36: 3.000 − 2.000 = 1.000 €/mes al `remainder`. Desde el mes 37 (`37 >= 37`):
    // ingreso de jubilación 0, gasto 2.000 y retirada extra de 400 ⇒ −2.400 €/mes, drenaje,
    // agotamiento dentro del horizonte y descubierto acumulado.
    // -----------------------------------------------------------------------------------------
    let mut p10 = base_input(
        120,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(50_000),
            true,
            Some(Decimal::from(5)),
        )],
        vec![rule_remainder(0)],
    );
    p10.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(37);
    p10.phase_plan.extra_monthly_withdrawal = Decimal::from(400);
    out.push(ProjCase {
        name: "P10_jubilacion_forzada",
        input: p10,
    });

    // -----------------------------------------------------------------------------------------
    // P11: DEFLACIÓN (−2 %) con objetivo `ExpenseMinusPension` y sumidero `remainder`.
    //
    // Dos cosas que solo pasan con inflación negativa (#146):
    //   · el gasto DECRECE mes a mes (`f(k) = 0,98^(k/12)`), así que un hogar en déficit acaba
    //     en superávit sin que cambie ningún input: el caso recorre la rama de drenaje Y la de
    //     cascada, y la cascada corre JUBILADO (#175, 4.12.1);
    //   · la necesidad del objetivo se AGOTA dentro del horizonte: 1.800·f − 1.500 ≤ 0 en cuanto
    //     f ≤ 5/6, o sea hacia el mes 109 — la base del objetivo pasa a 0 y solo queda deuda.
    // Objetivo(0) = (1.800 − 1.500)·12 / 0,035 = 102.857,14 € < 200.000 € líquidos ⇒ jubilado
    // desde el mes 0.
    // -----------------------------------------------------------------------------------------
    let mut p11 = base_input(
        240,
        Decimal::from(2_500),
        Decimal::from(1_800),
        vec![mk_asset(
            1,
            Decimal::from(200_000),
            true,
            Some(Decimal::from(4)),
        )],
        vec![rule_remainder(0)],
    );
    p11.annual_inflation_percent = Decimal::from(-2);
    p11.phase_plan.income_retirement_monthly = Decimal::from(1_500);
    p11.phase_plan.expense_retirement_monthly = Decimal::from(1_800);
    p11.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(1_800),
            pension_monthly: Decimal::from(1_500),
        },
        swr_pct: d(35, 1),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::from(-2),
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P11_deflacion_negativa",
        input: p11,
    });

    // -----------------------------------------------------------------------------------------
    // P12: los TRES tipos de tope de la cascada, dos reglas sobre el MISMO activo, y sumidero.
    //
    //   regla 0 — fija 200 → activo 0, tope `MonthsExpense(1)`   ⇒ techo = 1 · (gasto + deuda)
    //   regla 1 — 50 %     → activo 0, tope `IncomeMultiple(2)`  ⇒ techo = 2 · ingreso = 6.000
    //   regla 2 — fija 400 → activo 1, tope `Amount(5.000)`
    //   regla 3 — remainder → activo 2 (sumidero, arranca en 0)
    //
    // El techo `MonthsExpense` se mueve cada mes porque el gasto se indexa al 2 %: es el único
    // tope de los tres que NO es constante, y por eso el caso lleva inflación. Con 1.000 €/mes de
    // caja, el activo 0 rebasa su primer techo hacia el mes 3 (`CapFull`), el segundo hacia el 8,
    // y a partir de ahí la cascada empuja al fondo y luego al sumidero: el hash cubre `CapFull`,
    // `NotReached` y el recorte de una regla (intent > resolved) sin necesidad de un caso por
    // razón.
    // -----------------------------------------------------------------------------------------
    let mut p12 = base_input(
        60,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![
            mk_asset(1, Decimal::from(1_000), true, None),
            mk_asset(2, Decimal::from(2_000), true, Some(Decimal::from(5))),
            mk_asset(3, Decimal::ZERO, true, None),
        ],
        vec![
            rule_fixed(
                0,
                Decimal::from(200),
                Some(AllocationCap::MonthsExpense(Decimal::ONE)),
            ),
            rule_percent(
                0,
                Decimal::from(50),
                Some(AllocationCap::IncomeMultiple(Decimal::from(2))),
            ),
            rule_fixed(
                1,
                Decimal::from(400),
                Some(AllocationCap::Amount(Decimal::from(5_000))),
            ),
            rule_remainder(2),
        ],
    );
    p12.annual_inflation_percent = Decimal::from(2);
    out.push(ProjCase {
        name: "P12_topes_de_cascada",
        input: p12,
    });

    out
}

/// Todos los casos de proyección **de 4.15.0**: los del CSV primero, en su orden, y luego los
/// añadidos. Es EXACTAMENTE el conjunto que `pins-4.15.json` hashea, y por eso **no crece**: un
/// caso nuevo aquí obligaría a regenerar el pin que existe justamente para no moverse. Los casos
/// de 5.0.0 viven en [`projection_cases_5_0`].
pub fn projection_cases_all() -> Vec<ProjCase> {
    let mut out = projection_cases_audit();
    out.extend(projection_cases_extended());
    out
}

/// **Casos de 5.0.0 (WP2): las reglas de retirada y el techo numérico de la issue #209.**
///
/// Viven APARTE de [`projection_cases_all`] a propósito: aquel conjunto es el que `pins-4.15.json`
/// hashea para demostrar que el refactor no movió las salidas de 4.15.0, y añadirle un caso
/// obligaría a regenerar ese fichero — que es justo lo que no puede pasar. Estos se pinean solo en
/// `pins-5.0-outputs.json` (aditivo) y se vuelcan también al CSV de auditoría
/// ([`projection_cases_dumped`]): semántica nueva merece oráculo externo, no solo un hash interno.
///
/// Cada uno cubre un camino que ninguno de los P1–P13 tocaba:
///
/// | caso | regla | modo | fiscalidad | qué ejercita |
/// |---|---|---|---|---|
/// | P14 | `fixed_real` | ceiling | sin impuestos | la base de coste en el techo de `NUMERIC(18,4)` (#209) |
/// | P15 | `percent_of_balance` | ceiling | ES, `g` MIXTA | el techo BRUTO por el paseo directo mixto |
/// | P16 | `hybrid` | rule_is_spend | sin impuestos | cascada + venta el MISMO mes, y el latch |
/// | P17 | `guardrails` | ceiling | ES, `g` escalar | las revisiones anuales y el recorte indexado |
///
/// **WP3 añadió seis** (§B.1/§B.3/§B.7), uno por camino nuevo del bucle:
///
/// | caso | qué ejercita |
/// |---|---|
/// | P18 | pensión con fecha INDEXADA + objetivo **puente** al 5 %, con impuestos ES y `g` mixta |
/// | P19 | pensión que **cubre el gasto entero** ⇒ objetivo = deuda desde `P`, cruce inmediato |
/// | P20 | **media jornada** del ejemplo del issue (1.100 €/mes desde el mes 60, hueco 900) |
/// | P21 | `retire_at_age`: **cruce como lectura** + jubilación por edad infra-financiada |
/// | P22 | **techo de aportación** y la serie `disposable_cash` (el escenario del solve) |
/// | P23 | **pausa de ingresos** (P8.c) y el retraso que provoca |
pub fn projection_cases_5_0() -> Vec<ProjCase> {
    let mut out = Vec::new();

    // -----------------------------------------------------------------------------------------
    // P14: un activo en el TECHO de su columna — la regresión de la issue **#209**.
    //
    // `value = purchase_price = 99.999.999.999.999` (el techo de `NUMERIC(18,4)`,
    // `20260210120000_assets.sql`) al 20 %/año, gasto 1.000 €/mes, 840 meses. La base de coste
    // solo encoge (~1e14) mientras el valor compone al 20 %, así que el producto `b·v` de
    // `b' = b·v_post/v_pre` (#120) se sale del rango de `Decimal` (~7,9e28) en cuanto
    // `v > 7,9e14` — o sea `1,2^t > 7,9`, unos 11,3 años: **hacia el mes 136**. Hasta WP2 eso era
    // un pánico «Multiplication overflowed» y, en producción, un 400 `task_panic` opaco.
    //
    // Es `fixed_real`: el caso no va de reglas de retirada, va de que el motor no panique con
    // importes que la API acepta. Por eso está aquí y no entre los P1–P13 — no existía cuando se
    // escribió aquel pin, y meterlo allí habría obligado a regenerarlo.
    // -----------------------------------------------------------------------------------------
    let techo = Decimal::from(99_999_999_999_999i64);
    out.push(ProjCase {
        name: "P14_techo_numeric",
        input: base_input(
            840,
            Decimal::ZERO,
            Decimal::from(1_000),
            vec![mk_asset_with_basis(
                1,
                techo,
                true,
                Some(Decimal::from(20)),
                techo,
            )],
            vec![],
        ),
    });

    // -----------------------------------------------------------------------------------------
    // P15: `percent_of_balance` al 4 % con TECHO (`ceiling`), impuestos ES y `g` MIXTA.
    //
    // Jubilado desde el mes 1 con 400.000 € líquidos repartidos en dos fondos con base de coste
    // declarada (g = 0,2 y g = 0,6): el techo del mes 1 es `4 %·400.000/12 = 1.333,33 €` BRUTOS
    // (R9), la necesidad es `2.300 − 900 = 1.400 €` NETOS y su bruto con la escala ES pasa de
    // 1.700 — así que el techo ATA y la venta la resuelve el **paseo directo mixto**
    // (`mixed_drawdown_for_gross_cap`), el camino que ningún caso anterior tocaba.
    //
    // Con inflación del 2 % el gasto crece y el techo baja con el saldo: el recorte
    // (`withdrawal_shortfall`) crece mes a mes sin restar un euro de patrimonio — la separación
    // de las tres magnitudes, pineada.
    // -----------------------------------------------------------------------------------------
    let mut p15 = base_input(
        360,
        Decimal::from(1_000),
        Decimal::from(2_500),
        vec![
            // 0 — fondo conservador 3 %, coste 120.000 sobre 150.000 ⇒ g = 0,2 (drena primero).
            mk_asset_with_basis(
                1,
                Decimal::from(150_000),
                true,
                Some(Decimal::from(3)),
                Decimal::from(120_000),
            ),
            // 1 — fondo de RV 6 %, coste 100.000 sobre 250.000 ⇒ g = 0,6.
            mk_asset_with_basis(
                2,
                Decimal::from(250_000),
                true,
                Some(Decimal::from(6)),
                Decimal::from(100_000),
            ),
        ],
        vec![rule_remainder(0)],
    );
    p15.annual_inflation_percent = Decimal::from(2);
    p15.tax_brackets = es_tax_brackets_2025_26();
    p15.taxes_enabled = true;
    p15.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
    p15.phase_plan.income_retirement_monthly = Decimal::from(900);
    p15.phase_plan.expense_retirement_monthly = Decimal::from(2_300);
    p15.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance {
        pct: Decimal::from(4),
    };
    p15.phase_plan.spend_mode = SpendMode::Ceiling;
    out.push(ProjCase {
        name: "P15_percent_of_balance_ceiling",
        input: p15,
    });

    // -----------------------------------------------------------------------------------------
    // P16: `hybrid` 5 % → 3,5 % con la regla COMO GASTO (`rule_is_spend`, R7).
    //
    // Jubilado desde el mes 1 con 500.000 € al 10 %, rentas de 1.800 €/mes y gasto de 1.500
    // indexado al 2 %. Los primeros años hay SUPERÁVIT: la cascada reinvierte la caja del mes
    // PRIMERO y la regla vende DESPUÉS —el único caso de la batería donde las dos cosas pasan el
    // mismo mes—, y el sobrante (`withdrawal_excess`) se gasta y no vuelve. Cuando la inflación
    // se come el margen, el mismo caso pasa a déficit sin cambiar de regla.
    //
    // El latch: `3,5·L(k−1) ≥ 5·500.000·f(k−1)`, o sea `L ≥ 714.285,71·f`. Con el 10 % de
    // rentabilidad menos el 5 % que la regla saca, el líquido gana al umbral ~3 puntos al año y
    // el cambio de porcentaje cae dentro del horizonte — el pin dice exactamente cuándo.
    // -----------------------------------------------------------------------------------------
    let mut p16 = base_input(
        240,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(500_000),
            true,
            Some(Decimal::from(10)),
        )],
        vec![rule_remainder(0)],
    );
    p16.annual_inflation_percent = Decimal::from(2);
    p16.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
    p16.phase_plan.income_retirement_monthly = Decimal::from(1_800);
    p16.phase_plan.expense_retirement_monthly = Decimal::from(1_500);
    p16.phase_plan.withdrawal = WithdrawalRule::Hybrid {
        start_pct: Decimal::from(5),
        end_pct: d(35, 1),
    };
    p16.phase_plan.spend_mode = SpendMode::RuleIsSpend;
    out.push(ProjCase {
        name: "P16_hybrid_rule_is_spend",
        input: p16,
    });

    // -----------------------------------------------------------------------------------------
    // P17: `guardrails` 4 / 20 / 10 con techo, impuestos ES e inflación 2,5 % a 40 años.
    //
    // 700.000 € al 4,5 %, gasto de jubilación 2.600 €/mes indexado y sin rentas: la retirada
    // permitida arranca en `4 %·700.000/12 = 2.333,33 €` BRUTOS y se indexa al IPC, mientras la
    // necesidad bruta (con la escala ES por delante) es mayor desde el primer mes — el techo ata
    // SIEMPRE y el recorte es permanente.
    //
    // Lo que este caso pinea, y ninguno más hace, son las **revisiones anuales**: cada 12 meses
    // desde el mes 1 la tasa efectiva `12·W/L(k−1)` se compara con la banda (3,2 %–4,8 %) y el
    // multiplicador se mueve ±10 %. A 480 meses caben 39 revisiones: si alguna se desplaza un mes
    // o el multiplicador se aplica al `W` del año en vez de a la base, el hash lo dice.
    // -----------------------------------------------------------------------------------------
    let mut p17 = base_input(
        480,
        Decimal::from(2_000),
        Decimal::from(3_000),
        vec![mk_asset(1, Decimal::from(700_000), true, Some(d(45, 1)))],
        vec![rule_remainder(0)],
    );
    p17.annual_inflation_percent = d(25, 1);
    p17.tax_brackets = es_tax_brackets_2025_26();
    p17.taxes_enabled = true;
    p17.taxable_gain_ratio = Decimal::ONE;
    p17.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(1);
    p17.phase_plan.income_retirement_monthly = Decimal::ZERO;
    p17.phase_plan.expense_retirement_monthly = Decimal::from(2_600);
    p17.phase_plan.withdrawal = WithdrawalRule::Guardrails {
        pct: Decimal::from(4),
        band_pct: Decimal::from(20),
        adjust_pct: Decimal::from(10),
    };
    p17.phase_plan.spend_mode = SpendMode::Ceiling;
    out.push(ProjCase {
        name: "P17_guardrails_taxes_es",
        input: p17,
    });

    // -----------------------------------------------------------------------------------------
    // P18: **el ejemplo del issue #207** — pensión con fecha y objetivo PUENTE.
    //
    // Gasto de jubilación 2.000 €/mes, SWR 4 %, pensión INDEXADA de 1.200 €/mes desde el índice
    // 240 (20 años), puente descontado al 5 % anual, impuestos ES y `g` MIXTA (dos activos con
    // base declarada), inflación 2 %, 40 años de horizonte.
    //
    // Lo que pinea y ningún otro caso toca: la tabla del puente (240 gross-ups y 241 potencias
    // calculados UNA vez), el escalón del objetivo al llegar `P`, la pensión entrando como
    // INGRESO en un mes ya jubilado, y las dos lecturas nuevas
    // (`bridge_effective_withdrawal_pct`, `pension_coverage_ratio`).
    // -----------------------------------------------------------------------------------------
    let mut p18 = base_input(
        480,
        Decimal::from(3_500),
        Decimal::from(2_000),
        vec![
            // 0 — cuenta al 0 % con coste = valor ⇒ g = 0: drena primero (drain_order).
            mk_asset_with_basis(
                1,
                Decimal::from(10_000),
                true,
                Some(Decimal::ZERO),
                Decimal::from(10_000),
            ),
            // 1 — fondo al 6 % con plusvalía latente ⇒ g = 0,2.
            mk_asset_with_basis(
                2,
                Decimal::from(150_000),
                true,
                Some(Decimal::from(6)),
                Decimal::from(120_000),
            ),
        ],
        vec![rule_remainder(1)],
    );
    p18.annual_inflation_percent = Decimal::from(2);
    p18.tax_brackets = es_tax_brackets_2025_26();
    p18.taxes_enabled = true;
    p18.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p18.phase_plan.pension = Some(PensionSchedule {
        start_index: 240,
        monthly_today: Decimal::from(1_200),
        indexed: true,
        fraction_while_partial: Decimal::ZERO,
    });
    p18.phase_plan.target_basis = TargetBasis::BridgeToPension;
    p18.phase_plan.bridge_discount_annual_pct = Decimal::from(5);
    p18.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(2_000),
            pension_monthly: Decimal::ZERO,
        },
        swr_pct: Decimal::from(4),
        tax_brackets: es_tax_brackets_2025_26(),
        taxes_enabled: true,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::from(2),
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P18_pension_bridge",
        input: p18,
    });

    // -----------------------------------------------------------------------------------------
    // P19: **la pensión cubre el gasto entero** (2.500 contra 2.000, las dos indexadas al 1,5 %)
    // desde el índice 120, con base PERPETUIDAD y un pasivo vivo.
    //
    // Desde `P` la necesidad neta es 0 y el objetivo es SOLO el término de deuda (R6): con
    // 90.000 € líquidos el cruce es inmediato en el mes 121, que es justo lo que el hallazgo B3
    // de la revisión decía que no podía quedarse en `None`. Antes de `P` el objetivo son 600.000
    // (la pensión no se cuenta) más la deuda, y no se cruza.
    // -----------------------------------------------------------------------------------------
    let p19_liab = mk_liab(
        Decimal::from(60_000),
        Decimal::from(500),
        Some(Decimal::from(3)),
        RepaymentModel::French,
        Some(150),
    );
    let mut p19 = base_input(
        360,
        Decimal::from(2_500),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(90_000),
            true,
            Some(Decimal::from(3)),
        )],
        vec![rule_remainder(0)],
    );
    p19.annual_inflation_percent = d(15, 1);
    p19.liabilities = vec![p19_liab];
    p19.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p19.phase_plan.pension = Some(PensionSchedule {
        start_index: 120,
        monthly_today: Decimal::from(2_500),
        indexed: true,
        fraction_while_partial: Decimal::ZERO,
    });
    p19.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(2_000),
            pension_monthly: Decimal::ZERO,
        },
        swr_pct: Decimal::from(4),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: d(15, 1),
        debt_payments_remaining: debt_payments_remaining_series(&p19.liabilities, ref_date()),
    });
    out.push(ProjCase {
        name: "P19_pension_perpetuity_covering",
        input: p19,
    });

    // -----------------------------------------------------------------------------------------
    // P20: **la media jornada del ejemplo del issue** — 1.100 €/mes desde el mes 60, gasto de
    // jubilación 2.000, hueco 900 ⇒ `partial_gap_target = 900·12/0,04` = **270.000 €**.
    //
    // Sin impuestos ni inflación a propósito: el hueco de este caso tiene que salir en el número
    // redondo del issue. La fase come capital (900 €/mes contra ~330 de rentabilidad), así que
    // pinea también el aviso `PartialPhaseCapitalShrinking` y la venta SIN techo de la fase
    // parcial — la regla de retirada gobierna la jubilación, no la media jornada.
    // -----------------------------------------------------------------------------------------
    let mut p20 = base_input(
        300,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(20_000),
            true,
            Some(Decimal::from(5)),
        )],
        vec![rule_remainder(0)],
    );
    p20.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p20.phase_plan.partial = Some(PartialPhase {
        start_month: 60,
        income_monthly: Decimal::from(1_100),
        expense_basis: ExpenseBasis::Retirement,
    });
    p20.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(2_000),
            pension_monthly: Decimal::ZERO,
        },
        swr_pct: Decimal::from(4),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P20_partial_media_jornada",
        input: p20,
    });

    // -----------------------------------------------------------------------------------------
    // P21: **`retire_at_age` con el cruce como LECTURA** (D17).
    //
    // Jubilación forzada en el mes 120 con el objetivo TODAVÍA en el input: `crossing_is_reading_only`
    // impide que el cruce dispare nada, pero `liquid_crossing_month_index` se sigue anotando. Con
    // 400.000 € al 6 %, impuestos ES e inflación 2 %, el capital NO alcanza el objetivo en el mes
    // 120, así que el caso pinea además el aviso `RetireAtAgeUnderfunded`.
    // -----------------------------------------------------------------------------------------
    let mut p21 = base_input(
        360,
        Decimal::from(4_000),
        Decimal::from(2_000),
        vec![mk_asset(
            1,
            Decimal::from(400_000),
            true,
            Some(Decimal::from(6)),
        )],
        vec![rule_remainder(0)],
    );
    p21.annual_inflation_percent = Decimal::from(2);
    p21.tax_brackets = es_tax_brackets_2025_26();
    p21.taxes_enabled = true;
    p21.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(120);
    p21.phase_plan.crossing_is_reading_only = true;
    p21.phase_plan.expense_retirement_monthly = Decimal::from(2_500);
    p21.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(2_500),
            pension_monthly: Decimal::ZERO,
        },
        swr_pct: d(35, 1),
        tax_brackets: es_tax_brackets_2025_26(),
        taxes_enabled: true,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::from(2),
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P21_retire_at_age_reading_only",
        input: p21,
    });

    // -----------------------------------------------------------------------------------------
    // P22: **el escenario del solve**, congelado como caso.
    //
    // Es la ejecución que `required_contribution_monthly` devuelve para el laboratorio de
    // `tests/phases_wp3.rs`: sobrante 2.000 €/mes, techo 1.000, 0 % de rentabilidad, objetivo
    // 100.000 € plano y jubilación por edad en el mes 101. Aquí la aritmética es de una línea —
    // `líquido(k) = 1.000k`, `disposable_cash(k) = 1.000` — y por eso el pin de este caso es el
    // que caza cualquier deriva del techo de aportación o de la identidad
    // `sobrante = invertido + disponible`.
    // -----------------------------------------------------------------------------------------
    let mut p22 = base_input(
        120,
        Decimal::from(5_000),
        Decimal::from(3_000),
        vec![mk_asset(1, Decimal::ZERO, true, Some(Decimal::ZERO))],
        vec![rule_remainder(0)],
    );
    p22.phase_plan.expense_retirement_monthly = Decimal::from(3_000);
    p22.phase_plan.retirement_trigger = RetirementTrigger::AtMonth(101);
    p22.phase_plan.crossing_is_reading_only = true;
    p22.phase_plan.contribution_cap_monthly = Some(Decimal::from(1_000));
    p22.fire_target = Some(FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: Decimal::from(4_000),
        },
        swr_pct: Decimal::from(4),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P22_solve_required_contribution",
        input: p22,
    });

    // -----------------------------------------------------------------------------------------
    // P23: **pausa de ingresos** (P8.c) — dos meses sin nómina desde el mes 2.
    //
    // 5.000 € de cartera al 0 %, +1.000 €/mes, objetivo 10.000: sin pausa se cruza en el mes 6;
    // con ella, dos `+1.000` se convierten en dos `−2.000` y el cruce cae en el mes 12. El caso
    // pinea el vuelco completo, incluidas las dos ventas que la pausa obliga a hacer ANTES de
    // jubilarse (una pausa no es una jubilación: no hay regla de retirada que la tope).
    // -----------------------------------------------------------------------------------------
    let mut p23 = base_input(
        24,
        Decimal::from(3_000),
        Decimal::from(2_000),
        vec![mk_asset(1, Decimal::from(5_000), true, Some(Decimal::ZERO))],
        vec![rule_remainder(0)],
    );
    p23.phase_plan.expense_retirement_monthly = Decimal::from(2_000);
    p23.phase_plan.income_pause = Some(IncomePause {
        from_month: 2,
        months: 2,
        income_fraction: Decimal::ZERO,
    });
    p23.fire_target = Some(FireTarget {
        need: FireNeed::Indexed {
            annual_net_today: Decimal::from(400),
        },
        swr_pct: Decimal::from(4),
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: Vec::new(),
    });
    out.push(ProjCase {
        name: "P23_income_pause",
        input: p23,
    });

    // -----------------------------------------------------------------------------------------
    // P24: **la ESCALA del descubierto en la vía mixta** — reproductor mínimo del hallazgo F1 de
    // la revisión adversarial (fuzz diferencial contra 4.15.0, semilla 0x5EED1234ABCD0001 caso
    // #2, reducido por el shrinker a estas 12 líneas).
    //
    // Un solo mes. Dos activos LÍQUIDOS con `g` distinta (el 0 sin base declarada ⇒ `g` = el
    // escalar = 1; el 1 con base 588.831 sobre un valor de 294.416 ⇒ `b/v > 1` ⇒ `g` = 0), así
    // que el drenaje va por el PASEO MIXTO. El pasivo mete dos amortizaciones puntuales en el
    // mes 1 que llevan la necesidad a 28.417,03 € — escala 2 — y la cartera (682.878 €) la cubre
    // entera, de modo que el paseo publica `net_shortfall_monthly = 0` con **escala 0**.
    //
    // 4.15.0 acumulaba ese operando tal cual (`undrained_cumulative += dd.net_shortfall_monthly`)
    // y publicaba `uncovered_deficit_total` = **`0`**. Re-derivarlo como
    // `need − (need − shortfall)` da el mismo VALOR con otra escala — **`0.00`** — y eso es un
    // `Display` distinto, que es exactamente lo que el pin dorado hashea. 438 de 3.000 entradas
    // del fuzz cayeron por aquí y NINGÚN caso de la batería lo tocaba.
    // -----------------------------------------------------------------------------------------
    let mut p24 = base_input(
        1,
        Decimal::ZERO,
        Decimal::from(141),
        vec![
            mk_asset(1, Decimal::from(388_462), true, Some(Decimal::ZERO)),
            mk_asset_with_basis(
                2,
                Decimal::from(294_416),
                true,
                Some(Decimal::ZERO),
                Decimal::from(588_831),
            ),
        ],
        vec![],
    );
    p24.taxable_gain_ratio = Decimal::ONE;
    p24.liabilities = vec![ProjectionLiabilityInput {
        principal: Decimal::from(118_187),
        monthly_payment: Decimal::from(228),
        payment_end: None,
        repayment_model: RepaymentModel::FixedPayments,
        apr_percent: None,
        min_payment_pct: None,
        min_payment_eur: None,
        extra_principal_monthly: Decimal::ZERO,
        extra_principal_lump_sums: vec![(1, d(1_490_733, 2)), (1, d(1_059_170, 2))],
        early_repayment_fee_pct: None,
        early_repayment_effect: EarlyRepaymentEffect::ReduceTerm,
    }];
    p24.phase_plan = PhasePlan::forced_at(1, Decimal::ZERO, Decimal::from(2_690), Decimal::ZERO);
    out.push(ProjCase {
        name: "P24_undrained_scale",
        input: p24,
    });

    // -----------------------------------------------------------------------------------------
    // P25: **la ASOCIATIVIDAD del servicio de deuda** — reproductor mínimo del hallazgo F2 de la
    // revisión (mismo fuzz, caso #1911 reducido).
    //
    // DOS pasivos, y ahí está todo: 4.15.0 escribía `debt_service += cash + extra + fee`, o sea
    // `acc + ((cash + extra) + fee)`. El refactor genérico lo desparejó en `((acc + cash) + extra)
    // + fee` — la misma álgebra, y NO el mismo número: cada suma de `Decimal` redondea a 28
    // dígitos. Con un solo pasivo el acumulador vale 0 y las dos formas coinciden; hace falta el
    // segundo para que `acc` llegue con dígitos propios.
    //
    // El pasivo 0 es de cuota fija con amortización extra recurrente y efecto «reducir cuota»
    // (la cuota se recalcula con una DIVISIÓN, que es la que fabrica la cola de 28 dígitos); el 1
    // es revolving con mínimo porcentual. La divergencia sale en `per_asset_series[1][13]`:
    // `…422555` (4.15.0) frente a `…422545`.
    // -----------------------------------------------------------------------------------------
    let mut p25 = base_input(
        18,
        Decimal::ZERO,
        Decimal::from(1_204),
        vec![
            mk_asset(1, Decimal::from(355_772), true, Some(Decimal::ZERO)),
            mk_asset(2, Decimal::from(71_315), true, Some(Decimal::from(-40))),
        ],
        vec![],
    );
    p25.taxable_gain_ratio = Decimal::ZERO;
    p25.liabilities = vec![
        ProjectionLiabilityInput {
            principal: Decimal::from(218_138),
            monthly_payment: Decimal::from(2_178),
            payment_end: None,
            repayment_model: RepaymentModel::FixedPayments,
            apr_percent: None,
            min_payment_pct: None,
            min_payment_eur: None,
            extra_principal_monthly: d(4_703, 1),
            extra_principal_lump_sums: vec![],
            early_repayment_fee_pct: None,
            early_repayment_effect: EarlyRepaymentEffect::ReducePayment,
        },
        ProjectionLiabilityInput {
            principal: Decimal::from(4_555),
            monthly_payment: Decimal::from(647),
            payment_end: None,
            repayment_model: RepaymentModel::Revolving,
            apr_percent: None,
            min_payment_pct: Some(d(2_071, 3)),
            min_payment_eur: None,
            extra_principal_monthly: d(3_215_707, 4),
            extra_principal_lump_sums: vec![],
            early_repayment_fee_pct: None,
            early_repayment_effect: EarlyRepaymentEffect::ReduceTerm,
        },
    ];
    p25.phase_plan = PhasePlan::classic(Decimal::ZERO, Decimal::from(148));
    out.push(ProjCase {
        name: "P25_debt_service_assoc",
        input: p25,
    });

    out
}

/// Lo que `audit_dump` vuelca al CSV: la batería histórica (P1–P6 y P13) en su orden, **más** los
/// casos de 5.0.0. El formato del CSV es un contrato con un oráculo externo y no crece sin que se
/// declare: creció una vez en WP1a (P13, la regresión de #208) y otra en WP2 (P14–P17, el techo
/// numérico y las reglas de retirada). Un caso del pin que NO esté aquí es deliberado: los P7–P12
/// existen para que el hash cubra el motor entero, no para que el oráculo los reproduzca.
pub fn projection_cases_dumped() -> Vec<ProjCase> {
    let mut out = projection_cases_audit();
    out.extend(projection_cases_5_0());
    out
}
