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

#![allow(dead_code)]

use chrono::NaiveDate;
use futurefin_engine::{
    debt_payments_remaining_series, AllocationCap, AllocationKind, AllocationRule, FireNeed,
    FireTarget, PhasePlan, ProjectionInput, ProjectionLiabilityInput, RepaymentModel,
    RetirementTrigger, SimAsset, TaxBracket,
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

/// Todos los casos de proyección: los del CSV primero, en su orden, y luego los añadidos.
pub fn projection_cases_all() -> Vec<ProjCase> {
    let mut out = projection_cases_audit();
    out.extend(projection_cases_extended());
    out
}
