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
    project_net_worth_series, PhasePlan, ProjectionInput, SimAsset, SpendMode, WithdrawalRule,
};
use futurefin_engine_stochastic::{
    project_percentile_bands, run_path, seed_for, simulate_f64, McConfig, McOutcome,
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
    let mut input = single_asset_retiree(cash + equity, monthly_expense, annual_return, horizon);
    input.assets = vec![
        SimAsset {
            id: uuid::Uuid::from_u128(1),
            value: cash,
            purchase_price: None,
            is_liquid: true,
            expected_annual_return_percent: Some(Decimal::ZERO),
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
        cash_buffer_months: None,
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
        let expected = if det.assets_depleted_month_index.is_none() {
            1.0
        } else {
            0.0
        };
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
        cash_buffer_months: None,
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
        cash_buffer_months: None,
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

    // Con `fixed_real` la regla NO recorta nunca: el hogar retira lo que necesita hasta que no
    // queda nada. El recorte es cero y lo que hay es agotamiento — la separación de D22/D24.
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
    // La última fila NO es la ruina total: la tabla avanza de 60 en 60 desde la jubilación (mes
    // 1) y se detiene en el último múltiplo que cabe en el horizonte —el mes 361 de 420—, así que
    // los caminos que se agotan en los últimos cinco años quedan fuera. Es acumulada y acotada
    // por la ruina, no igual a ella; quien lea la tabla tiene que saberlo.
    let last_row = *cumulative.last().expect("hay filas");
    assert!(
        last_row <= ruin4,
        "la tabla acumulada ({last_row}) no puede pasarse de la ruina total ({ruin4})"
    );
    assert_eq!(
        out4.depletion_probability_by_age
            .last()
            .expect("hay filas")
            .0,
        361,
        "la última fila es el último múltiplo de 60 que cabe en el horizonte"
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
        cash_buffer_months: None,
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
/// 1. **Sin volatilidad declarada NO se instala.** No es que «no haya de dónde vender»:
///    `PathEngine::new` exige tres cosas para simularlo —el usuario lo pide, hay un activo
///    líquido que lo albergue y hay volatilidad de la que protegerse— y aquí falla la tercera.
///    `z_k` se sigue sorteando (el flujo del RNG no depende de los datos) pero no mueve ningún
///    retorno: rellenar «en los meses buenos» sería trasvasar valor —y pagar plusvalías—
///    guiándose por un shock que no afecta a nada, y además rompería la puerta de degeneración de
///    WP5.5 («σ=0 ⇒ la banda ES la línea determinista»). Resultado: `buffer_active: false`, las
///    dos lecturas a `None` («no se midió», que no es «cero rellenos») y un [`McOutcome`]
///    idéntico bit a bit al de no pedirlo.
/// 2. **Con volatilidad SÍ se instala, y aun así no mueve nada si no hay de dónde vender.** P7
///    tiene UN solo activo, que es a la vez el colchón: el conjunto vendible es vacío porque el
///    colchón jamás se vende a sí mismo (eso sería un trasvase circular que sube su propia base
///    sin mover un euro). `buffer_active: true`, `buffer_refills_p50: Some(0)` y las bandas
///    EXACTAMENTE iguales que sin colchón.
#[test]
fn mc_cash_buffer_is_installed_only_when_it_can_mean_something() {
    let input = case("P7_jubilado_pension_impuestos");
    let without = McConfig {
        seed: 5,
        paths: 32,
        percentiles: vec![10, 50, 90],
        cash_buffer_months: None,
    };
    let with = McConfig {
        cash_buffer_months: Some(24),
        ..without.clone()
    };

    // (1) σ = 0 en toda la cartera: el colchón no se instala y el resultado es el mismo objeto.
    let flat: Vec<Option<f64>> = input.assets.iter().map(|_| None).collect();
    let a = project_percentile_bands(&input, &flat, &without).expect("no falla");
    let b = project_percentile_bands(&input, &flat, &with).expect("no falla");
    assert!(!a.buffer_active && !b.buffer_active);
    assert_eq!(b.buffer_refills_p50, None);
    assert_eq!(b.buffer_refill_net_total_p50, None);
    assert_eq!(
        a, b,
        "con σ=0 el colchón no se instala: pedirlo no puede mover un dígito"
    );

    // (2) σ = 12 %: se instala, pero P7 tiene un solo activo y el colchón no se vende a sí mismo.
    let vols: Vec<Option<f64>> = input.assets.iter().map(|_| Some(12.0)).collect();
    let live_off = project_percentile_bands(&input, &vols, &without).expect("no falla");
    let live_on = project_percentile_bands(&input, &vols, &with).expect("no falla");
    println!(
        "[colchón] P7 (1 activo) · σ=0 ⇒ instalado={} · σ=12 % ⇒ instalado={} rellenos={:?} movido={:?}",
        b.buffer_active,
        live_on.buffer_active,
        live_on.buffer_refills_p50,
        live_on.buffer_refill_net_total_p50
    );
    assert!(
        live_on.buffer_active,
        "con volatilidad el colchón se simula"
    );
    assert_eq!(live_on.buffer_refills_p50, Some(0));
    assert_eq!(live_on.buffer_refill_net_total_p50, Some(0.0));
    assert_eq!(
        live_off.net_worth, live_on.net_worth,
        "sin otro activo que vender, el colchón no puede mover una sola serie"
    );
    assert_eq!(live_off.liquid_worth, live_on.liquid_worth);
}

/// **Con volatilidad y con algo que vender, el colchón cambia la distribución — y la empeora.**
///
/// El laboratorio del issue con manga de caja: 1.000.000 € (80.000 en cuenta al 0 % = 24 meses de
/// gasto, 920.000 en RV al 6,5 % con σ = 17 %), retirada fija real del 4 % del capital inicial, 35
/// años, sin impuestos y sin IPC. Las dos ejecuciones parten de la MISMA cartera; lo único que
/// cambia es si la cuenta se vuelve a llenar en los meses de shock positivo o se gasta una vez.
///
/// # Predicción y resultado
///
/// Se predijeron dos fuerzas opuestas: el **lastre de caja** (mantener 80.000 € fuera del mercado
/// cuesta ~5.200 €/año de crecimiento esperado) contra el **riesgo de secuencia** (sin colchón se
/// vende RV también después de una caída; con colchón, solo tras subir). La predicción escrita
/// antes de ejecutar fue «p10 arriba, p90 abajo, éxito arriba: la cola manda en la ruina».
///
/// **La predicción falló en el signo, y el motivo está dentro del propio modelo.** Medido con
/// 1.000 caminos y semilla 207:
///
/// ```text
///   colchón (meses):   0        6        12       24       60
///   éxito:             0,777    0,739    0,731    0,713    0,641
///   p10 del mes 240:  95.581   56.822   52.993   57.136   56.352
///   p50 final:      1.575.208 1.295.498 1.222.917 1.050.304  528.554
/// ```
///
/// Monótono en el tamaño del colchón y **en contra** en todos los percentiles, la cola incluida.
/// La razón no es un fallo del colchón: es que **este modelo no tiene autocorrelación** (está
/// declarado en el doc del módulo `mc`). Con shocks mensuales independientes, un mes malo no dice
/// nada del siguiente, así que no hay «mala racha que esperar sentado»: el colchón no compra
/// ninguna información y el lastre —que sí es cierto todos los meses— se cobra entero. En los
/// backtests históricos el colchón parece ayudar porque las series reales SÍ revierten a la
/// media; esa propiedad no está aquí y por eso el resultado no puede ser el de la literatura de
/// backtest.
///
/// Lo que este test fija, entonces: el colchón **actúa de verdad** (se rellena, mueve las bandas)
/// y, dentro de este modelo, **cuesta rentabilidad sin comprar seguridad**. Si alguien añade
/// reversión a la media al modelo, este `assert` se caerá — y eso será exactamente lo que hay que
/// mirar.
#[test]
fn mc_cash_buffer_costs_return_without_buying_safety_in_this_model() {
    let monthly =
        Decimal::from(1_000_000) * Decimal::from(4) / Decimal::from(100) / Decimal::from(12);
    let input = buffered_retiree(
        Decimal::from(80_000),
        Decimal::from(920_000),
        monthly,
        Decimal::try_from(6.5).unwrap(),
        420,
    );
    let vols = vec![None, Some(17.0)];
    let without = McConfig {
        seed: 207,
        paths: 1_000,
        percentiles: vec![10, 50, 90],
        cash_buffer_months: None,
    };
    let with = McConfig {
        cash_buffer_months: Some(24),
        ..without.clone()
    };
    let a = project_percentile_bands(&input, &vols, &without).expect("no falla");
    let b = project_percentile_bands(&input, &vols, &with).expect("no falla");
    assert!(!a.buffer_active && b.buffer_active);

    let mid = 240usize;
    let last = 420usize;
    println!(
        "\n[colchón] 1.000.000 € (80.000 cuenta + 920.000 RV 6,5 %/17 %) · 4 % real · 35 años · 1.000 caminos\n\
         [colchón]   éxito             sin colchón = {:.3}   con colchón = {:.3}   (Δ {:+.3})\n\
         [colchón]   líquido p10 mes 240 = {:>12.0} → {:>12.0} €\n\
         [colchón]   líquido p50 final   = {:>12.0} → {:>12.0} €\n\
         [colchón]   líquido p90 final   = {:>12.0} → {:>12.0} €\n\
         [colchón]   rellenos p50 = {:?} de 420 meses · movido p50 = {:?} €",
        a.success_probability,
        b.success_probability,
        b.success_probability - a.success_probability,
        a.liquid_worth[0][mid],
        b.liquid_worth[0][mid],
        a.liquid_worth[1][last],
        b.liquid_worth[1][last],
        a.liquid_worth[2][last],
        b.liquid_worth[2][last],
        b.buffer_refills_p50,
        b.buffer_refill_net_total_p50,
    );

    // (1) El colchón se rellena de verdad: ni cero meses ni cero euros. El total movido en la
    //     mediana ronda el gasto acumulado del horizonte (1,4 M€ en 420 meses), porque
    //     prácticamente TODO el gasto acaba pasando por la cuenta.
    let refills = b.buffer_refills_p50.expect("se simuló");
    let moved = b.buffer_refill_net_total_p50.expect("se simuló");
    assert!(refills > 0 && moved > 0.0);
    assert!(
        refills < 420,
        "solo se rellena en los meses de shock POSITIVO, que no son todos: {refills}"
    );
    // (2) Las bandas se MUEVEN: un colchón que no cambia la distribución no es un colchón.
    assert_ne!(
        a.liquid_worth, b.liquid_worth,
        "el colchón no ha movido la banda: o no actúa, o el gancho está desconectado"
    );
    // (3) Y se mueven a peor, en la mediana y en la cola: ver el porqué en el doc.
    assert!(
        b.liquid_worth[1][last] < a.liquid_worth[1][last],
        "el lastre de caja tiene que verse en la mediana"
    );
    assert!(
        b.success_probability <= a.success_probability,
        "con shocks independientes el colchón no puede MEJORAR la ruina: {:.3} > {:.3} \
         — si el modelo ha ganado reversión a la media, actualiza este test Y la ayuda de la SPA",
        b.success_probability,
        a.success_probability
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
