//! **Las puertas de Monte Carlo** (WP6 de 5.0.0, §B.5 del plan de la issue #207).
//!
//! Cinco cosas se prueban aquí, y las cinco son requisitos de la skill
//! `futurefin-research-frontier` §6 para que la palabra «Monte Carlo» pueda aparecer en un texto
//! público:
//!
//! 1. **Reproducibilidad por semilla** — misma entrada + misma semilla ⇒ el MISMO resultado, bit
//!    a bit (`mc_same_seed_bit_identical`), y semillas distintas ⇒ resultados distintos.
//! 2. **Degeneración con volatilidad cero** — sin volatilidad declarada, cada banda ES la serie
//!    determinista (`mc_zero_volatility_degenerates_to_deterministic`), en `f64` bit a bit y
//!    contra el motor `Decimal` dentro de las cotas de WP5.5.
//! 3. **Orden de las bandas** — p10 ≤ p50 ≤ p90 en todos los meses.
//! 4. **El modelo hace lo que dice** — la media del terminal sobre 2 000 caminos coincide con el
//!    terminal determinista dentro de la tolerancia DERIVADA de la varianza log-normal.
//! 5. **Los números del issue** — la tabla del #207 (6,5 % media / 17 % sd, 35 años, 3 % vs 4 %)
//!    reproducida dentro de horquillas anchas y declaradas, con los valores IMPRESOS.
//!
//! Las cifras que estos tests imprimen son la evidencia; las horquillas son anchas a propósito
//! porque los números del issue vienen de FUERA de la app y de otro modelo (normal anual frente a
//! log-normal mensual). Una horquilla estrecha aquí solo mediría la coincidencia de dos modelos
//! distintos, no la corrección de este.

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{projection_cases_5_0, projection_cases_all, ProjCase};
use futurefin_engine::{
    project_net_worth_series, AllocationKind, AllocationRule, FireNeed, FireTarget, PhasePlan,
    ProjectionInput, SimAsset, SpendMode, WithdrawalRule,
};
use futurefin_engine_stochastic::{
    project_percentile_bands, run_path, seed_for, simulate_f64, CashBufferSpec, McConfig,
    McOutcome,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

// =================================================================================================
// Utilidades compartidas
// =================================================================================================

fn all_cases() -> Vec<ProjCase> {
    let mut out = projection_cases_all();
    out.extend(projection_cases_5_0());
    out
}

fn case(name: &str) -> ProjectionInput {
    all_cases()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("{name} debe existir en la batería del motor"))
        .input
}

/// Volatilidades para P9, una por activo y con las magnitudes que la ayuda de la SPA sugiere
/// (RV global ~15-18 %, RF ~4-6 %, efectivo 0):
/// cuenta corriente 0 · bonos 5 % · RV 16 % · vivienda 8 % · cripto 70 %.
fn p9_volatilities() -> Vec<Option<f64>> {
    vec![None, Some(5.0), Some(16.0), Some(8.0), Some(70.0)]
}

/// Un hogar de UN activo, ya jubilado desde el primer mes, que gasta `monthly` euros constantes.
/// Es el laboratorio de la tabla del issue: sin impuestos, sin inflación, sin deuda y sin cascada,
/// para que lo único que decida el resultado sea la secuencia de retornos.
///
/// **La inflación va a 0 a propósito**: con IPC nulo, «gasto fijo real» y «gasto fijo nominal» son
/// lo mismo, y los `6,5 % / 17 %` del issue se leen como parámetros REALES — que es como se leen
/// en la literatura de la que sale esa tabla.
fn single_asset_retiree(
    capital: Decimal,
    monthly_expense: Decimal,
    annual_return: Decimal,
    horizon: u32,
) -> ProjectionInput {
    ProjectionInput {
        ref_date: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        horizon_months: horizon,
        annual_inflation_percent: Decimal::ZERO,
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        income_regular_monthly: Decimal::ZERO,
        expense_regular_monthly: monthly_expense,
        assets: vec![SimAsset {
            id: uuid::Uuid::from_u128(1),
            value: capital,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: Some(annual_return),
        }],
        allocation_rules: Vec::new(),
        liabilities: Vec::new(),
        planning_monthly_cash_adjustment: vec![Decimal::ZERO; horizon as usize],
        // Jubilado desde el mes 1: sin ingreso, con el gasto declarado. Sin `fire_target`, el
        // cruce no existe y el único trigger es el mes forzado.
        phase_plan: PhasePlan::forced_at(1, Decimal::ZERO, monthly_expense, Decimal::ZERO),
        fire_target: None,
    }
}

/// Un hogar que **todavía no se ha jubilado**: ahorra, invierte todo el sobrante y se jubilará
/// cuando el líquido cruce su número FIRE. Es el laboratorio de la definición de éxito (D22): sin
/// un trigger por cruce, «no jubilarse nunca» no es un suceso posible.
fn crossing_household(
    start: Decimal,
    income: Decimal,
    expense: Decimal,
    annual_return: Decimal,
    swr_pct: Decimal,
    horizon: u32,
) -> ProjectionInput {
    let mut input = single_asset_retiree(start, expense, annual_return, horizon);
    input.income_regular_monthly = income;
    input.allocation_rules = vec![AllocationRule {
        target_index: 0,
        kind: AllocationKind::Remainder,
        amount: None,
        cap: None,
    }];
    input.phase_plan = PhasePlan::classic(Decimal::ZERO, expense);
    input.fire_target = Some(FireTarget {
        need: FireNeed::ExpenseMinusPension {
            expense_monthly: expense,
            pension_monthly: Decimal::ZERO,
        },
        swr_pct,
        tax_brackets: Vec::new(),
        taxes_enabled: false,
        taxable_gain_ratio: Decimal::ONE,
        annual_inflation_percent: Decimal::ZERO,
        debt_payments_remaining: vec![Decimal::ZERO; horizon as usize + 1],
    });
    input
}

/// El mismo laboratorio con **dos** activos: una cuenta al 0 % (el colchón) y la renta variable.
///
/// «100 % renta variable **con manga de caja**» es lo que una estrategia de colchón significa: el
/// dinero no invertido ES el colchón. Los dos escenarios que se comparan (con y sin colchón)
/// arrancan de la MISMA cartera, así que lo único que cambia entre ellos es si la cuenta se
/// vuelve a llenar en los meses buenos o se gasta una vez y ya.
fn buffered_retiree(
    cash: Decimal,
    equity: Decimal,
    monthly_expense: Decimal,
    annual_return: Decimal,
    horizon: u32,
) -> ProjectionInput {
    buffered_retiree_at(
        cash,
        equity,
        monthly_expense,
        annual_return,
        Decimal::ZERO,
        horizon,
    )
}

/// El mismo laboratorio con la rentabilidad del COLCHÓN como eje propio.
///
/// Sin este eje no se puede separar lo que el colchón hace de lo que cuesta tenerlo: una cuenta
/// al 0 % arrastra ~5.200 €/año sobre 80.000 €, y ese lastre se confundía con «el colchón no
/// protege». Poniendo el colchón a la misma rentabilidad esperada que la RV —σ = 0, misma media—
/// el lastre desaparece y queda solo el efecto de la POLÍTICA.
fn buffered_retiree_at(
    cash: Decimal,
    equity: Decimal,
    monthly_expense: Decimal,
    annual_return: Decimal,
    cash_return: Decimal,
    horizon: u32,
) -> ProjectionInput {
    let mut input = single_asset_retiree(cash + equity, monthly_expense, annual_return, horizon);
    input.assets = vec![
        SimAsset {
            id: uuid::Uuid::from_u128(1),
            value: cash,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: Some(cash_return),
        },
        SimAsset {
            id: uuid::Uuid::from_u128(2),
            value: equity,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: Some(annual_return),
        },
    ];
    input
}

// =================================================================================================
// 1. Reproducibilidad
// =================================================================================================

/// **Misma semilla ⇒ mismo resultado, bit a bit.** El `assert_eq!` es sobre el [`McOutcome`]
/// ENTERO: bandas, probabilidades, percentiles del mes de jubilación y contadores. Si algo del
/// camino dependiera del orden de iteración de un mapa, del reloj o de una dirección de memoria,
/// fallaría aquí.
#[test]
fn mc_same_seed_bit_identical() {
    let input = case("P9_hogar_realista");
    let vols = p9_volatilities();
    let config = McConfig {
        seed: 20_260_903,
        paths: 64,
        ..Default::default()
    };
    let a = project_percentile_bands(&input, &vols, &config).expect("la ejecución no falla");
    let b = project_percentile_bands(&input, &vols, &config).expect("la ejecución no falla");
    assert_eq!(
        a, b,
        "dos ejecuciones con la misma semilla no son idénticas"
    );

    // Y camino a camino: el camino 7 es el camino 7 se pida solo o dentro de 64.
    let solo = run_path(&input, &vols, &config, 7).expect("un camino suelto no falla");
    let again = run_path(&input, &vols, &config, 7).expect("un camino suelto no falla");
    assert_eq!(solo.net_worth, again.net_worth);
    assert_eq!(solo.liquid_worth, again.liquid_worth);
    assert_eq!(
        solo.assets_depleted_month_index,
        again.assets_depleted_month_index
    );

    // Ampliar la muestra NO reescribe la muestra: el camino 7 de una ejecución de 64 y el de una
    // de 500 son el mismo (flujo propio por camino).
    let wider = McConfig {
        paths: 500,
        ..config.clone()
    };
    let in_500 = run_path(&input, &vols, &wider, 7).expect("un camino suelto no falla");
    assert_eq!(solo.net_worth, in_500.net_worth);
}

/// **Semillas distintas, mercados distintos.** Sin esto, «reproducible» podría significar
/// «constante», que es otra cosa.
#[test]
fn mc_different_seed_differs() {
    let input = case("P9_hogar_realista");
    let vols = p9_volatilities();
    let base = McConfig {
        seed: 1,
        paths: 48,
        ..Default::default()
    };
    let other = McConfig {
        seed: 2,
        ..base.clone()
    };
    let a = project_percentile_bands(&input, &vols, &base).expect("no falla");
    let b = project_percentile_bands(&input, &vols, &other).expect("no falla");
    assert_ne!(a.net_worth, b.net_worth, "dos semillas dan la misma banda");
    assert_ne!(a.liquid_worth, b.liquid_worth);
}

// =================================================================================================
// 2. Degeneración con volatilidad cero
// =================================================================================================

/// La cota de contrato de WP5.5: 1 € por mes. Copiada de `degeneration.rs` a propósito — es la
/// MISMA cota y este test es su continuación con el sorteo en medio.
const EUR_TOLERANCE: f64 = 1.0;
/// Cota relativa para los casos cuyas magnitudes superan `2^53 €`, donde el propio espaciado de
/// los `f64` ya es mayor que un euro.
const REL_TOLERANCE: f64 = 1e-12;
/// `2^53`: por encima, un `f64` no distingue euros enteros.
const EXACT_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

/// **Volatilidad cero degenera en el camino determinista.**
///
/// Es LA puerta del modelo: con `σ = None` en todos los activos, el factor del mes es `m_i`
/// exactamente —rama explícita, no `exp(0)`— y por tanto los `paths` caminos son el mismo camino.
/// Consecuencias que se comprueban:
///
/// - cada banda, en cada percentil y cada mes, es **bit a bit** la serie de `simulate_f64`;
/// - `success_probability` es **exactamente** `0.0` o `1.0` (nunca un `0,9999…`), y coincide con
///   lo que el camino determinista dice sobre el agotamiento;
/// - contra el motor `Decimal`, las bandas caen dentro de las cotas de WP5.5.
///
/// Si esto fallara, «la banda p50 con σ=0» dejaría de ser «la línea que la app pinta» y las dos
/// curvas del chart de Riesgo contarían historias distintas sin que nada avisara.
#[test]
fn mc_zero_volatility_degenerates_to_deterministic() {
    let config = McConfig {
        seed: 0xDEAD_BEEF,
        paths: 8,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };

    let mut checked = 0usize;
    println!(
        "\n{:<32} {:>5} {:>12} {:>12} {:>14} {:>8}",
        "caso", "meses", "max|Δ| NW", "max|Δ| LIQ", "regla", "éxito"
    );
    for c in all_cases() {
        let Ok(det) = simulate_f64(&c.input) else {
            continue;
        };
        let vols: Vec<Option<f64>> = vec![None; c.input.assets.len()];
        let out = project_percentile_bands(&c.input, &vols, &config)
            .unwrap_or_else(|e| panic!("{}: Monte Carlo falló ({e})", c.name));

        assert!(
            !out.any_volatility_declared,
            "{}: sin volatilidad declarada, `any_volatility_declared` debe ser false",
            c.name
        );
        assert!(
            !out.buffer_active,
            "{}: el colchón sigue sin simularse",
            c.name
        );

        // (a) Bit a bit contra el camino determinista en `f64`.
        for (j, p) in out.percentiles.iter().enumerate() {
            let band_nw: Vec<f64> = det.net_worth.iter().map(|v| v.0).collect();
            let band_lq: Vec<f64> = det.liquid_worth.iter().map(|v| v.0).collect();
            assert_eq!(
                out.net_worth[j], band_nw,
                "{}: la banda p{p} de net_worth no es la serie determinista",
                c.name
            );
            assert_eq!(
                out.liquid_worth[j], band_lq,
                "{}: la banda p{p} de liquid_worth no es la serie determinista",
                c.name
            );
        }

        // (b) La probabilidad de éxito es EXACTAMENTE 0 o 1, y es la del camino determinista.
        //
        // Desde el pase de correcciones de la revisión adversarial, «éxito» exige que el plan
        // OCURRA: el hogar se jubila dentro del horizonte (o el trigger es por edad, y entonces
        // la jubilación es un dato) y además no agota la cartera. Con σ=0 todos los caminos son
        // el determinista, así que la expectativa se lee de él en los dos términos.
        let age_triggered = c
            .input
            .phase_plan
            .retirement_trigger
            .forced_month()
            .is_some();
        let expected = if (age_triggered || det.retirement_month_index.is_some())
            && det.assets_depleted_month_index.is_none()
        {
            1.0
        } else {
            0.0
        };
        assert_eq!(
            out.never_retired_probability,
            if age_triggered || det.retirement_month_index.is_some() {
                0.0
            } else {
                1.0
            },
            "{}: con σ=0 «no se jubila nunca» tampoco admite matices",
            c.name
        );
        assert_eq!(
            out.success_probability, expected,
            "{}: con σ=0 el éxito no admite matices",
            c.name
        );

        // (c) Contra el motor exacto, con las cotas de WP5.5.
        let dec = project_net_worth_series(&c.input).expect("el camino Decimal no falla");
        let (nw_max, nw_rel, nw_mag) = worst(&dec.net_worth, &out.net_worth[1]);
        let (lq_max, lq_rel, lq_mag) = worst(&dec.liquid_worth, &out.liquid_worth[1]);
        let rule = if nw_mag.max(lq_mag) <= EXACT_INTEGER_LIMIT {
            assert!(
                nw_max <= EUR_TOLERANCE && lq_max <= EUR_TOLERANCE,
                "{}: la banda p50 se desvía {nw_max:.6} € / {lq_max:.6} € del motor exacto",
                c.name
            );
            "≤ 1 €"
        } else {
            assert!(
                nw_rel <= REL_TOLERANCE && lq_rel <= REL_TOLERANCE,
                "{}: la banda p50 se desvía {nw_rel:.3e} / {lq_rel:.3e} relativo",
                c.name
            );
            "relativa 1e-12"
        };
        println!(
            "{:<32} {:>5} {:>12.3e} {:>12.3e} {:>14} {:>8.1}",
            c.name, c.input.horizon_months, nw_max, lq_max, rule, out.success_probability
        );
        checked += 1;
    }
    assert!(
        checked >= 23,
        "la batería del motor no puede encogerse sin que este test lo diga: {checked} casos"
    );
}

/// `(max |Δ|, relativa en ese mes, magnitud máxima)` entre una serie `Decimal` y una `f64`.
fn worst(dec: &[Decimal], flo: &[f64]) -> (f64, f64, f64) {
    assert_eq!(dec.len(), flo.len());
    let (mut max_abs, mut rel, mut mag) = (0.0f64, 0.0f64, 0.0f64);
    for (d, x) in dec.iter().zip(flo.iter()) {
        let dv = d.to_f64().expect("cabe en coma flotante");
        mag = mag.max(dv.abs());
        let diff = (x - dv).abs();
        if diff > max_abs {
            max_abs = diff;
            rel = if dv == 0.0 { 0.0 } else { diff / dv.abs() };
        }
    }
    (max_abs, rel, mag)
}

// =================================================================================================
// 3. Orden de las bandas
// =================================================================================================

/// **p10 ≤ p50 ≤ p90, en todos los meses y en las dos series.**
///
/// Es una propiedad del rango más cercano sobre una muestra ordenada —el índice es monótono en
/// `p`—, y por eso se comprueba con una batería de percentiles más ancha que la que la UI dibuja:
/// lo que se está pineando es que el cálculo del percentil no se salte esa monotonía por un
/// redondeo o por ordenar dos series con criterios distintos.
#[test]
fn mc_bands_are_ordered() {
    let input = case("P9_hogar_realista");
    let vols = p9_volatilities();
    let config = McConfig {
        seed: 99,
        paths: 200,
        percentiles: vec![1, 5, 10, 25, 50, 75, 90, 95, 99],
        cash_buffer: None,
    };
    let out = project_percentile_bands(&input, &vols, &config).expect("no falla");
    assert!(out.any_volatility_declared);

    for (label, bands) in [
        ("net_worth", &out.net_worth),
        ("liquid_worth", &out.liquid_worth),
    ] {
        for k in 0..=(input.horizon_months as usize) {
            for j in 1..bands.len() {
                assert!(
                    bands[j][k] >= bands[j - 1][k],
                    "{label}: en el mes {k}, p{} ({}) < p{} ({})",
                    out.percentiles[j],
                    bands[j][k],
                    out.percentiles[j - 1],
                    bands[j - 1][k]
                );
            }
        }
    }

    // La dispersión existe de verdad: en el último mes, p90 debe estar por encima de p10.
    let last = input.horizon_months as usize;
    let p10 = out.net_worth[2][last];
    let p90 = out.net_worth[6][last];
    println!(
        "[bandas] P9 a {} meses · p10 = {p10:.0} €  p90 = {p90:.0} €  (anchura {:.0} €)",
        input.horizon_months,
        p90 - p10
    );
    assert!(p90 > p10, "la banda es una línea con volatilidad declarada");
}

// =================================================================================================
// 4. El modelo hace lo que dice: E[factor] = m
// =================================================================================================

/// **La media del terminal es el terminal determinista.**
///
/// Un solo activo, sin gasto ni ingreso ni impuestos: el patrimonio terminal de un camino es
///
/// ```text
///   V_H = V_0 · Π_k m·exp(σ z_k − σ²/2) = D · exp(σ·S − H·σ²/2),   S = Σ z_k ~ N(0, H)
/// ```
///
/// con `D = V_0·m^H` el terminal determinista. `E[V_H] = D` **exactamente**, y
/// `Var(V_H) = D²·(exp(H σ²) − 1)`.
///
/// # La tolerancia, derivada y no elegida
///
/// Con `r = 7 %`, `σ_a = 15 %`, `H = 120` meses y `N = 2 000` caminos:
///
/// ```text
///   σ_m² = (0,15/√12)² = 1,875e-3      H·σ_m² = 0,225
///   sd relativa de V_H         = √(e^0,225 − 1)      = 0,5023
///   sd relativa de la MEDIA    = 0,5023/√2000        = 0,01123   (1,12 %)
/// ```
///
/// La cota exigida es **5 %**, es decir ≈ 4,45 desviaciones típicas de la media muestral. No es
/// un margen de seguridad arbitrario: por debajo de ~4 σ un test así falla de vez en cuando por
/// azar aunque el modelo sea correcto, y un test que falla al azar es un test que se acaba
/// ignorando. La semilla es fija, así que el valor observado es DETERMINISTA — se imprime.
#[test]
fn mc_mean_growth_matches_expected() {
    let capital = Decimal::from(100_000);
    let horizon = 120u32;
    let input = single_asset_retiree(capital, Decimal::ZERO, Decimal::from(7), horizon);
    let vols = vec![Some(15.0)];
    let paths = 2_000u32;
    let config = McConfig {
        seed: 4_242,
        paths,
        ..Default::default()
    };

    let deterministic = simulate_f64(&input).expect("no falla");
    let d_terminal = deterministic.net_worth[horizon as usize].0;

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    for p in 0..paths {
        let out = run_path(&input, &vols, &config, p).expect("ningún camino falla");
        let v = out.net_worth[horizon as usize].0;
        sum += v;
        sum_sq += v * v;
    }
    let mean = sum / f64::from(paths);
    let var = (sum_sq - f64::from(paths) * mean * mean) / (f64::from(paths) - 1.0);
    let rel_error = (mean - d_terminal) / d_terminal;

    // Predicciones cerradas del modelo, para contrastarlas con lo medido.
    let sigma_m2 = (0.15f64 / 12f64.sqrt()).powi(2);
    let predicted_rel_sd = ((f64::from(horizon) * sigma_m2).exp() - 1.0).sqrt();
    let predicted_mean_sd = predicted_rel_sd / f64::from(paths).sqrt();

    println!(
        "\n[media] 1 activo · 100.000 € · 7 % · σ 15 % · {horizon} meses · {paths} caminos\n\
         [media]   terminal determinista D = {d_terminal:.2} €\n\
         [media]   media muestral        = {mean:.2} €   (error relativo {:+.4} %)\n\
         [media]   sd relativa del camino: predicha {predicted_rel_sd:.4}, observada {:.4}\n\
         [media]   sd relativa de la media: predicha {predicted_mean_sd:.4} ⇒ cota 5 % ≈ {:.1} σ",
        rel_error * 100.0,
        var.sqrt() / d_terminal,
        0.05 / predicted_mean_sd
    );

    assert!(
        rel_error.abs() < 0.05,
        "la media muestral se desvía {:.3} % del terminal determinista: E[factor] ≠ m",
        rel_error * 100.0
    );
    // La dispersión también debe ser la del modelo (± 20 % relativo sobre la sd, que con 2 000
    // caminos tiene su propio error de ~1/√(2N) = 1,6 % más la asimetría log-normal).
    let observed_rel_sd = var.sqrt() / d_terminal;
    assert!(
        (observed_rel_sd / predicted_rel_sd - 1.0).abs() < 0.2,
        "la dispersión observada ({observed_rel_sd:.4}) no es la del modelo ({predicted_rel_sd:.4})"
    );
}

// =================================================================================================
// 5. La tabla del issue #207
// =================================================================================================

/// Ejecuta el laboratorio del issue con una tasa de retirada dada y devuelve la probabilidad de
/// RUINA (agotamiento antes del horizonte).
fn ruin_probability(withdrawal_pct: f64, paths: u32) -> (f64, McOutcome) {
    let capital = Decimal::from(1_000_000);
    // `withdrawal_pct` % del capital inicial, repartido en 12 mensualidades, fijo en términos
    // reales (IPC = 0 en este laboratorio).
    let monthly = Decimal::from(1_000_000) * Decimal::try_from(withdrawal_pct).unwrap()
        / Decimal::from(100)
        / Decimal::from(12);
    let input = single_asset_retiree(capital, monthly, Decimal::try_from(6.5).unwrap(), 420);
    let config = McConfig {
        seed: 207,
        paths,
        // Un solo percentil: lo que se mide es una probabilidad, no una banda, y ordenar tres
        // veces 421 vectores de 2.000 no aporta nada.
        percentiles: vec![50],
        cash_buffer: None,
    };
    let out = project_percentile_bands(&input, &[Some(17.0)], &config).expect("no falla");
    (1.0 - out.success_probability, out)
}

/// **La tabla del issue #207, reproducida dentro de la app.**
///
/// El issue trae, calculados FUERA de FutureFin: con 6,5 % de media y 17 % de desviación típica,
/// 35 años y una retirada fija en términos reales, ~7-10 % de ruina al 3 % del capital inicial y
/// ~18-23 % al 4 %.
///
/// # Por qué las horquillas son anchas
///
/// Aquellos números salen de otro modelo. Las diferencias, todas conocidas y ninguna un error:
///
/// - **Log-normal mensual vs normal anual.** Aquí el shock es log-normal y compone 12 veces al
///   año; la cola izquierda de una log-normal es más benigna que la de una normal (que admite
///   retornos por debajo de −100 %). Esto EMPUJA LA RUINA A LA BAJA respecto al modelo del issue.
/// - **Retirada mensual vs anual.** Retirar 1/12 cada mes en vez del año entero por adelantado
///   deja más capital invertido: otro empujón a la baja.
/// - **`6,5 %` como media ARITMÉTICA.** Este modelo la respeta exactamente (`E[factor] = m`); si
///   el del issue la hubiera tomado como geométrica, su cartera sería ~1,4 pp/año peor.
///
/// Por eso se exige **3-15 %** y **12-30 %** en vez de las horquillas del issue: lo que este test
/// prueba es que el orden de magnitud y —sobre todo— la RELACIÓN entre el 3 % y el 4 % son las
/// que la literatura describe. Los valores medidos se imprimen para que la comparación la haga
/// quien lea la salida, no el `assert`.
#[test]
fn mc_success_probability_of_the_issue_table() {
    let paths = 1_000u32;
    let (ruin3, out3) = ruin_probability(3.0, paths);
    let (ruin4, out4) = ruin_probability(4.0, paths);

    println!(
        "\n[issue #207] 1.000.000 € · 6,5 % media · 17 % sd · 35 años · {paths} caminos\n\
         [issue #207]   retirada 3 % ({:>7.2} €/mes): ruina = {:>6.2} %   (issue: 7-10 %, exigido 3-15 %)\n\
         [issue #207]   retirada 4 % ({:>7.2} €/mes): ruina = {:>6.2} %   (issue: 18-23 %, exigido 12-30 %)\n\
         [issue #207]   éxito 3 % = {:.3}   éxito 4 % = {:.3}\n\
         [issue #207]   meses con recorte (p50): {} / {}   ratio retirada:necesidad (p50): {:?} / {:?}",
        1_000_000.0 * 0.03 / 12.0,
        ruin3 * 100.0,
        1_000_000.0 * 0.04 / 12.0,
        ruin4 * 100.0,
        out3.success_probability,
        out4.success_probability,
        out3.months_below_need_p50,
        out4.months_below_need_p50,
        out3.withdrawal_to_need_ratio_p50,
        out4.withdrawal_to_need_ratio_p50,
    );

    assert!(
        (0.03..=0.15).contains(&ruin3),
        "ruina al 3 % = {:.2} %, fuera de 3-15 %",
        ruin3 * 100.0
    );
    assert!(
        (0.12..=0.30).contains(&ruin4),
        "ruina al 4 % = {:.2} %, fuera de 12-30 %",
        ruin4 * 100.0
    );
    assert!(
        ruin4 > ruin3,
        "retirar más no puede arruinar menos: {ruin4} ≤ {ruin3}"
    );

    // Con `fixed_real` la regla NO recorta nunca (`withdrawal_shortfall ≡ 0`, la separación de
    // D22/D24), pero la NECESIDAD NO CUBIERTA sí existe cuando la cartera se acaba, y desde el
    // pase de correcciones cuenta: `months_below_need_p50` mide meses con `recorte + descubierto
    // > 0`. El camino mediano al 3 % no se arruina y no tiene ninguno; al 4 % la mediana tampoco
    // (la ruina está en el 12-30 %), así que ambos siguen en 0 — lo que cambia es que ahora
    // cuentan por la razón correcta, y el caso que lo demuestra es
    // `mc_coverage_counts_the_need_the_portfolio_could_not_fund`.
    assert_eq!(out3.months_below_need_p50, 0);
    assert_eq!(out4.months_below_need_p50, 0);

    // La tabla de agotamiento por edad arranca en la jubilación (mes 1) y avanza de 5 en 5 años.
    assert_eq!(out4.depletion_probability_by_age[0].0, 1);
    assert_eq!(out4.depletion_probability_by_age[1].0, 61);
    let cumulative: Vec<f64> = out4
        .depletion_probability_by_age
        .iter()
        .map(|(_, p)| *p)
        .collect();
    println!("[issue #207]   agotamiento acumulado cada 5 años (4 %): {cumulative:?}");
    for w in cumulative.windows(2) {
        assert!(w[1] >= w[0], "una probabilidad ACUMULADA no puede bajar");
    }
    // **La última fila ES el horizonte** (corrección de la revisión adversarial): la rejilla
    // avanza de 60 en 60 desde la jubilación y antes se detenía en el último múltiplo que cabía
    // —el mes 361 de 420—, dejando fuera sin avisar a los caminos que se agotaban en los últimos
    // cinco años. Ahora cierra en el mes 420 y esa fila ES la ruina total.
    let last_row = *cumulative.last().expect("hay filas");
    assert_eq!(
        out4.depletion_probability_by_age
            .last()
            .expect("hay filas")
            .0,
        420,
        "la última fila de la tabla es el HORIZONTE, no el último múltiplo de 60"
    );
    assert!(
        (last_row - ruin4).abs() < 1e-12,
        "la última fila ({last_row}) es la ruina total ({ruin4})"
    );
}

/// **`percent_of_balance` no puede arruinar a nadie, y eso es una propiedad del modelo, no una
/// medición.**
///
/// Con la regla como GASTO (`rule_is_spend`), la retirada del mes es `pct/100 · líquido(k−1)/12`,
/// que es una FRACCIÓN de la cartera: mientras quede algo, se retira menos que todo. La cartera
/// se hace pequeña, pero no se agota — por eso `success_probability` debe ser exactamente `1.0`
/// con la misma volatilidad del 17 % que arruina al 20 % de los caminos con `fixed_real`.
///
/// Lo que esa regla sí hace es **recortar el gasto**, y eso se ve en la otra dimensión (D24):
/// meses por debajo de la necesidad y ratio retirada:necesidad. Un plan «sin riesgo de ruina»
/// que en realidad vive con la mitad del presupuesto no es un plan que salga bien, y ese es
/// exactamente el matiz que la app tiene que enseñar en vez de un semáforo verde.
#[test]
fn mc_percent_of_balance_never_ruins_but_cuts_the_spending() {
    let capital = Decimal::from(1_000_000);
    let monthly =
        Decimal::from(1_000_000) / Decimal::from(100) * Decimal::from(4) / Decimal::from(12);
    let mut input = single_asset_retiree(capital, monthly, Decimal::try_from(6.5).unwrap(), 420);
    input.phase_plan.withdrawal = WithdrawalRule::PercentOfBalance {
        pct: Decimal::from(4),
    };
    input.phase_plan.spend_mode = SpendMode::RuleIsSpend;

    let config = McConfig {
        seed: 207,
        paths: 1_000,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };
    let out = project_percentile_bands(&input, &[Some(17.0)], &config).expect("no falla");

    println!(
        "\n[percent_of_balance] 4 % del saldo, regla = gasto · 35 años · 1.000 caminos\n\
         [percent_of_balance]   éxito = {} (agotamientos = {})\n\
         [percent_of_balance]   meses con recorte (p50) = {} de 420\n\
         [percent_of_balance]   ratio retirada:necesidad (p50) = {:?}\n\
         [percent_of_balance]   líquido final p10/p50/p90 = {:.0} / {:.0} / {:.0} €",
        out.success_probability,
        ((1.0 - out.success_probability) * 1000.0).round() as u32,
        out.months_below_need_p50,
        out.withdrawal_to_need_ratio_p50,
        out.liquid_worth[0][420],
        out.liquid_worth[1][420],
        out.liquid_worth[2][420],
    );

    assert_eq!(
        out.success_probability, 1.0,
        "la regla porcentual retira una FRACCIÓN del saldo: agotarlo es imposible por construcción"
    );
    // Y sin embargo hay recorte: la mediana de los caminos pasa meses por debajo de la necesidad.
    assert!(
        out.months_below_need_p50 > 0,
        "sin recorte, esta regla sería gratis — y no lo es"
    );
    let ratio = out
        .withdrawal_to_need_ratio_p50
        .expect("hay meses jubilados con necesidad");
    assert!(
        (0.0..=1.0).contains(&ratio),
        "el ratio retirada:necesidad vive en [0,1]: {ratio}"
    );
}

// =================================================================================================
// 6. Semilla estable (D23) y colchón declarado (P4)
// =================================================================================================

/// **La semilla de un usuario no cambia nunca.** Dos pares de identificadores pineados: si el
/// hash cambiara, todas las bandas de todos los usuarios cambiarían a la vez y nadie sabría por
/// qué. Es el mismo tipo de pin que `the_chacha_stream_is_pinned`, un nivel más arriba.
#[test]
fn mc_seed_for_is_stable() {
    let a = seed_for(
        0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210_fedc_ba98_7654_3210,
    );
    let b = seed_for(1, 2);
    println!("[seed_for] pin A = {a:#018x}   pin B = {b:#018x}");
    assert_eq!(
        a, 0x4001_837e_2537_07e6,
        "la semilla del par A se ha movido"
    );
    assert_eq!(
        b, 0x4390_6262_9bbe_2641,
        "la semilla del par B se ha movido"
    );
    // Determinista y puro: mil llamadas, un solo valor.
    for _ in 0..1_000 {
        assert_eq!(seed_for(1, 2), b);
    }
}

/// **Las dos maneras de que el colchón no haga nada, y son distintas.**
///
/// 1. **Sin volatilidad declarada NO se instala** (`no_volatility`): `z_k` se sigue sorteando
///    —el flujo del RNG no depende de los datos— pero no mueve ningún retorno, así que rellenar
///    «tras un mes bueno» sería trasvasar valor y pagar plusvalías guiándose por un shock que no
///    afecta a nada. Resultado: `buffer_active: false`, las dos lecturas a `None` («no se midió»,
///    que no es «cero rellenos») y un [`McOutcome`] idéntico al de no pedirlo salvo el motivo.
/// 2. **Sin un activo líquido SIN RIESGO tampoco** (`no_safe_liquid_asset`), y esto es la
///    corrección de la revisión adversarial: el índice del colchón salía de `cash_buffer_index`,
///    que se deriva del orden de drenaje y no sabe de volatilidad. En una cartera de un solo
///    fondo con σ = 12 % elegía **ese fondo** como colchón — un colchón con la volatilidad de la
///    cartera no es un colchón, es la misma cartera con más impuestos.
/// 3. **Con un activo líquido a σ = 0 sí se instala**, y entonces las lecturas existen.
#[test]
fn mc_cash_buffer_is_installed_only_when_it_can_mean_something() {
    let input = case("P7_jubilado_pension_impuestos");
    let without = McConfig {
        seed: 5,
        paths: 32,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };
    let with = McConfig {
        cash_buffer: Some(CashBufferSpec::Months(24)),
        ..without.clone()
    };

    // (1) σ = 0 en toda la cartera: no hay riesgo del que protegerse.
    let flat: Vec<Option<f64>> = input.assets.iter().map(|_| None).collect();
    let a = project_percentile_bands(&input, &flat, &without).expect("no falla");
    let b = project_percentile_bands(&input, &flat, &with).expect("no falla");
    assert!(!a.buffer_active && !b.buffer_active);
    assert_eq!(
        a.buffer_inactive_reason.map(|r| r.code()),
        Some("not_requested")
    );
    assert_eq!(
        b.buffer_inactive_reason.map(|r| r.code()),
        Some("no_volatility")
    );
    assert_eq!(b.buffer_refills_p50, None);
    assert_eq!(b.buffer_refill_net_total_p50, None);
    assert_eq!(
        a,
        McOutcome {
            buffer_inactive_reason: a.buffer_inactive_reason,
            ..b.clone()
        },
        "con σ=0 el colchón no se instala: pedirlo no puede mover un dígito (salvo el motivo)"
    );

    // (2) σ = 12 % en TODO: hay riesgo, pero no hay dónde alojar el colchón. Antes se instalaba
    //     sobre el propio fondo volátil.
    let vols: Vec<Option<f64>> = input.assets.iter().map(|_| Some(12.0)).collect();
    let live_off = project_percentile_bands(&input, &vols, &without).expect("no falla");
    let live_on = project_percentile_bands(&input, &vols, &with).expect("no falla");
    assert!(
        !live_on.buffer_active,
        "un fondo con σ = 12 % no puede ser su propio colchón"
    );
    assert_eq!(
        live_on.buffer_inactive_reason.map(|r| r.code()),
        Some("no_safe_liquid_asset")
    );
    assert_eq!(live_on.buffer_refills_p50, None);
    assert_eq!(
        live_off.net_worth, live_on.net_worth,
        "un colchón que no se instala no puede mover una sola serie"
    );
    assert_eq!(live_off.liquid_worth, live_on.liquid_worth);

    // (3) Añadiendo una cuenta LÍQUIDA a σ = 0, el colchón ya tiene casa.
    let monthly =
        Decimal::from(1_000_000) * Decimal::from(4) / Decimal::from(100) / Decimal::from(12);
    let two = buffered_retiree(
        Decimal::from(80_000),
        Decimal::from(920_000),
        monthly,
        Decimal::try_from(6.5).unwrap(),
        420,
    );
    let installed = project_percentile_bands(&two, &[None, Some(17.0)], &with).expect("no falla");
    println!(
        "[colchón] instalación · σ=0 ⇒ {:?} · σ=12 % en todo ⇒ {:?} · cuenta σ=0 + RV σ=17 % ⇒ activo={}",
        b.buffer_inactive_reason.map(|r| r.code()),
        live_on.buffer_inactive_reason.map(|r| r.code()),
        installed.buffer_active
    );
    assert!(installed.buffer_active);
    assert_eq!(installed.buffer_inactive_reason, None);
    assert!(installed.buffer_refills_p50.expect("se simuló") > 0);
}

/// **El colchón, descompuesto: cuánto cuesta tenerlo y cuánto protege.**
///
/// El laboratorio del issue con manga de caja: 1.000.000 € (80.000 en cuenta = 24 meses de gasto,
/// 920.000 en RV al 6,5 % con σ = 17 %), retirada fija real del 4 % del capital inicial, 35 años,
/// sin impuestos y sin IPC, semilla 207, 1.000 caminos.
///
/// # Por qué el test tiene DOS escenarios y no uno
///
/// La versión anterior medía un solo escenario —la cuenta al 0 %— y concluyó «el colchón empeora
/// el plan». La revisión adversarial (D20) mostró que esa conclusión mezclaba **tres** efectos
/// distintos, dos de ellos ajenos a la política de colchón:
///
/// 1. **Lastre de caja.** Mantener 80.000 € al 0 % en vez de al 6,5 % cuesta ~5.200 €/año de
///    crecimiento esperado. No es el colchón: es la cuenta.
/// 2. **Protección.** Gastar de una reserva sin riesgo evita vender RV justo después de una
///    caída. Es lo que el colchón dice hacer.
/// 3. **Anticipación.** El código autorizaba el relleno con el shock del PROPIO mes (`z_k`) y el
///    relleno se ejecuta ANTES del crecimiento: vendía RV al precio de antes de una subida que ya
///    sabía que venía. Eso no es un colchón, es una apuesta con información del futuro — y salía
///    cara. Corregido a `z_{k−1}` en el pase de correcciones.
///
/// # Medido, con el modelo corregido (relleno NO anticipativo)
///
/// ```text
///   colchón de 24 meses           éxito sin → con        Δ
///   cuenta al 0 %   (con lastre)  0,7750 → 0,7400     −3,50 pp
///   cuenta al 6,5 % (sin lastre)  0,7800 → 0,8190     +3,90 pp
/// ```
///
/// - **Descomposición**: lastre = 0,7400 − 0,8190 = **−7,90 pp**; protección = 0,8190 − 0,7800 =
///   **+3,90 pp**; y la anticipación que se retiró valía **+2,7 pp** (el mismo escenario al 0 %
///   daba 0,713 con `z_k` y da 0,740 con `z_{k−1}`).
/// - Con el lastre fuera, el colchón **mejora** el plan y sobre todo la cola: el líquido p10 del
///   mes 240 pasa de 99.409 € a 197.767 €, casi el doble.
/// - Con la cuenta al 0 % el colchón sigue costando, y eso es lo que la ayuda de la UI tiene que
///   decir: *la protección es real, pero no es gratis — la paga la rentabilidad que renuncias
///   por tener 24 meses de gasto fuera del mercado*.
///
/// La predicción escrita antes de ejecutar («con el colchón a la rentabilidad de la RV el éxito
/// SUBE; al 0 % puede seguir costando») se cumplió en los dos signos. El `assert` fija esos dos
/// signos, no un número ajustado.
#[test]
fn mc_cash_buffer_protects_and_the_drag_is_what_costs() {
    let monthly =
        Decimal::from(1_000_000) * Decimal::from(4) / Decimal::from(100) / Decimal::from(12);
    let vols = vec![None, Some(17.0)];
    let without = McConfig {
        seed: 207,
        paths: 1_000,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };
    let with = McConfig {
        cash_buffer: Some(CashBufferSpec::Months(24)),
        ..without.clone()
    };
    let run = |cash_return: Decimal| {
        let input = buffered_retiree_at(
            Decimal::from(80_000),
            Decimal::from(920_000),
            monthly,
            Decimal::try_from(6.5).unwrap(),
            cash_return,
            420,
        );
        let a = project_percentile_bands(&input, &vols, &without).expect("no falla");
        let b = project_percentile_bands(&input, &vols, &with).expect("no falla");
        (a, b)
    };
    let (flat_off, flat_on) = run(Decimal::ZERO);
    let (fair_off, fair_on) = run(Decimal::try_from(6.5).unwrap());
    assert!(!flat_off.buffer_active && flat_on.buffer_active);
    assert!(!fair_off.buffer_active && fair_on.buffer_active);

    let drag = flat_on.success_probability - fair_on.success_probability;
    let protection = fair_on.success_probability - fair_off.success_probability;
    println!(
        "\n[colchón] 1.000.000 € (80.000 cuenta + 920.000 RV 6,5 %/17 %) · 4 % real · 35 años · 1.000 caminos\n\
         [colchón]   cuenta al 0 %   : éxito {:.4} → {:.4}  (Δ {:+.4})   líquido p10 mes 240 {:>10.0} → {:>10.0} €\n\
         [colchón]   cuenta al 6,5 % : éxito {:.4} → {:.4}  (Δ {:+.4})   líquido p10 mes 240 {:>10.0} → {:>10.0} €\n\
         [colchón]   descomposición  : lastre {:+.4}   protección {:+.4}\n\
         [colchón]   rellenos p50 = {:?} de 420 · movido p50 = {:?} €",
        flat_off.success_probability,
        flat_on.success_probability,
        flat_on.success_probability - flat_off.success_probability,
        flat_off.liquid_worth[0][240],
        flat_on.liquid_worth[0][240],
        fair_off.success_probability,
        fair_on.success_probability,
        protection,
        fair_off.liquid_worth[0][240],
        fair_on.liquid_worth[0][240],
        drag,
        protection,
        fair_on.buffer_refills_p50,
        fair_on.buffer_refill_net_total_p50,
    );

    // (1) El colchón ACTÚA: se rellena, y no todos los meses (solo tras un shock positivo).
    let refills = fair_on.buffer_refills_p50.expect("se simuló");
    assert!(refills > 0 && refills < 420, "rellenos = {refills}");
    assert!(fair_on.buffer_refill_net_total_p50.expect("se simuló") > 0.0);
    assert_ne!(fair_off.liquid_worth, fair_on.liquid_worth);

    // (2) **Sin lastre, el colchón PROTEGE**: más éxito y, sobre todo, mucha más cola.
    assert!(
        protection > 0.0,
        "con el colchón a la rentabilidad de la cartera el éxito tiene que subir: {:.4} ≤ {:.4}",
        fair_on.success_probability,
        fair_off.success_probability
    );
    assert!(
        fair_on.liquid_worth[0][240] > fair_off.liquid_worth[0][240],
        "la protección se ve en la COLA (p10), que es donde vive la ruina"
    );

    // (3) **El lastre es lo que cuesta**, y cuesta más de lo que la protección aporta: por eso el
    //     escenario realista (cuenta al 0 %) sigue en negativo.
    assert!(
        drag < 0.0,
        "el lastre de caja no puede ser gratis: {drag:+.4}"
    );
    assert!(
        flat_on.success_probability < flat_off.success_probability,
        "con la cuenta al 0 % el colchón sigue costando en este modelo: {:.4} ≥ {:.4}",
        flat_on.success_probability,
        flat_off.success_probability
    );
    assert!(
        drag.abs() > protection,
        "y el lastre ({drag:+.4}) tiene que dominar a la protección ({protection:+.4}), que es lo \
         que explica el signo del escenario realista"
    );
}

/// **El colchón `Amount` mantiene el TOPE, en nominal, y no se indexa** (5.0.0, V6/P2).
///
/// Es la puerta de la variante que el colchón derivado del tope de una regla de ahorro necesita.
/// El tope `amount` de una regla es un importe **nominal fijo** que la cascada persigue sin
/// indexar nunca (`resolve_cap_ceiling_g`); el colchón en MESES, en cambio, se dimensiona contra
/// el gasto **ya indexado** del mes. Derivar «≈ 24 meses» de un tope de 48.000 € y dejar que se
/// indexe convertiría la regla del usuario en otra cosa: a 35 años con un 2,5 % el objetivo
/// acabaría en ~113.000 € nominales, **2,4× lo que escribió**.
///
/// Lo que se fija aquí, y por qué cada aserción:
///
/// 1. **Con `Amount(48 000)` el colchón nunca pasa del tope.** La cuenta no renta (0 %) y la
///    retirada sale de ella primero, así que el único mecanismo que la sube es el relleno — y el
///    relleno apunta a `max(0, tope − valor)`. Un techo que se respetara «casi» sería un techo
///    indexado.
/// 2. **Y lo alcanza**: si no llegara al tope, el techo no probaría nada (un colchón que no se
///    rellena también «no lo pasa»).
/// 3. **Con `Months(24)` el objetivo SÍ se indexa** — la variante histórica no cambia — y el
///    colchón supera el tope con holgura en la segunda mitad del horizonte.
/// 4. **Con inflación 0 las dos convenciones son la MISMA**, bit a bit: `Months(24)` sobre un
///    gasto de 2.000 € es exactamente `Amount(48 000)`. Es la prueba de que la variante nueva no
///    cambia la aritmética, solo la base contra la que se mide.
/// 5. **El relleno sigue siendo condicional**: se rellena tras un shock positivo, no todos los
///    meses. Sin la puerta, el colchón subiría en ~todos los meses posteriores a una retirada.
#[test]
fn mc_cash_buffer_amount_holds_the_cap() {
    let horizon = 420u32;
    let monthly = Decimal::from(2_000);
    // 24 meses del gasto del mes 0. Las dos configuraciones piden LO MISMO a mes 0 y divergen
    // solo por la indexación.
    let cap = Decimal::from(48_000);
    let vols = vec![None, Some(17.0)];
    let build = |inflation_pct: Decimal| {
        let mut input = buffered_retiree_at(
            Decimal::from(20_000),
            Decimal::from(980_000),
            monthly,
            Decimal::try_from(6.5).unwrap(),
            // La cuenta no renta: es el colchón, y es también su lastre.
            Decimal::ZERO,
            horizon,
        );
        input.annual_inflation_percent = inflation_pct;
        input
    };
    let base = McConfig {
        seed: 20_260_905,
        paths: 64,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };
    let amount_cfg = McConfig {
        cash_buffer: Some(CashBufferSpec::Amount(cap)),
        ..base.clone()
    };
    let months_cfg = McConfig {
        cash_buffer: Some(CashBufferSpec::Months(24)),
        ..base.clone()
    };

    let inflated = build(Decimal::try_from(2.5).unwrap());
    let cap_f = 48_000.0_f64;
    // El activo 0 ES el colchón: líquido, σ = 0 y el de menor rentabilidad, o sea el primero del
    // orden de drenaje (`safe_cash_buffer_index`).
    let buffer_series = |cfg: &McConfig| -> Vec<f64> {
        run_path(&inflated, &vols, cfg, 0).expect("un camino no falla").per_asset_series[0]
            .iter()
            .map(|v| v.0)
            .collect()
    };
    let by_amount = buffer_series(&amount_cfg);
    let by_months = buffer_series(&months_cfg);
    let peak = |v: &[f64]| v.iter().copied().fold(f64::MIN, f64::max);
    let tail_peak = |v: &[f64]| peak(&v[v.len() / 2..]);
    println!(
        "\n[colchón/tope] 2,5 % de inflación · 35 años · tope {cap_f:.0} €\n\
         [colchón/tope]   Amount : máximo {:>10.0} €   máximo en la 2.ª mitad {:>10.0} €\n\
         [colchón/tope]   Months : máximo {:>10.0} €   máximo en la 2.ª mitad {:>10.0} €",
        peak(&by_amount),
        tail_peak(&by_amount),
        peak(&by_months),
        tail_peak(&by_months),
    );

    // (1) El tope se respeta en TODO el horizonte: nominal, sin indexar.
    for (k, v) in by_amount.iter().enumerate() {
        assert!(
            *v <= cap_f + 1.0,
            "el colchón `Amount` no puede pasar del tope: mes {k} vale {v:.2} € > {cap_f:.0} €"
        );
    }
    // (2) …y se alcanza: el techo prueba algo porque el colchón llega a él.
    assert!(
        peak(&by_amount) >= cap_f - 1.0,
        "el colchón `Amount` tiene que llegar al tope: máximo {:.2} €",
        peak(&by_amount)
    );

    // (3) La variante en MESES sigue indexándose (no cambia con este WP): supera el tope con
    //     holgura en la segunda mitad del horizonte, que es donde la inflación ya pesa.
    assert!(
        tail_peak(&by_months) > cap_f * 1.4,
        "el colchón en meses se indexa con el gasto: máximo en la 2.ª mitad {:.2} € ≤ {:.0} €",
        tail_peak(&by_months),
        cap_f * 1.4
    );

    // (4) Sin inflación las dos convenciones son la MISMA plan: `Months(24)` × 2.000 € = 48.000 €.
    let flat = build(Decimal::ZERO);
    let flat_amount =
        project_percentile_bands(&flat, &vols, &amount_cfg).expect("no falla");
    let flat_months =
        project_percentile_bands(&flat, &vols, &months_cfg).expect("no falla");
    assert!(flat_amount.buffer_active && flat_months.buffer_active);
    assert_eq!(
        flat_amount, flat_months,
        "con inflación 0, `Amount(24 × gasto)` y `Months(24)` son el mismo plan"
    );

    // (5) El relleno sigue siendo condicional al shock positivo del mes anterior: si se rellenara
    //     siempre, el colchón subiría en casi todos los meses posteriores a una retirada.
    let refills = by_amount.windows(2).filter(|w| w[1] > w[0] + 1e-9).count();
    assert!(
        refills > horizon as usize / 8 && refills < horizon as usize * 3 / 4,
        "los rellenos se autorizan solo tras un mes al alza: {refills} de {horizon}"
    );
}

/// Las lecturas que dependen del TRIGGER: los percentiles del mes de jubilación existen solo si
/// jubila el cruce, y la probabilidad de infra-financiación solo si jubila la edad.
#[test]
fn mc_readings_follow_the_retirement_trigger() {
    let config = McConfig {
        seed: 11,
        paths: 40,
        ..Default::default()
    };

    // (a) P3 se jubila por CRUCE (190.000 € creciendo hacia un objetivo de 200.000): hay
    //     percentiles del mes, no hay infra-financiación. Con volatilidad el mes de cruce se
    //     DISPERSA, que es justo la lectura que las estrategias por cruce necesitan.
    let crossing = case("P3_superavit_jubilacion");
    let out = project_percentile_bands(&crossing, &[Some(18.0)], &config).expect("no falla");
    let months = out
        .retirement_month_index_percentiles
        .as_ref()
        .expect("con trigger por cruce, los percentiles del mes existen");
    assert_eq!(months.len(), out.percentiles.len());
    // Ordenados: un percentil mayor no puede jubilarse ANTES. `None` («nunca») ordena el último.
    for w in months.windows(2) {
        match (w[0], w[1]) {
            (Some(a), Some(b)) => assert!(a <= b, "p{a} > p{b}: los meses no están ordenados"),
            (None, Some(_)) => panic!("«nunca» debe ordenar después de cualquier mes"),
            _ => {}
        }
    }
    let deterministic_month = simulate_f64(&crossing)
        .expect("no falla")
        .retirement_month_index;
    println!(
        "[trigger] P3 (cruce) · mes de jubilación p10/p50/p90 = {months:?}           (determinista: {deterministic_month:?})"
    );
    assert!(
        months[0] != months[2],
        "con 18 % de volatilidad el mes de cruce no puede salir constante: {months:?}"
    );
    assert!(out.underfunded_probability.is_none());

    // (b) P21 se jubila por EDAD con el objetivo vivo como LECTURA (D17): hay probabilidad de
    //     infra-financiación, no hay percentiles del mes. En el camino determinista el capital NO
    //     llega al objetivo del mes 120 (el caso pinea `RetireAtAgeUnderfunded`), así que la
    //     probabilidad tiene que salir ALTA — y con volatilidad, no exactamente 1: algún camino
    //     afortunado sí llega.
    let forced = case("P21_retire_at_age_reading_only");
    let out = project_percentile_bands(&forced, &[Some(20.0)], &config).expect("no falla");
    assert!(out.retirement_month_index_percentiles.is_none());
    let p = out
        .underfunded_probability
        .expect("con trigger por edad, la infra-financiación existe");
    println!("[trigger] P21 (edad) · probabilidad de infra-financiación = {p:.3}");
    assert!((0.0..=1.0).contains(&p));
    assert!(
        p > 0.5,
        "el caso está pineado como infra-financiado en el camino determinista: {p}"
    );
}

// =================================================================================================
// 8. Las dos lecturas que la segunda revisión adversarial (D20) corrigió
// =================================================================================================

/// **La cobertura cuenta la necesidad que la CARTERA no pudo fundar, no solo la que la regla
/// rechazó.**
///
/// El hogar: 100.000 € al 4 % con σ = 15 %, 3.000 €/mes de gasto, 400 meses. Se arruina en el mes
/// 35 (mediana) y pasa 364 de los 400 meses con la cartera vacía. Con `fixed_real` la regla NO
/// recorta nunca (`withdrawal_shortfall ≡ 0` por construcción: el permitido ES la necesidad), así
/// que el denominador `Σ(w + s)` era `Σ w` y el cociente salía **1,0 en los 1.000 caminos** — «la
/// regla cubrió el 100 % de la necesidad» sobre hogares que cubrieron el 8,8 %.
///
/// Lo que faltaba estaba en la tercera magnitud, `unmet_need`, que el motor no publicaba mes a
/// mes. Ahora sí, y el cociente es `Σ w / Σ (w + recorte + descubierto)`.
#[test]
fn mc_coverage_counts_the_need_the_portfolio_could_not_fund() {
    let input = single_asset_retiree(
        Decimal::from(100_000),
        Decimal::from(3_000),
        Decimal::from(4),
        400,
    );
    let config = McConfig {
        seed: 207,
        paths: 1_000,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };
    let out = project_percentile_bands(&input, &[Some(15.0)], &config).expect("no falla");
    let ratio = out
        .withdrawal_to_need_ratio_p50
        .expect("hay meses jubilados");
    println!(
        "\n[cobertura] 100.000 € al 4 %/15 % · 3.000 €/mes · 400 meses · 1.000 caminos\n\
         [cobertura]   éxito = {:.4}   cobertura p50 = {ratio:.4}   meses por debajo p50 = {}",
        out.success_probability, out.months_below_need_p50
    );

    assert_eq!(out.success_probability, 0.0, "ningún camino sobrevive");
    assert!(
        (0.05..0.15).contains(&ratio),
        "la cobertura real ronda el 8,8 %, no el 100 %: medido {ratio}"
    );
    assert!(
        out.months_below_need_p50 > 300,
        "el camino mediano pasa la mayor parte del horizonte sin cubrir su gasto: {}",
        out.months_below_need_p50
    );
}

/// **No jubilarse nunca no es un éxito.**
///
/// El hogar: 1.000 € de partida, 2.100 € de ingreso contra 2.000 € de gasto, 6,5 % con σ = 17 %,
/// SWR 4 % (objetivo 600.000 €), 840 meses, todo el sobrante a un único fondo. El camino
/// determinista se jubila en el mes 655 — al filo del horizonte—, así que **un tercio de los
/// caminos sorteados no llega nunca**.
///
/// Con la definición anterior (D22: «la cartera no se agota»), esos 331 caminos contaban como
/// éxito porque un hogar que nunca se jubila nunca drena: 0,960 publicado. Entre los que sí se
/// jubilan, la ruina es del 6 % y el éxito 0,940. La diferencia llega a **+6,8 pp** en el barrido
/// medido (SWR 6 %, ingreso 2.050).
#[test]
fn mc_never_retiring_is_not_a_success() {
    let input = crossing_household(
        Decimal::from(1_000),
        Decimal::from(2_100),
        Decimal::from(2_000),
        Decimal::try_from(6.5).unwrap(),
        Decimal::from(4),
        840,
    );
    let config = McConfig {
        seed: 207,
        paths: 1_000,
        percentiles: vec![10, 50, 90],
        cash_buffer: None,
    };
    let out = project_percentile_bands(&input, &[Some(17.0)], &config).expect("no falla");
    let conditional = out.success_given_retired.expect("algún camino se jubila");
    println!(
        "\n[éxito] cruce a 840 meses · 1.000 caminos\n\
         [éxito]   éxito = {:.4}   nunca se jubilan = {:.4}   éxito | jubilado = {conditional:.4}",
        out.success_probability, out.never_retired_probability
    );

    // Un tercio de los caminos no llega: eso ya no se cuenta como plan cumplido.
    assert!(
        (0.30..0.36).contains(&out.never_retired_probability),
        "nunca se jubilan: {}",
        out.never_retired_probability
    );
    assert!(
        (out.success_probability - (1.0 - out.never_retired_probability) * conditional).abs()
            < 1e-9,
        "éxito = P(jubilarse) × P(no agotar | jubilado)"
    );
    // Y la lectura vieja («no agotar», sin exigir jubilarse) era exactamente 0,960: la diferencia
    // con la nueva es la fracción que no llega.
    assert!(
        out.success_probability < 0.70,
        "el éxito honesto de este hogar está muy por debajo del 0,960 que se publicaba: {}",
        out.success_probability
    );
    assert!(
        (0.92..0.96).contains(&conditional),
        "condicional a jubilarse, la ruina sigue siendo baja: {conditional}"
    );
}
