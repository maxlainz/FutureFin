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
//! 4. **El modelo hace lo que dice** — la MEDIANA del terminal sobre 2 500 caminos ES la línea
//!    determinista (la rentabilidad declarada es una CAGR, decisión M8) y la MEDIA queda por
//!    encima justo la prima de varianza, las dos dentro de tolerancias DERIVADAS de la log-normal
//!    (`mc_median_is_the_deterministic_line`).
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
    project_net_worth_series, PhasePlan, ProjectionInput, SimAsset,
};
use futurefin_engine_stochastic::{
    project_percentile_bands, run_path, seed_for, simulate_f64, McConfig, McOutcome,
    WILSON_Z_95,
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
            start_month: None,
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
        threads: None,
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
        // **E9 (McOutcome v2)**: el éxito ya no se reconstruye a mano comparando jubilación y
        // agotamiento por separado — se lee DIRECTAMENTE de `failure_month_index` (ningún fallo
        // F1/F2), la MISMA fuente que usa `project_percentile_bands`. Con σ=0 los `paths`
        // caminos son el determinista, así que el éxito es EXACTAMENTE 0 o 1 y coincide con él.
        //
        // La comparación vieja (jubilación + `assets_depleted_month_index`) dejó de ser
        // equivalente en cuanto F2 (y, hasta el 2026-10-10, F3) entraron en el bucle: un camino
        // puede fallar por la tasa inicial SIN agotar nunca la cartera, y la vieja fórmula lo
        // habría marcado «éxito».
        let expected = if det.failure_month_index.is_none() { 1.0 } else { 0.0 };
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
    // 23 hasta el 2026-10-10: P15–P17 salieron de la batería con las reglas de retirada por saldo.
    assert!(
        checked >= 22,
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
        threads: None,
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
// 4. El modelo hace lo que dice: mediana(f) = m,  E[f] = m·exp(σ_m²/2)
// =================================================================================================

/// **La línea determinista es la MEDIANA de los caminos; la media va por encima.**
///
/// La rentabilidad declarada de un activo es COMPUESTA —una CAGR, la que publican los fondos
/// (decisión M8 del modelo v2, owner 2026-09-06)—, así que el factor mensual sorteado tiene que
/// tener a `m` por **mediana**, no por media. El sorteo lo consigue subiendo la deriva a
/// `d = m·exp(σ_m²/2)` (`PathEngine::new`), y este test mide las DOS consecuencias a la vez: dónde
/// cae la mediana del terminal y dónde cae su media.
///
/// Un solo activo, sin gasto ni ingreso ni impuestos: el patrimonio terminal de un camino es
///
/// ```text
///   V_H = V_0 · Π_k d·exp(σ z_k − σ²/2) = V_0·m^H · exp(σ·S) = D · exp(σ·S),   S = Σ z_k ~ N(0,H)
/// ```
///
/// con `D = V_0·m^H` el terminal determinista. `exp(σ·S)` es log-normal de **mediana 1** y media
/// `exp(H·σ²/2)`, luego
///
/// ```text
///   mediana(V_H) = D                    ← la línea determinista, EXACTA (sin flujos)
///   E[V_H]       = D · exp(H·σ_m²/2)    ← la prima de varianza
///   Var(V_H)     = E[V_H]²·(exp(H·σ_m²) − 1)
/// ```
///
/// # Predicción, escrita ANTES de correr (100.000 € · 7 % CAGR · σ_a = 15 % · H = 120 · N = 2.500)
///
/// ```text
///   D          = 100.000 · 1,07^10                        = 196.715,14 €
///   σ_m²       = 0,15² / 12                               =      0,001875
///   H·σ_m²/2   = 120 · 0,001875 / 2                       =      0,1125
///   E[V_H]     = 196.715,14 · exp(0,1125) = · 1,1190723   = 220.138,45 €
/// ```
///
/// # Las tolerancias, derivadas y no elegidas a ojo
///
/// `s = σ_m·√H = √0,225 = 0,4743` es la desviación típica del LOG del terminal.
///
/// - **Mediana muestral**: error típico `1/(2·f(D)·√N)` con `f` la densidad log-normal evaluada en
///   su mediana, `f(D) = 1/(D·s·√(2π))` ⇒ `sd = D·s·√(2π)/(2·√N) = 1,19 % · D` con `N = 2.500`.
///   La cota exigida —**5 %**— es ≈ **4,2 σ**.
/// - **Media muestral**: la sd relativa de un camino es `√(exp(H·σ_m²) − 1) = 0,5023`; dividida por
///   `√N` da **1,00 %**. La misma cota del 5 % es ≈ **5,0 σ**.
///
/// Por debajo de ~4 σ un test así falla de vez en cuando por azar aunque el modelo sea correcto, y
/// un test que falla al azar es un test que se acaba ignorando. La semilla es fija, así que lo
/// observado es DETERMINISTA — se imprime al lado de lo predicho.
#[test]
fn mc_median_is_the_deterministic_line() {
    let capital = Decimal::from(100_000);
    let horizon = 120u32;
    let input = single_asset_retiree(capital, Decimal::ZERO, Decimal::from(7), horizon);
    let vols = vec![Some(15.0)];
    let paths = 2_500u32;
    let config = McConfig {
        seed: 4_242,
        paths,
        ..Default::default()
    };

    let deterministic = simulate_f64(&input).expect("no falla");
    let d_terminal = deterministic.net_worth[horizon as usize].0;

    let mut terminals: Vec<f64> = Vec::with_capacity(paths as usize);
    for p in 0..paths {
        let out = run_path(&input, &vols, &config, p).expect("ningún camino falla");
        terminals.push(out.net_worth[horizon as usize].0);
    }

    let n = f64::from(paths);
    let mean = terminals.iter().sum::<f64>() / n;
    let var = terminals.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0);
    // Mediana por **rango más cercano**, la misma convención que las bandas
    // (`nearest_rank_index`): `⌈50·N/100⌉ − 1`. Nunca interpola — el valor es uno que el sorteo
    // produjo de verdad.
    let mut sorted = terminals.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[(paths as usize).div_ceil(2) - 1];

    // Predicciones cerradas del modelo, para contrastarlas con lo medido.
    let sigma_m2 = (0.15f64 / 12f64.sqrt()).powi(2);
    let h_sigma2 = f64::from(horizon) * sigma_m2;
    let predicted_mean = d_terminal * (h_sigma2 / 2.0).exp();
    let predicted_rel_sd = (h_sigma2.exp() - 1.0).sqrt();
    let predicted_median_sd = h_sigma2.sqrt() * (2.0 * std::f64::consts::PI).sqrt() / (2.0 * n.sqrt());
    let predicted_mean_sd = predicted_rel_sd / n.sqrt();

    let median_err = (median - d_terminal) / d_terminal;
    let mean_err = (mean - predicted_mean) / predicted_mean;
    let observed_rel_sd = var.sqrt() / predicted_mean;

    println!(
        "\n[mediana] 1 activo · 100.000 € · 7 % CAGR · σ 15 % · {horizon} meses · {paths} caminos\n\
         [mediana]   línea determinista D  = predicha 196.715,14 €   medida {d_terminal:.2} €\n\
         [mediana]   MEDIANA del terminal  = predicha {d_terminal:.2} €   medida {median:.2} €   \
         (error {:+.3} %, cota 5 % ≈ {:.1} σ)\n\
         [mediana]   MEDIA del terminal    = predicha {predicted_mean:.2} €   medida {mean:.2} €   \
         (error {:+.3} %, cota 5 % ≈ {:.1} σ)\n\
         [mediana]   prima de varianza exp(H·σ_m²/2) = {:.6}   (media/mediana medida = {:.6})\n\
         [mediana]   sd relativa del camino: predicha {predicted_rel_sd:.4}, observada {observed_rel_sd:.4}",
        median_err * 100.0,
        0.05 / predicted_median_sd,
        mean_err * 100.0,
        0.05 / predicted_mean_sd,
        (h_sigma2 / 2.0).exp(),
        mean / median,
    );

    assert!(
        median_err.abs() < 0.05,
        "la MEDIANA muestral se desvía {:.3} % de la línea determinista: la rentabilidad declarada \
         ha dejado de ser la COMPUESTA",
        median_err * 100.0
    );
    assert!(
        mean_err.abs() < 0.05,
        "la MEDIA muestral se desvía {:.3} % de D·exp(H·σ_m²/2): la prima de varianza no es la del \
         modelo",
        mean_err * 100.0
    );
    // Y la media tiene que quedar POR ENCIMA de la mediana: si coincidieran, la conversión
    // CAGR → aritmética no se estaría aplicando y la declarada volvería a ser la media.
    assert!(
        mean > median,
        "sin prima de varianza la media ({mean:.2} €) no supera a la mediana ({median:.2} €)"
    );
    // La dispersión también debe ser la del modelo (± 20 % relativo sobre la sd, que con 2 500
    // caminos tiene su propio error de ~1/√(2N) = 1,4 % más la asimetría log-normal).
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
        threads: None,
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
/// - **`6,5 %` es media ARITMÉTICA allí y CAGR aquí (M8, modelo v2).** El issue toma ese 6,5 %
///   como media aritmética anual; desde el modelo v2 este motor lo lee como COMPUESTO y sube la
///   deriva para que la geométrica siga siendo 6,5 %, de modo que su media aritmética equivalente
///   es `1,065·exp(σ_a²/2) − 1 ≈ 8,0 %`: **~1,5 pp/año más de deriva** que el modelo del issue.
///   Es el tercer empujón a la baja sobre la ruina, y el mayor de los tres.
///
/// Por eso se exigen horquillas propias en vez de las del issue: lo que este test prueba es que el
/// orden de magnitud y —sobre todo— la RELACIÓN entre el 3 % y el 4 % son las que la literatura
/// describe. Los valores medidos se imprimen para que la comparación la haga quien lea la salida,
/// no el `assert`.
///
/// # Las horquillas se re-centraron con la convención CAGR (E5, modelo v2, 2026-09-06)
///
/// Al pasar la rentabilidad declarada de aritmética a COMPUESTA, la deriva de este activo subió
/// `exp(σ_a²/2) = exp(0,17²/2) = 1,0146` al año y la ruina bajó, medido con la misma semilla y los
/// mismos 1.000 caminos:
///
/// ```text
///   retirada     antes (declarada = aritmética)   ahora (declarada = CAGR)   horquilla
///     3 %                 10,60 %                        5,00 %              2-12 %  (antes 3-15 %)
///     4 %                 22,30 %                       13,00 %              7-22 %  (antes 12-30 %)
/// ```
///
/// Las horquillas nuevas son **más estrechas** que las viejas (10 y 15 puntos frente a 12 y 18): se
/// re-centran sobre lo medido, no se ensanchan para que pase. El 13,00 % del 4 % estaba a 1 punto
/// del suelo de 12 %, y un suelo a un punto es un test que se acaba relajando con prisa.
#[test]
fn mc_success_probability_of_the_issue_table() {
    let paths = 1_000u32;
    let (ruin3, out3) = ruin_probability(3.0, paths);
    let (ruin4, out4) = ruin_probability(4.0, paths);

    println!(
        "\n[issue #207] 1.000.000 € · 6,5 % media · 17 % sd · 35 años · {paths} caminos\n\
         [issue #207]   retirada 3 % ({:>7.2} €/mes): ruina = {:>6.2} %   (issue: 7-10 %, exigido 2-12 %)\n\
         [issue #207]   retirada 4 % ({:>7.2} €/mes): ruina = {:>6.2} %   (issue: 18-23 %, exigido 7-22 %)\n\
         [issue #207]   éxito 3 % = {:.3}   éxito 4 % = {:.3}",
        1_000_000.0 * 0.03 / 12.0,
        ruin3 * 100.0,
        1_000_000.0 * 0.04 / 12.0,
        ruin4 * 100.0,
        out3.success_probability,
        out4.success_probability,
    );

    assert!(
        (0.02..=0.12).contains(&ruin3),
        "ruina al 3 % = {:.2} %, fuera de 2-12 %",
        ruin3 * 100.0
    );
    assert!(
        (0.07..=0.22).contains(&ruin4),
        "ruina al 4 % = {:.2} %, fuera de 7-22 %",
        ruin4 * 100.0
    );
    assert!(
        ruin4 > ruin3,
        "retirar más no puede arruinar menos: {ruin4} ≤ {ruin3}"
    );

    // La tabla de FALLO acumulado (E9: `cumulative_failure_by_age`, sustituye a
    // `depletion_probability_by_age`) arranca en la jubilación (mes 1) y avanza de 5 en 5 años.
    // Sin puerta de tasa inicial (`single_asset_retiree` no declara ninguna) el único motivo posible es F1
    // (`PortfolioDepleted`), así que estos números no cambian frente a la vieja tabla de
    // agotamiento — es la MISMA cuenta, leída de la fuente nueva.
    assert_eq!(out4.cumulative_failure_by_age[0].0, 1);
    assert_eq!(out4.cumulative_failure_by_age[1].0, 61);
    let cumulative: Vec<f64> = out4
        .cumulative_failure_by_age
        .iter()
        .map(|(_, p)| *p)
        .collect();
    println!("[issue #207]   fallo acumulado cada 5 años (4 %): {cumulative:?}");
    for w in cumulative.windows(2) {
        assert!(w[1] >= w[0], "una probabilidad ACUMULADA no puede bajar");
    }
    // **La última fila ES el horizonte** (corrección de la revisión adversarial): la rejilla
    // avanza de 60 en 60 desde la jubilación y antes se detenía en el último múltiplo que cabía
    // —el mes 361 de 420—, dejando fuera sin avisar a los caminos que se agotaban en los últimos
    // cinco años. Ahora cierra en el mes 420 y esa fila ES `1 − éxito`.
    let last_row = *cumulative.last().expect("hay filas");
    assert_eq!(
        out4.cumulative_failure_by_age.last().expect("hay filas").0,
        420,
        "la última fila de la tabla es el HORIZONTE, no el último múltiplo de 60"
    );
    assert!(
        (last_row - ruin4).abs() < 1e-12,
        "la última fila ({last_row}) es la ruina total ({ruin4}, = 1 − éxito)"
    );
}

// `mc_percent_of_balance_never_ruins_but_cuts_the_spending` y
// `mc_f3_is_a_property_of_the_plan_not_of_the_draw` vivían aquí: medían la regla
// `percent_of_balance` como gasto y su fallo F3. Se retiraron el 2026-10-10 con las reglas de
// retirada por saldo, el modo de gasto y F3 (decisión «solo SWR»). Su medición —2,52 M€ de
// capital necesario con F3 mes a mes frente a 620 k€ con el gasto fijo— está en
// `futurefin-failure-archaeology`.

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

/// **Las lecturas de fallo ya NO dependen del tipo de trigger** (E9, `McOutcome` v2).
///
/// Antes de este WP, `retirement_month_index_percentiles` solo existía por CRUCE y
/// `underfunded_probability` solo por EDAD — dos campos que se excluían mutuamente según cómo se
/// jubilara el plan. Los dos se retiraron: con la API v2 todo plan se sortea con un mes FORZADO
/// (`RetirementTrigger::AtMonth`, resuelto por el solver externo antes de llegar aquí), así que
/// «el mes de jubilación es una distribución» dejó de tener sentido y la infra-financiación de una
/// edad fija es ahora `1 − éxito(R)` (`solve_mc::success_at_month`). Lo que SÍ se publica —
/// `failures_by_kind` y `cumulative_failure_by_age`— se publica IGUAL sea cual sea el trigger, y
/// eso es justo lo que este test comprueba: un plan legacy por CRUCE (P3, que no ha migrado al mes
/// forzado) y un plan por mes FORZADO (P21) dan lecturas con la MISMA forma.
///
/// Reemplaza a `mc_readings_follow_the_retirement_trigger`.
#[test]
fn mc_readings_are_the_same_for_every_trigger_now() {
    let config = McConfig {
        seed: 11,
        paths: 200,
        ..Default::default()
    };

    // (a) P3 sigue trayendo `RetirementTrigger::LiquidCrossing` (caso legacy que no ha migrado al
    //     mes forzado de la v2): el ANCLA de `cumulative_failure_by_age` cae al camino
    //     determinista (ver el doc de `McOutcome::cumulative_failure_by_age`), no al mes forzado.
    let crossing = case("P3_superavit_jubilacion");
    let out_crossing = project_percentile_bands(&crossing, &[Some(18.0)], &config).expect("no falla");

    // (b) P21 trae `RetirementTrigger::AtMonth` — el caso normal desde 5.0.0.
    let forced = case("P21_retire_at_age_reading_only");
    let out_forced = project_percentile_bands(&forced, &[Some(20.0)], &config).expect("no falla");

    for (label, out) in [
        ("P3 (cruce, legacy)", &out_crossing),
        ("P21 (mes forzado)", &out_forced),
    ] {
        let n = f64::from(out.paths);
        let failures_counted: u32 = out.failures_by_kind.iter().sum();
        let failures_expected = ((1.0 - out.success_probability) * n).round() as u32;
        println!(
            "[trigger] {label} · éxito = {:.4}   fallos por motivo = {:?} (suma {failures_counted}, \
             esperado {failures_expected})\n[trigger] {label} · fallo acumulado = {:?}",
            out.success_probability, out.failures_by_kind, out.cumulative_failure_by_age
        );
        assert_eq!(
            failures_counted, failures_expected,
            "{label}: `failures_by_kind` no suma `paths − paths·éxito`"
        );

        assert!(
            !out.cumulative_failure_by_age.is_empty(),
            "{label}: los dos casos se jubilan dentro del horizonte, la tabla no puede ir vacía"
        );
        for w in out.cumulative_failure_by_age.windows(2) {
            assert!(
                w[1].1 >= w[0].1,
                "{label}: una probabilidad ACUMULADA no puede bajar ({:?} → {:?})",
                w[0],
                w[1]
            );
        }
        let (last_month, last_p) = *out.cumulative_failure_by_age.last().expect("no vacío");
        assert_eq!(
            last_month, out.horizon_months,
            "{label}: la última fila de la tabla es el HORIZONTE"
        );
        assert!(
            (last_p - (1.0 - out.success_probability)).abs() < 1e-9,
            "{label}: la última fila ({last_p:.4}) debe coincidir con 1 − éxito ({:.4})",
            1.0 - out.success_probability
        );
    }
}

// =================================================================================================
// 8. Las dos lecturas que la segunda revisión adversarial (D20) corrigió
// =================================================================================================

// `mc_coverage_counts_the_need_the_portfolio_could_not_fund` vivía aquí y medía
// `withdrawal_to_need_ratio_p50` / `months_below_need_p50`. Las dos lecturas se retiraron el
// 2026-10-10 con las reglas de retirada por saldo: sin reglas, un mes por debajo del gasto ES un
// fallo F1, así que sus medianas solo podían ser distintas de «0 meses / 100 %» con un éxito por
// debajo del 50 % — fuera de cualquier umbral que la app acepta (80–100).

/// **La ruina es F1 y solo F1, y el éxito es 0 cuando ningún camino sobrevive.**
///
/// El hogar del laboratorio de cobertura que se retiró el 2026-10-10 (100.000 € al 4 % con
/// σ = 15 %, 3.000 €/mes, 400 meses) se arruina en TODOS los caminos. La métrica de cobertura se
/// fue con las reglas por saldo; la aserción que viajaba con ella —«ningún camino sobrevive»— se
/// queda aquí, y gana el reparto por motivo: `single_asset_retiree` no lleva puerta de tasa inicial
/// (`PhasePlan::classic` → `initial_rate: None`), así que F2 es imposible y los 1.000 fallos tienen
/// que ser F1 (cartera agotada), ni uno más ni uno menos.
#[test]
fn mc_ruin_is_portfolio_depletion_and_only_that() {
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
        threads: None,
    };
    let out = project_percentile_bands(&input, &[Some(15.0)], &config).expect("no falla");
    println!(
        "\n[ruina=F1] 100.000 € al 4 %/15 % · 3.000 €/mes · 400 meses · 1.000 caminos\n\
         [ruina=F1]   éxito = {:.4}   fallos por motivo (F1, F2) = {:?}",
        out.success_probability, out.failures_by_kind
    );
    assert_eq!(out.success_probability, 0.0, "ningún camino sobrevive");
    assert_eq!(
        out.failures_by_kind,
        [1_000, 0],
        "sin puerta de tasa inicial toda la ruina es F1: {:?}",
        out.failures_by_kind
    );
}

// =================================================================================================
// 9. `McOutcome` v2 (E9): éxito, motivo del fallo y su intervalo
// =================================================================================================

/// **Éxito = cero fallos, y los motivos suman exactamente los caminos fallidos.**
///
/// Dos laboratorios: uno donde NADIE falla (colchón enorme, sin volatilidad — los `paths` caminos
/// son el mismo determinista) y uno donde SÍ hay ruina (el mismo del issue #207, con una mezcla de
/// motivos real). En los dos, `failures_by_kind.iter().sum()` tiene que ser exactamente
/// `paths − paths·éxito` — la propiedad que el `debug_assert` de `project_percentile_bands` ya
/// vigila en cada ejecución, medida aquí desde fuera del crate.
#[test]
fn mc_success_is_zero_failures_and_the_kinds_add_up() {
    // (a) Colchón amplio, sin volatilidad: los 200 caminos son el determinista, que no falla.
    let comfortable = single_asset_retiree(
        Decimal::from(10_000_000),
        Decimal::from(1_000),
        Decimal::from(5),
        120,
    );
    let config = McConfig {
        seed: 1,
        paths: 200,
        ..Default::default()
    };
    let out = project_percentile_bands(&comfortable, &[None], &config).expect("no falla");
    println!(
        "[éxito=0 fallos] éxito = {}   fallos por motivo = {:?}",
        out.success_probability, out.failures_by_kind
    );
    assert_eq!(
        out.success_probability, 1.0,
        "colchón amplio y sin volatilidad: nadie puede fallar"
    );
    assert_eq!(out.failures_by_kind, [0, 0]);

    // (b) El laboratorio de ruina del issue #207 al 4 %: aquí SÍ hay fallos, y tienen que sumar.
    let (ruin, out2) = ruin_probability(4.0, 500);
    let failures_expected = (ruin * 500.0).round() as u32;
    let failures_counted: u32 = out2.failures_by_kind.iter().sum();
    println!(
        "[éxito=fallos suman] ruina = {ruin:.4} (500 caminos)   fallos contados = {failures_counted}   \
         por motivo = {:?} (F1/F2)",
        out2.failures_by_kind
    );
    assert!(failures_counted > 0, "al 4 % tiene que haber ruina de sobra");
    assert_eq!(
        failures_counted, failures_expected,
        "los motivos deben sumar exactamente los caminos fallidos, sea cual sea la mezcla"
    );
}

/// **`cumulative_failure_by_age` es monótona y cierra en el horizonte con `1 − éxito`.**
///
/// Reutiliza el laboratorio de ruina del issue #207 (con retirada al 4 %, que arruina a una
/// fracción material de los caminos): el ancla es el mes 1 (jubilación forzada desde el mes 1 en
/// `single_asset_retiree`), la rejilla avanza de [`FAILURE_STEP_MONTHS`] en
/// [`FAILURE_STEP_MONTHS`] y —sea cual sea el múltiplo que le toque al horizonte— la ÚLTIMA fila
/// tiene que ser el mes 420 con la probabilidad ACUMULADA exactamente `1 − success_probability`, la
/// misma identidad que mide `mc_readings_are_the_same_for_every_trigger_now` para otros dos casos.
#[test]
fn mc_cumulative_failure_by_age_is_monotone_and_closes_at_one_minus_success() {
    let (_, out) = ruin_probability(4.0, 500);
    println!(
        "[fallo acumulado] éxito = {:.4}   tabla = {:?}",
        out.success_probability, out.cumulative_failure_by_age
    );
    assert!(
        !out.cumulative_failure_by_age.is_empty(),
        "el hogar se jubila desde el mes 1 — la tabla no puede ir vacía"
    );
    assert_eq!(
        out.cumulative_failure_by_age[0].0, 1,
        "el ancla es el mes de jubilación forzado (mes 1 en este laboratorio)"
    );
    for w in out.cumulative_failure_by_age.windows(2) {
        assert!(
            w[1].1 >= w[0].1,
            "una probabilidad ACUMULADA no puede bajar ({:?} → {:?})",
            w[0],
            w[1]
        );
    }
    let (last_month, last_p) = *out
        .cumulative_failure_by_age
        .last()
        .expect("no vacío, ya comprobado arriba");
    assert_eq!(last_month, out.horizon_months, "la última fila es el HORIZONTE (420)");
    assert!(
        (last_p - (1.0 - out.success_probability)).abs() < 1e-9,
        "la última fila ({last_p:.4}) debe coincidir con 1 − éxito ({:.4})",
        1.0 - out.success_probability
    );
}

/// **El éxito carga un intervalo de Wilson, y nunca un `half_width_pp` de cero.**
///
/// Con 0 fallos de N, Wilson colapsa a la forma cerrada `n/(n+z²)` (`solve_mc::wilson_lower_bound`,
/// derivada a mano en su propio doc): se mide contra ESA fórmula, no contra un número copiado, para
/// que el pin viaje con la derivación. `half_width_pp` tiene que ser estrictamente positivo incluso
/// aquí — la propiedad entera de usar Wilson en vez de la aproximación normal.
#[test]
fn mc_success_carries_a_wilson_interval() {
    let comfortable = single_asset_retiree(
        Decimal::from(10_000_000),
        Decimal::from(1_000),
        Decimal::from(5),
        120,
    );
    let config = McConfig {
        seed: 1,
        paths: 200,
        ..Default::default()
    };
    let out = project_percentile_bands(&comfortable, &[None], &config).expect("no falla");
    assert_eq!(out.success_probability, 1.0, "0 fallos de N, precondición del test");

    let n = f64::from(out.paths);
    let z2 = WILSON_Z_95 * WILSON_Z_95;
    let expected_wilson_low = n / (n + z2);
    println!(
        "[wilson] N = {} (0 fallos)   wilson_low = {:.6} (forma cerrada: {:.6})   half_width_pp = {:.4}",
        out.paths, out.wilson_low, expected_wilson_low, out.half_width_pp
    );
    assert!(
        (out.wilson_low - expected_wilson_low).abs() < 1e-12,
        "wilson_low ({}) no coincide con la forma cerrada de `solve_mc::SuccessAt` ({expected_wilson_low})",
        out.wilson_low
    );
    assert!(out.wilson_low < 1.0, "0 fallos no es «100 % seguro»");
    assert!(
        out.half_width_pp > 0.0,
        "la barra de error hacia abajo nunca es 0, ni con 0 fallos observados"
    );
}
