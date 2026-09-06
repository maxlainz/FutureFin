//! **Arnés aleatorio de identidades contables del motor** — 500 entradas por semilla, sin
//! depender de ningún engine anterior ni de ninguna dependencia nueva.
//!
//! # Por qué existe
//!
//! El pin dorado (`golden_pins.rs`) demuestra que 25 casos ELEGIDOS no se mueven. No demuestra
//! que las identidades del modelo se cumplan en el caso que nadie eligió, y ese es justo el
//! agujero que encontró la segunda revisión adversarial (D20): el fuzz diferencial contra 4.15.0
//! cazó dos regresiones de bit-identidad —la ESCALA del descubierto en la vía mixta y la
//! AGRUPACIÓN del servicio de deuda— que **ningún caso de la batería tocaba**, y una violación de
//! contrato del propio 4.15.0 (`uncovered_deficit_total > 0` con `assets_depleted_month_index =
//! None`, 571 veces en 9.000 entradas).
//!
//! Aquel fuzz necesitaba el motor de 4.15.0 compilado al lado. Este no necesita nada: comprueba
//! que la salida es **coherente consigo misma**, así que sobrevive a los cambios de semántica que
//! un pin diferencial no puede sobrevivir.
//!
//! # Qué afirma
//!
//! | # | Invariante |
//! |---|---|
//! | 1 | Longitudes: toda serie mide `horizon + 1`, y el mes 0 de las series de FLUJO es cero |
//! | 2 | Las tres magnitudes del mes cierran: `retirada + recorte + descubierto − sobrante = necesidad` |
//! | 3 | Balance: `NW(k) = Σ activos(k) − descubierto acumulado(k)` (sin pasivos por construcción) |
//! | 4 | Signos: ninguna de las tres magnitudes publicadas es negativa |
//! | 5 | `uncovered_deficit_total > 1e-12 ⇒ assets_depleted_month_index` existe |
//! | 6 | Agotamiento ⇒ alguna necesidad quedó sin cubrir en ese mes o después (modo `ceiling`) |
//! | 7 | Determinismo: dos ejecuciones de la MISMA entrada dan el mismo `Display`, dígito a dígito |
//! | 8 | La necesidad ORDINARIA no lleva deuda ni «Próximos»: `ordinaria ≤ neta + deuda + max(0, próximo)` |
//! | 9 | El veredicto del camino: los dos latches se fijan a la vez, en rango, y en el PRIMER mes |
//!
//! # El generador
//!
//! Un LCG de 64 bits (constantes de Knuth/MMIX), sin `rand`: el freezer de este crate prohíbe
//! RNG en el motor, y una dependencia de test que no existe no puede colarse en el binario.
//! Los hogares se sortean SIN pasivos, SIN flujos puntuales y SIN inflación, y con la jubilación
//! forzada en el mes 1. No es pereza: así la necesidad neta del mes es una CONSTANTE que el test
//! conoce (`max(0, gasto_jubilado − ingreso_jubilado − pensión)`), y el invariante 2 —el que de
//! verdad ata las tres magnitudes— se puede comprobar sin reimplementar el bucle dentro del test.
//! Lo que sí se sortea es todo lo que decide RAMA: número de activos, base de coste declarada o
//! no (vía escalar contra paseo mixto), liquidez, rentabilidades, impuestos y escala, reglas de
//! retirada, modo de gasto, cascada y pensión con fecha.

#[path = "common/cases.rs"]
mod cases;

use cases::es_tax_brackets_2025_26;
use futurefin_engine::{
    project_net_worth_series, simulate, AllocationCap, AllocationKind, AllocationRule, PathFailure,
    PensionSchedule, PhasePlan, ProjectionInput, ProjectionOutput, SimAsset, SimInput, SimOutput,
    SpendMode, WithdrawalRule,
};
use rust_decimal::Decimal;
use uuid::Uuid;

/// Cuántas entradas por semilla. 500 tarda ~1 s en `--release` y ~6 s en `debug`.
const CASES: u64 = 500;

/// Tolerancia de las identidades, en euros. El motor acumula el operando LITERAL de 4.15.0 y
/// `after_tax(gross_up(n))` devuelve `n` solo hasta el redondeo a 28 dígitos: la cola medida en
/// el corpus diferencial llega a ~5e-23 €. `1e-12` es doce órdenes de magnitud por encima de esa
/// cola y doce por debajo de un céntimo — no puede tapar un error de dinero.
const EPS: Decimal = Decimal::from_parts(1, 0, 0, false, 12);

// =================================================================================================
// Generador
// =================================================================================================

/// LCG de 64 bits (MMIX de Knuth). Determinista, sin dependencias y suficiente para elegir ramas.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // Los bits ALTOS son los buenos en un LCG; devolverlos ya mezclados evita que
        // `next_range(2)` se convierta en «par/impar del contador».
        self.0 ^ (self.0 >> 31)
    }
    /// Entero en `[0, n)`.
    fn upto(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// ¿Cara? con probabilidad `1/n`.
    fn one_in(&mut self, n: u64) -> bool {
        self.upto(n) == 0
    }
    /// Un importe en `[0, max]` con dos decimales.
    fn money(&mut self, max: u64) -> Decimal {
        Decimal::new(self.upto(max * 100 + 1) as i64, 2)
    }
    /// Un porcentaje en `[-12, 12]` con un decimal.
    fn pct(&mut self) -> Decimal {
        Decimal::new(self.upto(241) as i64 - 120, 1)
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.upto(xs.len() as u64) as usize]
    }
}

/// Un hogar aleatorio. Ver el doc del módulo para lo que se sortea y lo que NO, y por qué.
fn gen_case(seed: u64, i: u64) -> ProjectionInput {
    let mut r = Lcg(seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let horizon = r.pick(&[1u32, 2, 6, 12, 36, 120, 240]);
    let n_assets = 1 + r.upto(4) as usize;
    // La ESCALA de la cartera se sortea aparte del valor: sin hogares pequeños casi ninguno se
    // arruina en 240 meses, y el invariante del agotamiento —el que 4.15.0 rompía— no se
    // ejercitaría. El control de vida del final vigila que siga siendo así.
    let scale = r.pick(&[2_000u64, 30_000, 200_000]);
    let assets: Vec<SimAsset> = (0..n_assets)
        .map(|j| SimAsset {
            id: Uuid::from_u128(j as u128 + 1),
            value: r.money(scale),
            // La base declarada es la que manda la venta por la vía ESCALAR o por el paseo
            // MIXTO: un tercio sin base, un tercio con base por debajo del valor y un tercio con
            // base por encima (activo bajo el agua ⇒ `g = 0` por el clamp).
            purchase_price: match r.upto(3) {
                0 => None,
                1 => Some(r.money(scale)),
                _ => Some(r.money(scale * 2)),
            },
            is_liquid: !r.one_in(4),
            expected_annual_return_percent: if r.one_in(6) { None } else { Some(r.pct()) },
        })
        .collect();
    let rules: Vec<AllocationRule> = (0..r.upto(3))
        .map(|_| {
            let target_index = r.upto(n_assets as u64) as usize;
            match r.upto(3) {
                0 => AllocationRule {
                    target_index,
                    kind: AllocationKind::Fixed,
                    amount: Some(r.money(2_000)),
                    cap: r.one_in(3).then(|| AllocationCap::Amount(r.money(50_000))),
                },
                1 => AllocationRule {
                    target_index,
                    kind: AllocationKind::Percent,
                    amount: Some(Decimal::from(r.upto(101) as u32)),
                    cap: r
                        .one_in(3)
                        .then(|| AllocationCap::MonthsExpense(Decimal::from(r.upto(24) as u32))),
                },
                _ => AllocationRule {
                    target_index,
                    kind: AllocationKind::Remainder,
                    amount: None,
                    cap: None,
                },
            }
        })
        .collect();

    let income_ret = r.money(3_000);
    let expense_ret = r.money(4_000);
    let mut plan = PhasePlan::forced_at(1, income_ret, expense_ret, Decimal::ZERO);
    plan.withdrawal = match r.upto(4) {
        0 => WithdrawalRule::FixedReal,
        1 => WithdrawalRule::PercentOfBalance {
            pct: Decimal::new(r.upto(200) as i64 + 1, 1),
        },
        2 => WithdrawalRule::Hybrid {
            start_pct: Decimal::new(r.upto(100) as i64 + 20, 1),
            end_pct: Decimal::new(r.upto(100) as i64 + 1, 1),
        },
        _ => WithdrawalRule::Guardrails {
            pct: Decimal::new(r.upto(150) as i64 + 5, 1),
            band_pct: Decimal::from(r.upto(40) as u32 + 5),
            adjust_pct: Decimal::from(r.upto(40) as u32 + 5),
        },
    };
    plan.spend_mode = if r.one_in(3) {
        SpendMode::RuleIsSpend
    } else {
        SpendMode::Ceiling
    };
    // Pensión con fecha, plana (sin IPC en este arnés, «indexada» y «plana» coinciden) y siempre
    // desde el mes 1 del bucle (índice 0): así entra en la necesidad de TODOS los meses y la
    // constante que el test conoce sigue siendo una constante.
    let pension_monthly = if r.one_in(3) {
        let m = r.money(2_000);
        plan.pension = Some(PensionSchedule {
            start_index: 0,
            monthly_today: m,
            indexed: false,
            fraction_while_partial: Decimal::ZERO,
        });
        m
    } else {
        Decimal::ZERO
    };
    let _ = pension_monthly;

    let taxes_enabled = !r.one_in(3);
    ProjectionInput {
        ref_date: cases::ref_date(),
        horizon_months: horizon,
        annual_inflation_percent: Decimal::ZERO,
        tax_brackets: if taxes_enabled {
            es_tax_brackets_2025_26()
        } else {
            Vec::new()
        },
        taxes_enabled,
        taxable_gain_ratio: Decimal::new(r.upto(11) as i64, 1),
        income_regular_monthly: r.money(5_000),
        expense_regular_monthly: r.money(3_000),
        assets,
        allocation_rules: rules,
        liabilities: Vec::new(),
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        phase_plan: plan,
        fire_target: None,
    }
}

/// La necesidad NETA que el bucle ve cada mes en las entradas de este arnés: jubilado desde el
/// mes 1, sin IPC, sin pasivos y sin flujos puntuales, así que es una constante.
fn need_net_of(input: &ProjectionInput) -> Decimal {
    let pension = input
        .phase_plan
        .pension
        .map_or(Decimal::ZERO, |p| p.monthly_today);
    (input.phase_plan.expense_retirement_monthly
        - input.phase_plan.income_retirement_monthly
        - pension)
        .max(Decimal::ZERO)
}

/// Texto canónico de TODA la salida, para el control de determinismo.
fn render(o: &ProjectionOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(
        s,
        "{:?}|{}|{}|",
        o.assets_depleted_month_index, o.uncovered_deficit_total, o.unallocated_savings_total
    );
    for v in o
        .net_worth
        .iter()
        .chain(&o.liquid_worth)
        .chain(&o.contributed_capital)
        .chain(&o.withdrawal)
        .chain(&o.withdrawal_shortfall)
        .chain(&o.withdrawal_excess)
        .chain(&o.unmet_need)
    {
        let _ = write!(s, "{v},");
    }
    for a in &o.per_asset_series {
        for v in a {
            let _ = write!(s, "{v},");
        }
    }
    s
}

// =================================================================================================
// El test
// =================================================================================================

/// Las SIETE identidades sobre una salida. Devuelve `Err(motivo)` en vez de reventar para que el
/// control negativo de abajo pueda comprobar que cada una de ellas se entera de verdad: un
/// detector sin control negativo es un test que siempre pasa.
fn check(
    name: &str,
    input: &ProjectionInput,
    out: &ProjectionOutput,
    sim: &SimOutput<Decimal>,
) -> Result<(), String> {
    let h = input.horizon_months as usize;
    let fail = |m: String| Err(format!("{name}: {m}"));

    // (1) LONGITUDES y el cero del mes 0.
    for (label, serie) in [
        ("net_worth", &out.net_worth),
        ("liquid_worth", &out.liquid_worth),
        ("contributed_capital", &out.contributed_capital),
        ("withdrawal", &out.withdrawal),
        ("withdrawal_shortfall", &out.withdrawal_shortfall),
        ("withdrawal_excess", &out.withdrawal_excess),
        ("unmet_need", &out.unmet_need),
        ("disposable_cash", &out.disposable_cash),
    ] {
        if serie.len() != h + 1 {
            return fail(format!("longitud de {label}: {} ≠ {}", serie.len(), h + 1));
        }
        if serie[0] != Decimal::ZERO
            && matches!(
                label,
                "withdrawal" | "withdrawal_shortfall" | "withdrawal_excess" | "unmet_need"
            )
        {
            return fail(format!("{label}[0] = {} y tiene que ser 0", serie[0]));
        }
    }
    if out.per_asset_series.len() != input.assets.len() {
        return fail("per_asset_series no tiene un vector por activo".into());
    }
    for a in &out.per_asset_series {
        if a.len() != h + 1 {
            return fail("longitud de per_asset_series".into());
        }
    }

    // (2) LAS TRES MAGNITUDES CIERRAN, mes a mes:
    //     retirada + recorte + descubierto − sobrante = necesidad.
    //     El sobrante resta porque es gasto DISCRECIONAL: euros que la regla mandó gastar por
    //     encima de la necesidad y que por tanto ya están dentro de `withdrawal`.
    let need = need_net_of(input);
    for k in 1..=h {
        let closed = out.withdrawal[k] + out.withdrawal_shortfall[k] + out.unmet_need[k]
            - out.withdrawal_excess[k];
        if (closed - need).abs() > EPS {
            return fail(format!(
                "mes {k}: {} + {} + {} − {} = {closed} ≠ necesidad {need}",
                out.withdrawal[k],
                out.withdrawal_shortfall[k],
                out.unmet_need[k],
                out.withdrawal_excess[k]
            ));
        }
    }

    // (3) BALANCE. Sin pasivos, `NW(k) = Σ activos(k) − descubierto acumulado(k)`.
    let mut undrained = Decimal::ZERO;
    for k in 0..=h {
        undrained += out.unmet_need[k];
        let assets: Decimal = out.per_asset_series.iter().map(|a| a[k]).sum();
        if (out.net_worth[k] - (assets - undrained)).abs() > EPS {
            return fail(format!(
                "mes {k}: NW {} ≠ Σactivos {assets} − descubierto {undrained}",
                out.net_worth[k]
            ));
        }
    }
    if (out.uncovered_deficit_total - undrained).abs() > EPS {
        return fail(format!(
            "el total del descubierto ({}) no es la suma de la serie ({undrained})",
            out.uncovered_deficit_total
        ));
    }

    // (4) SIGNOS: lo que se publica no lleva números negativos.
    for k in 0..=h {
        for (label, v) in [
            ("retirada", out.withdrawal[k]),
            ("recorte", out.withdrawal_shortfall[k]),
            ("sobrante", out.withdrawal_excess[k]),
            ("descubierto", out.unmet_need[k]),
        ] {
            if v < Decimal::ZERO {
                return fail(format!("{label} del mes {k} es negativo: {v}"));
            }
        }
    }

    // (5) UN DESCUBIERTO REAL IMPLICA AGOTAMIENTO. Es el contrato que 4.15.0 rompía en 571 de
    //     9.000 entradas del corpus diferencial (hasta 168.685 € de descubierto con
    //     `assets_depleted_month_index = None`): el paseo mixto comparaba `Σ(cap·12)/12` con
    //     `Σ cap` y el `>=` fallaba por un ULP.
    if out.uncovered_deficit_total > EPS && out.assets_depleted_month_index.is_none() {
        return fail(format!(
            "descubierto de {} € y la cartera «nunca se agotó»",
            out.uncovered_deficit_total
        ));
    }

    // (6) Y EL AGOTAMIENTO IMPLICA NECESIDAD SIN CUBRIR desde ese mes en adelante — modo
    //     `ceiling`, donde toda venta persigue la necesidad. Sin esto, un puente que se vacía
    //     justo cuando entra una pensión que cubre el gasto posterior salía como «cartera
    //     agotada» con cero euros sin cubrir.
    if let Some(d) = out.assets_depleted_month_index {
        if d < 1 || d as usize > h {
            return fail(format!("mes de agotamiento {d} fuera del horizonte"));
        }
        if input.phase_plan.spend_mode == SpendMode::Ceiling && need > Decimal::ZERO {
            let after: Decimal = out.unmet_need[d as usize..=h].iter().copied().sum();
            if after <= Decimal::ZERO {
                return fail(format!(
                    "agotada en el mes {d} y ni un euro de necesidad sin cubrir después"
                ));
            }
        }
    }

    // (8) LA NECESIDAD ORDINARIA NO LLEVA NI DEUDA NI «PRÓXIMOS» (E1, C1).
    //
    //     La identidad DEMOSTRABLE, con `A = gasto + retirada extra − ingresos` (la ordinaria es
    //     `max(0, A)`) y `neta = max(0, A + deuda − próximo)`:
    //
    //         ordinaria ≤ neta + deuda + max(0, próximo)
    //
    //     El signo del término de «Próximos» es el POSITIVO, no el negativo: un ingreso puntual
    //     BAJA la necesidad neta sin bajar la ordinaria (es justo el caso que C1 excluye del
    //     veredicto), mientras que un gasto puntual la sube y la desigualdad se cumple sola.
    //     Con `próximo` negativo o nulo, `ordinaria ≤ neta + deuda`.
    //
    //     Este arnés sortea hogares SIN pasivos y SIN flujos puntuales, así que aquí los dos
    //     términos son cero y la desigualdad se aprieta hasta la IGUALDAD con la constante que
    //     el test conoce: la necesidad ordinaria ES la neta. Un mes con deuda o con un Próximo
    //     que las separe vive en `phases_wp3.rs`
    //     (`f3_ignores_the_mortgage_and_looks_at_the_ordinary_need`).
    if sim.ordinary_need.len() != h + 1 {
        return fail(format!(
            "longitud de ordinary_need: {} ≠ {}",
            sim.ordinary_need.len(),
            h + 1
        ));
    }
    if sim.ordinary_need[0] != Decimal::ZERO {
        return fail(format!(
            "ordinary_need[0] = {} y tiene que ser 0",
            sim.ordinary_need[0]
        ));
    }
    for k in 1..=h {
        let debt = Decimal::ZERO; // el generador no sortea pasivos
        let planning = input.planning_monthly_cash_adjustment[k - 1];
        let bound = need + debt + planning.max(Decimal::ZERO);
        if sim.ordinary_need[k] > bound + EPS {
            return fail(format!(
                "mes {k}: necesidad ordinaria {} > neta {need} + deuda {debt} + próximo {}",
                sim.ordinary_need[k],
                planning.max(Decimal::ZERO)
            ));
        }
        if (sim.ordinary_need[k] - need).abs() > EPS {
            return fail(format!(
                "mes {k}: sin deuda ni Próximos la necesidad ordinaria ({}) ES la neta ({need})",
                sim.ordinary_need[k]
            ));
        }
    }

    // (9) EL VEREDICTO DEL CAMINO: los dos latches se fijan A LA VEZ, dentro del horizonte, y en
    //     el PRIMER mes en que algo falla — no en uno posterior.
    if out.failure_kind.is_some() != out.failure_month_index.is_some() {
        return fail(format!(
            "los latches del fallo no cuadran: mes {:?}, motivo {:?}",
            out.failure_month_index, out.failure_kind
        ));
    }
    if out.failure_kind == Some(PathFailure::InitialRateExceeded) {
        return fail("ningún caso del arnés declara puerta de tasa inicial".into());
    }
    let rule_has_ceiling = input.phase_plan.withdrawal != WithdrawalRule::FixedReal;
    let first_f1 = (1..=h).find(|k| out.unmet_need[*k] > EPS);
    // Con techo, «la regla permitió menos que la necesidad ordinaria» ⟺ hubo recorte: en este
    // arnés la necesidad ordinaria y la neta coinciden (ver (8)), y el recorte ES su diferencia
    // contra lo que la regla permitió.
    let first_f3 = rule_has_ceiling
        .then(|| (1..=h).find(|k| out.withdrawal_shortfall[*k] > EPS))
        .flatten();
    match out.failure_month_index {
        Some(m) => {
            if m < 1 || m as usize > h {
                return fail(format!("mes de fallo {m} fuera del horizonte"));
            }
            let expected = match (first_f1, first_f3) {
                (Some(a), Some(b)) => a.min(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => {
                    return fail(format!(
                        "fallo publicado en el mes {m} sin descubierto ni recorte que lo explique"
                    ))
                }
            };
            // El latch es MONÓTONO: el mes publicado es el PRIMERO en que algo se rompió. Un
            // latch que se moviera publicaría un mes posterior.
            if m as usize > expected {
                return fail(format!(
                    "el fallo se publica en el mes {m} y el primer mes roto es el {expected}:                      el latch no es monótono"
                ));
            }
            let kind_ok = match out.failure_kind {
                Some(PathFailure::PortfolioDepleted) => out.unmet_need[m as usize] > EPS,
                Some(PathFailure::RuleBelowNeed) => {
                    out.withdrawal_shortfall[m as usize] > EPS && out.unmet_need[m as usize] <= EPS
                }
                _ => false,
            };
            if !kind_ok {
                return fail(format!(
                    "mes {m}: el motivo {:?} no cuadra con descubierto {} y recorte {}",
                    out.failure_kind,
                    out.unmet_need[m as usize],
                    out.withdrawal_shortfall[m as usize]
                ));
            }
        }
        None => {
            if let Some(k) = first_f1 {
                return fail(format!(
                    "descubierto de {} € en el mes {k} y el camino «no falla»",
                    out.unmet_need[k]
                ));
            }
            if let Some(k) = first_f3 {
                return fail(format!(
                    "la regla recortó {} € en el mes {k} y el camino «no falla»",
                    out.withdrawal_shortfall[k]
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn random_households_satisfy_the_accounting_identities() {
    // Tres semillas fijas: el arnés es una RED, no un sorteo distinto en cada CI. Una que falla
    // hoy tiene que fallar mañana con el mismo número de caso.
    for seed in [0x5EED_0001u64, 0x5EED_0002, 0x5EED_0003] {
        let mut simulated = 0u64;
        let mut with_depletion = 0u64;
        let mut with_cut = 0u64;
        let mut with_excess = 0u64;
        for i in 0..CASES {
            let input = gen_case(seed, i);
            // Se llama al NÚCLEO y se convierte, en vez de a `project_net_worth_series`: es la
            // misma simulación (aquella función es este par de líneas) y así la necesidad
            // ORDINARIA —que no se refleja en `ProjectionOutput`— llega al arnés sin pagar una
            // segunda proyección.
            let Ok(sim) = simulate(&SimInput::<Decimal>::from(&input)) else {
                // Un error TIPADO es una respuesta válida del motor (desbordamiento de un valor,
                // regla imposible). Lo que este arnés prohíbe es el pánico, y un pánico aquí hace
                // fallar el test por sí solo.
                continue;
            };
            simulated += 1;
            let out = ProjectionOutput::from(sim.clone());
            let name = format!("seed {seed:#x} caso #{i}");
            if let Err(why) = check(&name, &input, &out, &sim) {
                panic!("{why}");
            }
            if out.assets_depleted_month_index.is_some() {
                with_depletion += 1;
            }
            if out.withdrawal_shortfall.iter().any(|v| *v > Decimal::ZERO) {
                with_cut += 1;
            }
            if out.withdrawal_excess.iter().any(|v| *v > Decimal::ZERO) {
                with_excess += 1;
            }

            // (7) DETERMINISMO: la misma entrada, dos veces, el mismo `Display`.
            let again = project_net_worth_series(&input).expect("la segunda vez tampoco falla");
            assert_eq!(
                render(&out),
                render(&again),
                "{name}: dos ejecuciones de la MISMA entrada no coinciden"
            );
        }

        // Control de vida del generador: un arnés que no ejercita ninguna rama interesante pasa
        // siempre y no prueba nada. Estas cotas son holgadas a propósito — miden que el sorteo
        // sigue produciendo hogares que se arruinan, que recortan y que gastan de más.
        println!(
            "[fuzz_invariants] semilla {seed:#x}: {simulated}/{CASES} simulados · \
             {with_depletion} con agotamiento · {with_cut} con recorte · {with_excess} con sobrante"
        );
        assert!(
            simulated >= CASES * 9 / 10,
            "semilla {seed:#x}: solo {simulated} de {CASES} entradas simulan"
        );
        assert!(
            with_depletion >= CASES / 25,
            "semilla {seed:#x}: solo {with_depletion} hogares se agotan — el generador se ha vuelto plácido"
        );
        assert!(
            with_cut >= CASES / 25,
            "semilla {seed:#x}: solo {with_cut} hogares sufren recorte de la regla"
        );
        assert!(
            with_excess >= CASES / 50,
            "semilla {seed:#x}: solo {with_excess} hogares gastan por encima de la necesidad"
        );
    }
}

/// **Control negativo: cada invariante se entera.** Se toma un hogar que se arruina, se le mueve
/// UN número a la salida y se comprueba que `check` lo caza. Sin esto, las 3.500 comprobaciones
/// de arriba podrían estar pasando porque no miran nada.
#[test]
fn the_harness_notices_a_single_broken_identity() {
    // Un caso con agotamiento y descubierto: es el que ejercita los nueve carriles.
    let (input, base_sim) = (0..CASES)
        .map(|i| gen_case(0x5EED_0001, i))
        .filter_map(|inp| {
            let sim = simulate(&SimInput::<Decimal>::from(&inp)).ok()?;
            let out = ProjectionOutput::from(sim.clone());
            (out.assets_depleted_month_index.is_some()
                && out.uncovered_deficit_total > Decimal::ONE)
                .then_some((inp, sim))
        })
        .next()
        .expect("el generador produce hogares arruinados");
    let base = ProjectionOutput::from(base_sim.clone());
    assert!(
        check("base", &input, &base, &base_sim).is_ok(),
        "el caso base cumple"
    );
    let dep = base.assets_depleted_month_index.expect("se agota");
    let failed_at = base
        .failure_month_index
        .expect("un hogar con descubierto REAL y jubilado desde el mes 1 falla por F1");
    let cent = Decimal::new(1, 2);

    type Mutation = Box<dyn Fn(&mut ProjectionOutput, &mut SimOutput<Decimal>)>;
    let mutations: Vec<(&str, Mutation)> = vec![
        (
            "longitud",
            Box::new(|o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| {
                o.unmet_need.pop();
            }),
        ),
        (
            "mes 0 de una serie de flujo",
            Box::new(move |o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| o.withdrawal[0] = cent),
        ),
        (
            "las tres magnitudes no cierran",
            Box::new(move |o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| o.withdrawal[1] += cent),
        ),
        (
            "el balance no cuadra",
            Box::new(move |o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| o.net_worth[1] += cent),
        ),
        (
            "un signo negativo",
            Box::new(move |o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| {
                o.withdrawal_shortfall[1] = -cent;
                o.withdrawal[1] += cent;
            }),
        ),
        (
            "descubierto sin agotamiento",
            Box::new(|o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| o.assets_depleted_month_index = None),
        ),
        (
            "agotamiento sin necesidad sin cubrir",
            Box::new(move |o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| {
                o.assets_depleted_month_index = Some(dep.min(1));
                for k in dep as usize..o.unmet_need.len() {
                    o.withdrawal[k] += o.unmet_need[k];
                    o.unmet_need[k] = Decimal::ZERO;
                }
            }),
        ),
        (
            "la necesidad ordinaria se lleva algo que no es suyo",
            Box::new(move |_o: &mut ProjectionOutput, s: &mut SimOutput<Decimal>| {
                s.ordinary_need[1] += cent;
            }),
        ),
        (
            "un latch del fallo sin el otro",
            Box::new(|o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| {
                o.failure_kind = None;
            }),
        ),
        (
            "el fallo se publica más tarde de lo que ocurrió",
            Box::new(move |o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| {
                o.failure_month_index = Some(failed_at + 1);
                // …y con un motivo que ese mes sí podría justificar, para que lo que caza el
                // arnés sea la MONOTONÍA del latch y no el motivo.
                o.failure_kind = Some(PathFailure::PortfolioDepleted);
                o.unmet_need[failed_at as usize + 1] += Decimal::ONE;
            }),
        ),
        (
            "un camino que falla y dice que no",
            Box::new(|o: &mut ProjectionOutput, _s: &mut SimOutput<Decimal>| {
                o.failure_month_index = None;
                o.failure_kind = None;
            }),
        ),
    ];
    for (label, mutate) in mutations {
        let mut m = base.clone();
        let mut ms = base_sim.clone();
        mutate(&mut m, &mut ms);
        assert!(
            check("mutado", &input, &m, &ms).is_err(),
            "el arnés NO se entera de: {label}"
        );
    }
}
