//! **LA PUERTA DE DEGENERACIÓN** — el test de aceptación de que el camino de coma flotante y el
//! camino exacto son la MISMA simulación (§B.4/§B.5 del plan de la issue #207).
//!
//! Qué prueba: para **todos** los casos de la batería del motor (`crates/engine/tests/common/cases.rs`,
//! reutilizada por `#[path]` — una sola definición, dos crates que la corren), se ejecuta la
//! proyección en `Decimal` y en [`F64Money`] y se comparan:
//!
//! - `net_worth` y `liquid_worth` **mes a mes, en todo el horizonte** (840 meses en P9/P13/P18);
//! - `retirement_month_index`, `liquid_crossing_month_index`, `assets_depleted_month_index` y
//!   `phase_transitions`, que son las decisiones DISCRETAS del bucle.
//!
//! Por qué importa: si los dos caminos divergieran, Monte Carlo estaría midiendo la dispersión de
//! *otro* modelo y el número que la UI pinta en verde no significaría nada. Es la salvaguarda con
//! la que la arqueología readmite la coma flotante, y la que hace que el bucle genérico valga la
//! pena frente a un segundo bucle duplicado.
//!
//! # La cota, y por qué no es una sola
//!
//! La cota de contrato es **1 € por mes**. Se aplica tal cual a todo caso cuyas magnitudes caben
//! en el rango donde un `f64` puede *distinguir* un euro: por encima de `2^53 ≈ 9,0e15 €` la
//! distancia entre dos `f64` consecutivos ya es mayor que 1 €, así que exigir «± 1 €» ahí no es
//! una cota estricta, es una cota IMPOSIBLE — y una cota imposible no mide nada, solo obliga a
//! desactivar el test. Para esos casos —que en la batería son sintéticos: activos en el techo de
//! `NUMERIC(18,4)` componiendo al 20 % durante 70 años— la cota es RELATIVA
//! ([`REL_TOLERANCE`]), y **el caso se marca en la tabla** para que se vea cuál se está midiendo
//! con qué regla.
//!
//! Ningún caso se excluye. Ninguna cota se relaja «porque falla»: cada fila de la tabla imprime
//! su máximo, su mes y qué regla se le aplicó.

#[path = "../../engine/tests/common/cases.rs"]
mod cases;

use cases::{projection_cases_5_0, projection_cases_all, ProjCase};
use futurefin_engine::{project_net_worth_series, Phase, ProjectionOutput};
use futurefin_engine_stochastic::{
    deterministic_growth_multipliers, simulate_f64, simulate_f64_with_multipliers, F64Money,
};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

/// La cota de contrato: **1 euro** de desviación máxima por mes, en todo el horizonte.
const EUR_TOLERANCE: f64 = 1.0;

/// Cota RELATIVA para los casos cuyas magnitudes superan `2^53` euros, donde el propio espaciado
/// de los `f64` ya es mayor que un euro. `1e-12` deja ~4 órdenes de magnitud de margen sobre el
/// épsilon de la doble precisión (2,2e-16) para el error acumulado de 840 meses de aritmética
/// encadenada.
const REL_TOLERANCE: f64 = 1e-12;

/// Por encima de este valor absoluto, un `f64` ya no puede representar euros enteros: `2^53`.
const EXACT_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

fn f(d: Decimal) -> f64 {
    d.to_f64()
        .expect("todo Decimal de la batería cabe en coma flotante")
}

/// Una serie comparada: el mayor `|Δ|`, en qué mes, y la mayor magnitud implicada.
struct SeriesDelta {
    max_abs: f64,
    at_month: usize,
    max_magnitude: f64,
    /// `|Δ| / |valor|` en el mes del máximo (0 si el valor es 0).
    rel_at_max: f64,
}

fn compare_series(dec: &[Decimal], flo: &[F64Money]) -> SeriesDelta {
    assert_eq!(dec.len(), flo.len(), "las dos series tienen el mismo largo");
    let mut out = SeriesDelta {
        max_abs: 0.0,
        at_month: 0,
        max_magnitude: 0.0,
        rel_at_max: 0.0,
    };
    for (i, (d, x)) in dec.iter().zip(flo.iter()).enumerate() {
        let dv = f(*d);
        let diff = (x.0 - dv).abs();
        out.max_magnitude = out.max_magnitude.max(dv.abs());
        if diff > out.max_abs {
            out.max_abs = diff;
            out.at_month = i;
            out.rel_at_max = if dv == 0.0 { 0.0 } else { diff / dv.abs() };
        }
    }
    out
}

/// ¿Pasa la fila? Devuelve `(ok, regla aplicada)`.
fn verdict(d: &SeriesDelta) -> (bool, &'static str) {
    if d.max_magnitude <= EXACT_INTEGER_LIMIT {
        (d.max_abs <= EUR_TOLERANCE, "≤ 1 €")
    } else {
        (d.rel_at_max <= REL_TOLERANCE, "relativa 1e-12")
    }
}

/// Diferencia entre dos índices opcionales. `None` = los dos ausentes o los dos presentes e
/// iguales; `Some(n)` = distancia en meses; `Some(i64::MAX)` = uno existe y el otro no.
fn index_delta(a: Option<u32>, b: Option<u32>) -> Option<i64> {
    match (a, b) {
        (None, None) => None,
        (Some(x), Some(y)) => {
            let d = i64::from(x) - i64::from(y);
            (d != 0).then_some(d)
        }
        _ => Some(i64::MAX),
    }
}

fn describe(delta: Option<i64>) -> String {
    match delta {
        None => "=".to_string(),
        Some(i64::MAX) => "PRESENCIA DISTINTA".to_string(),
        Some(d) => format!("{d:+}"),
    }
}

fn all_cases() -> Vec<ProjCase> {
    let mut out = projection_cases_all();
    out.extend(projection_cases_5_0());
    out
}

/// **La puerta**: todos los casos, todo el horizonte, las dos series y los cuatro índices.
#[test]
fn every_case_degenerates_from_decimal_to_floating_point() {
    let cases = all_cases();
    assert!(
        cases.len() >= 23,
        "la batería del motor no puede encogerse sin que este test lo diga: {} casos",
        cases.len()
    );

    let mut failures: Vec<String> = Vec::new();
    println!(
        "\n{:<32} {:>5} {:>11} {:>10} {:>5} {:>11} {:>10} {:>5} {:>13}  {:>6} {:>6} {:>6} {:>6}",
        "caso",
        "meses",
        "max|Δ| NW",
        "rel NW",
        "mes",
        "max|Δ| LIQ",
        "rel LIQ",
        "mes",
        "regla",
        "jubil.",
        "cruce",
        "agot.",
        "fases"
    );

    for case in &cases {
        let dec: ProjectionOutput = match project_net_worth_series(&case.input) {
            Ok(o) => o,
            Err(e) => {
                failures.push(format!("{}: el camino Decimal falló ({e})", case.name));
                continue;
            }
        };
        let flo = match simulate_f64(&case.input) {
            Ok(o) => o,
            Err(e) => {
                failures.push(format!(
                    "{}: el camino de coma flotante falló ({e}) donde el exacto no — \
                     `checked_*` se rindió con un no-finito",
                    case.name
                ));
                continue;
            }
        };

        let nw = compare_series(&dec.net_worth, &flo.net_worth);
        let lq = compare_series(&dec.liquid_worth, &flo.liquid_worth);
        let (nw_ok, nw_rule) = verdict(&nw);
        let (lq_ok, lq_rule) = verdict(&lq);

        let d_ret = index_delta(dec.retirement_month_index, flo.retirement_month_index);
        let d_cross = index_delta(
            dec.liquid_crossing_month_index,
            flo.liquid_crossing_month_index,
        );
        let d_dep = index_delta(
            dec.assets_depleted_month_index,
            flo.assets_depleted_month_index,
        );

        // Fases: la SECUENCIA debe ser la misma; el mes de cada transición, a ≤ 1.
        let phases_dec: Vec<Phase> = dec.phase_transitions.iter().map(|(p, _)| *p).collect();
        let phases_flo: Vec<Phase> = flo.phase_transitions.iter().map(|(p, _)| *p).collect();
        let mut phase_note = String::from("=");
        if phases_dec != phases_flo {
            phase_note = format!("{phases_dec:?} vs {phases_flo:?}");
            failures.push(format!(
                "{}: la SECUENCIA de fases difiere — {phases_dec:?} vs {phases_flo:?}",
                case.name
            ));
        } else {
            let worst = dec
                .phase_transitions
                .iter()
                .zip(flo.phase_transitions.iter())
                .map(|((_, a), (_, b))| i64::from(*a) - i64::from(*b))
                .max_by_key(|d| d.abs())
                .unwrap_or(0);
            if worst != 0 {
                phase_note = format!("{worst:+}");
            }
            if worst.abs() > 1 {
                failures.push(format!(
                    "{}: una transición de fase se mueve {worst} meses (> 1)",
                    case.name
                ));
            }
        }

        debug_assert_eq!(nw_rule, lq_rule, "las dos series comparten magnitud, y por tanto regla");
        println!(
            "{:<32} {:>5} {:>11.3e} {:>10.2e} {:>5} {:>11.3e} {:>10.2e} {:>5} {:>13}  {:>6} {:>6} {:>6} {:>6}",
            case.name,
            case.input.horizon_months,
            nw.max_abs,
            nw.rel_at_max,
            nw.at_month,
            lq.max_abs,
            lq.rel_at_max,
            lq.at_month,
            if nw_rule == lq_rule { nw_rule } else { "mixta" },
            describe(d_ret),
            describe(d_cross),
            describe(d_dep),
            phase_note,
        );

        if !nw_ok {
            failures.push(format!(
                "{}: net_worth se desvía {:.6} € en el mes {} (magnitud máx {:.3e}, relativa {:.3e}, regla «{}»)",
                case.name, nw.max_abs, nw.at_month, nw.max_magnitude, nw.rel_at_max, nw_rule
            ));
        }
        if !lq_ok {
            failures.push(format!(
                "{}: liquid_worth se desvía {:.6} € en el mes {} (magnitud máx {:.3e}, relativa {:.3e}, regla «{}»)",
                case.name, lq.max_abs, lq.at_month, lq.max_magnitude, lq.rel_at_max, lq_rule
            ));
        }
        for (label, delta) in [
            ("retirement_month_index", d_ret),
            ("liquid_crossing_month_index", d_cross),
            ("assets_depleted_month_index", d_dep),
        ] {
            match delta {
                None => {}
                Some(i64::MAX) => failures.push(format!(
                    "{}: {label} existe en un camino y no en el otro ({:?} vs {:?})",
                    case.name,
                    match label {
                        "retirement_month_index" => dec.retirement_month_index,
                        "liquid_crossing_month_index" => dec.liquid_crossing_month_index,
                        _ => dec.assets_depleted_month_index,
                    },
                    match label {
                        "retirement_month_index" => flo.retirement_month_index,
                        "liquid_crossing_month_index" => flo.liquid_crossing_month_index,
                        _ => flo.assets_depleted_month_index,
                    },
                )),
                Some(d) if d.abs() > 1 => failures.push(format!(
                    "{}: {label} se mueve {d} meses (> 1)",
                    case.name
                )),
                Some(_) => {}
            }
        }
    }

    assert!(
        failures.is_empty(),
        "\nLA PUERTA DE DEGENERACIÓN NO PASA ({} hallazgo(s)):\n  {}\n\n\
         Un fallo aquí NO se arregla subiendo la cota. Significa que el camino de coma flotante y \
         el exacto han dejado de ser la misma simulación, y hasta saber POR QUÉ ninguna cifra de \
         Monte Carlo describe el modelo que la app publica.",
        failures.len(),
        failures.join("\n  ")
    );
}

/// **Volatilidad cero degenera en el camino determinista, bit a bit.**
///
/// Es el control del gancho de WP6: si inyectar los multiplicadores DETERMINISTAS por
/// `growth_overrides` no reprodujera exactamente [`simulate_f64`], el hueco estaría cambiando el
/// modelo antes incluso de que exista el sorteo, y ninguna banda de percentiles significaría lo
/// que dice significar.
#[test]
fn mc_zero_volatility_degenerates() {
    for case in all_cases() {
        let plain = match simulate_f64(&case.input) {
            Ok(o) => o,
            Err(_) => continue,
        };
        let multipliers = deterministic_growth_multipliers(&case.input);
        assert_eq!(
            multipliers.len(),
            case.input.horizon_months as usize,
            "{}: una fila por mes simulado",
            case.name
        );
        let injected = simulate_f64_with_multipliers(&case.input, &multipliers)
            .expect("con los multiplicadores deterministas la simulación no puede fallar");
        assert_eq!(
            plain.net_worth, injected.net_worth,
            "{}: net_worth debe ser BIT A BIT el mismo",
            case.name
        );
        assert_eq!(
            plain.liquid_worth, injected.liquid_worth,
            "{}: liquid_worth debe ser BIT A BIT el mismo",
            case.name
        );
        assert_eq!(
            plain.contributed_capital, injected.contributed_capital,
            "{}: contributed_capital debe ser BIT A BIT el mismo",
            case.name
        );
        assert_eq!(
            plain.per_asset_series, injected.per_asset_series,
            "{}: las series por activo deben ser BIT A BIT las mismas",
            case.name
        );
        assert_eq!(
            plain.retirement_month_index, injected.retirement_month_index,
            "{}: el mes de jubilación no puede moverse",
            case.name
        );
        assert_eq!(
            plain.assets_depleted_month_index, injected.assets_depleted_month_index,
            "{}: el mes de agotamiento no puede moverse",
            case.name
        );
    }
}

/// Una fila de overrides mal dimensionada NO panica: ese mes cae al multiplicador determinista.
/// El motor es una función pura y su firma admite cualquier vector.
#[test]
fn a_badly_sized_override_row_falls_back_instead_of_panicking() {
    let case = all_cases()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 existe");
    let mut multipliers = deterministic_growth_multipliers(&case.input);
    multipliers[10].truncate(1); // fila con menos factores que activos
    let out = simulate_f64_with_multipliers(&case.input, &multipliers)
        .expect("no panica y no falla");
    let plain = simulate_f64(&case.input).expect("el determinista tampoco falla");
    assert_eq!(
        out.net_worth, plain.net_worth,
        "la fila inservible se ignora y ese mes usa el multiplicador de siempre"
    );
}
