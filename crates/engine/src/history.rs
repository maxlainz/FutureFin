//! Interpolación pura de la **serie histórica** de patrimonio a partir de snapshots manuales.
//!
//! El motor no hace I/O ni conoce el reloj: recibe una [`HistoryTimeline`] (fechas de snapshot
//! ascendentes — la última puede ser una observación «hoy» virtual que añade el llamante; el
//! motor no sabe ni le importa cuáles son virtuales) y una rejilla de fechas (`grid_dates`,
//! normalmente los primeros-de-mes con `month_index ≤ 0`), y devuelve, por item, la serie de
//! valores evaluada en cada punto de la rejilla.
//!
//! Reglas de evaluación (plan §3.4): antes del primer snapshot → 0, salvo el punto de rejilla
//! del propio mes del primer snapshot (se «engancha» y evalúa en él). Dentro de un segmento
//! `[s_a, s_{a+1}]`: observado en ambos extremos → interpola (activos: lineal en días civiles;
//! pasivos: amortización francesa corregida por residuo, [`amortized_segment_value`]); observado
//! en un solo extremo → su valor exacto en su fecha de snapshot, 0 en el resto del segmento
//! (aparece/desaparece sin rampas inventadas); en ninguno → 0. Se garantiza **exactitud en cada
//! fecha de snapshot** (el total suma exactamente lo observado).
//!
//! Sin coma flotante nativa en ningún punto del módulo: sólo `rust_decimal::Decimal` +
//! `chrono::NaiveDate`.

use crate::EngineError;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::MathematicalOps;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Días medios de un mes del calendario gregoriano (365.2425 / 12 = 30.436875). Convierte el ancho
/// civil de un segmento en «meses de amortización» (`N = días_total / avg_month_days`).
fn avg_month_days() -> Decimal {
    Decimal::new(30_436_875, 6)
}

/// Tipo de item histórico. Los pasivos interpolan con curva de amortización francesa; los activos
/// linealmente en días civiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryItemKind {
    Asset,
    Liability,
}

/// Términos del préstamo copiados en la observación de un pasivo. `apr_percent` es la TAE nominal
/// (5 = 5 %/año); `monthly_payment` es la cuota mensual (el llamante ya normaliza `weekly → ×52/12`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoanTerms {
    pub apr_percent: Decimal,
    pub monthly_payment: Decimal,
}

/// Valor observado de un item en un snapshot concreto, con los términos del préstamo si es un
/// pasivo (los activos llevan `terms = None`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryObservation {
    pub value: Decimal,
    pub terms: Option<LoanTerms>,
}

/// Un item (`source_item_id`) a lo largo del timeline. `observations[j]` es la observación en el
/// snapshot con fecha `HistoryTimeline::dates[j]` (o `None` si el item no aparece en ese snapshot).
/// Un vector más corto que `dates` se trata como `None` en los índices que falten.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub source_item_id: Uuid,
    pub kind: HistoryItemKind,
    pub observations: Vec<Option<HistoryObservation>>,
}

/// Un timeline es el conjunto de snapshots de un `(owner_user_id, kind)`: fechas **estrictamente
/// ascendentes** (la última puede ser una observación «hoy» virtual) compartidas por todos los
/// items, más los propios items con sus observaciones paralelas a `dates`.
#[derive(Debug, Clone)]
pub struct HistoryTimeline {
    pub dates: Vec<NaiveDate>,
    pub items: Vec<HistoryItem>,
}

/// Suma (firmada) de meses sobre una fecha con semántica **primero-de-mes**: el resultado es
/// siempre el día 1 del mes destino. Aritmética por meses-totales robusta para `delta` negativos
/// (usa `div_euclid`/`rem_euclid`, no requiere clamping porque el día resultante es siempre 1).
pub fn add_months_signed(date: NaiveDate, delta: i32) -> NaiveDate {
    let base = date.year() * 12 + (date.month() as i32 - 1);
    let total = base + delta;
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) + 1; // 1..=12
    NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap_or(date)
}

/// Índice de mes firmado de `date` respecto a un ancla primero-de-mes:
/// `(y2 − y1) · 12 + (m2 − m1)`. `0` en el propio mes del ancla, negativo hacia el pasado.
pub fn month_index_of(date: NaiveDate, anchor_month_first: NaiveDate) -> i32 {
    (date.year() - anchor_month_first.year()) * 12
        + (date.month() as i32 - anchor_month_first.month() as i32)
}

/// Interpolación lineal en días civiles entre `(0, v_a)` y `(days_total, v_b)`, evaluada a
/// `days_from_start`. `days_from_start = 0 → v_a` y `days_from_start = days_total → v_b` exactos.
fn interpolate_linear(v_a: Decimal, v_b: Decimal, days_from_start: i64, days_total: i64) -> Decimal {
    if days_total <= 0 {
        return v_a;
    }
    let f = Decimal::from(days_from_start.clamp(0, days_total)) / Decimal::from(days_total);
    v_a + f * (v_b - v_a)
}

/// Valor de un pasivo dentro de un segmento `(P_a) → (P_b)`, con curva de amortización francesa
/// **corregida por residuo** (plan §4). Con `terms` resueltos (los de la observación inicial, con
/// fallback a la final; el llamante ya aplica ese `or`):
///
/// - `i = apr/1200`, `u = 1 + i`, `f = days_from_start / days_total`, `N = days_total / 30.436875`,
///   `x = f·N`.
/// - `theo(y) = P_a·u^y − M·(u^y − 1)/i`  (con `checked_powd` de la feature `maths`),
///   `theo_c(y) = max(theo(y), 0)`.
/// - `P(g) = max( theo_c(x) + f·(P_b − theo_c(N)), 0 )`.
///
/// La corrección residual `f·(P_b − theo_c(N))` hace que los extremos sean **exactos**:
/// `f = 0 → P_a`, `f = 1 → P_b`, con independencia del error de `powd` (en `f = 1` el término
/// `theo_c(N)` se cancela; en `f = 0` el sumando residual es 0 y `theo_c(0) = P_a`).
///
/// Cae a **interpolación lineal** cuando: `terms` es `None`, `i ≤ 0`, `M ≤ 0`, `M ≤ P_a·i`
/// (la cuota no cubre ni el interés), o cualquier operación `checked_*` falla.
pub fn amortized_segment_value(
    p_a: Decimal,
    p_b: Decimal,
    terms: Option<&LoanTerms>,
    days_from_start: i64,
    days_total: i64,
) -> Decimal {
    fn linear(p_a: Decimal, p_b: Decimal, f: Decimal) -> Decimal {
        (p_a + f * (p_b - p_a)).max(Decimal::ZERO)
    }

    let dt = Decimal::from(days_total.max(0));
    let ds = Decimal::from(days_from_start.clamp(0, days_total.max(0)));
    // `f` estructuralmente exacto en los extremos: `ds = 0 → 0`, `ds = dt → 1`.
    let f = if dt.is_zero() {
        Decimal::ZERO
    } else {
        ds / dt
    };

    let Some(terms) = terms else {
        return linear(p_a, p_b, f);
    };
    let i = terms.apr_percent / Decimal::from(1200);
    let m = terms.monthly_payment;
    if dt.is_zero() || i <= Decimal::ZERO || m <= Decimal::ZERO || m <= p_a * i {
        return linear(p_a, p_b, f);
    }

    let n = dt / avg_month_days();
    let x = f * n;
    let u = Decimal::ONE + i;

    // Todo el cálculo transcendental es `checked_*`; a la mínima señal de fallo → lineal.
    let amort = (|| -> Option<Decimal> {
        // `u^0 = 1` estructural (evita cualquier error de `powd` en `f = 0`).
        let u_pow_x = if x.is_zero() {
            Decimal::ONE
        } else {
            u.checked_powd(x)?
        };
        let u_pow_n = u.checked_powd(n)?;
        let theo = |u_pow: Decimal| -> Option<Decimal> {
            let grow = p_a.checked_mul(u_pow)?;
            let interest = m
                .checked_mul(u_pow.checked_sub(Decimal::ONE)?)?
                .checked_div(i)?;
            grow.checked_sub(interest)
        };
        let theo_c_x = theo(u_pow_x)?.max(Decimal::ZERO);
        let theo_c_n = theo(u_pow_n)?.max(Decimal::ZERO);
        let residual = f.checked_mul(p_b.checked_sub(theo_c_n)?)?;
        Some(theo_c_x.checked_add(residual)?.max(Decimal::ZERO))
    })();

    amort.unwrap_or_else(|| linear(p_a, p_b, f))
}

/// Evalúa un único item sobre la rejilla, en el punto `g`, según las reglas del plan §3.4.
fn evaluate_item_at(dates: &[NaiveDate], item: &HistoryItem, g: NaiveDate) -> Decimal {
    let m = dates.len();
    if m == 0 {
        return Decimal::ZERO;
    }

    // Fecha de evaluación efectiva: `g`, salvo el «enganche» del primer mes visible.
    let e = if g < dates[0] {
        if crate::projection::month_first_calendar(dates[0]) <= g {
            // El punto de rejilla cae en el propio mes del primer snapshot: se evalúa en él.
            dates[0]
        } else {
            // Estrictamente antes del primer snapshot: aún no existe nada.
            return Decimal::ZERO;
        }
    } else {
        g
    };

    // Tras el último snapshot (sólo posible si el llamante no añadió «hoy» virtual): 0.
    if e > dates[m - 1] {
        return Decimal::ZERO;
    }

    // `a` = mayor índice con `dates[a] <= e`. Como `dates[0] <= e <= dates[m-1]`, `a` es válido.
    let a = match dates.binary_search(&e) {
        Ok(idx) => idx,
        Err(idx) => idx.saturating_sub(1),
    };

    let obs_at = |j: usize| item.observations.get(j).and_then(|o| o.as_ref());

    if a == m - 1 {
        // `e` coincide con el último snapshot (o timeline de un solo snapshot): valor exacto o 0.
        return obs_at(a).map(|o| o.value).unwrap_or(Decimal::ZERO);
    }

    // Segmento `[dates[a], dates[a+1]]` con `dates[a] <= e < dates[a+1]`.
    let d_a = dates[a];
    let d_b = dates[a + 1];
    let days_total = (d_b - d_a).num_days();
    let days_from_start = (e - d_a).num_days();

    match (obs_at(a), obs_at(a + 1)) {
        (Some(lo), Some(ro)) => match item.kind {
            HistoryItemKind::Asset => {
                interpolate_linear(lo.value, ro.value, days_from_start, days_total)
            }
            HistoryItemKind::Liability => {
                let terms = lo.terms.as_ref().or(ro.terms.as_ref());
                amortized_segment_value(lo.value, ro.value, terms, days_from_start, days_total)
            }
        },
        // Observado sólo en el extremo izquierdo: su valor exacto en su fecha, 0 en el resto.
        (Some(lo), None) => {
            if e == d_a {
                lo.value
            } else {
                Decimal::ZERO
            }
        }
        // Observado sólo en el extremo derecho: como `e < d_b` en este segmento, aquí siempre 0
        // (el valor de `d_b` se sirve cuando la rejilla lo alcanza, con `a = a+1`).
        (None, Some(ro)) => {
            if e == d_b {
                ro.value
            } else {
                Decimal::ZERO
            }
        }
        (None, None) => Decimal::ZERO,
    }
}

/// Evalúa el timeline sobre `grid_dates`, devolviendo una serie por item (paralela a
/// `timeline.items`), cada una paralela a `grid_dates`. Reglas del plan §3.4 (ver módulo).
///
/// Valida que las fechas del timeline sean **estrictamente ascendentes**; si no,
/// [`EngineError::InvalidHistoryTimeline`].
pub fn evaluate_timeline(
    timeline: &HistoryTimeline,
    grid_dates: &[NaiveDate],
) -> Result<Vec<Vec<Decimal>>, EngineError> {
    for w in timeline.dates.windows(2) {
        if w[0] >= w[1] {
            return Err(EngineError::InvalidHistoryTimeline);
        }
    }

    let mut out: Vec<Vec<Decimal>> = Vec::with_capacity(timeline.items.len());
    for item in &timeline.items {
        let mut series = Vec::with_capacity(grid_dates.len());
        for &g in grid_dates {
            series.push(evaluate_item_at(&timeline.dates, item, g));
        }
        out.push(series);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::MathematicalOps;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn dec(v: i64) -> Decimal {
        Decimal::from(v)
    }

    fn terms(apr: i64, pay: i64) -> LoanTerms {
        LoanTerms {
            apr_percent: dec(apr),
            monthly_payment: dec(pay),
        }
    }

    fn obs(v: i64) -> Option<HistoryObservation> {
        Some(HistoryObservation {
            value: dec(v),
            terms: None,
        })
    }

    fn obs_liab(v: i64, apr: i64, pay: i64) -> Option<HistoryObservation> {
        Some(HistoryObservation {
            value: dec(v),
            terms: Some(terms(apr, pay)),
        })
    }

    fn item(kind: HistoryItemKind, observations: Vec<Option<HistoryObservation>>) -> HistoryItem {
        HistoryItem {
            source_item_id: Uuid::from_u128(1),
            kind,
            observations,
        }
    }

    /// N (meses) del ancho civil `days_total`.
    fn n_months(days_total: i64) -> Decimal {
        Decimal::from(days_total) / Decimal::new(30_436_875, 6)
    }

    /// Balance francés teórico (forma factorizada, algebraicamente distinta de la del motor):
    /// `B(x) = (P_a − M/i)·u^x + M/i`. Referencia independiente para los tests.
    fn theo_balance(p_a: Decimal, apr: i64, pay: i64, x: Decimal) -> Decimal {
        let i = dec(apr) / dec(1200);
        let u = Decimal::ONE + i;
        let c = dec(pay) / i;
        (p_a - c) * u.powd(x) + c
    }

    // ---- Interpolación de activos (lineal en días) -------------------------------------------

    #[test]
    fn asset_linear_midpoint() {
        let tl = HistoryTimeline {
            dates: vec![d(2025, 1, 1), d(2025, 1, 11)],
            items: vec![item(HistoryItemKind::Asset, vec![obs(1000), obs(2000)])],
        };
        let grid = vec![d(2025, 1, 1), d(2025, 1, 6), d(2025, 1, 11)];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        // 5/10 del camino → punto medio exacto.
        assert_eq!(out[0][0], dec(1000));
        assert_eq!(out[0][1], dec(1500));
        assert_eq!(out[0][2], dec(2000));
    }

    #[test]
    fn endpoint_exactness_at_snapshot_dates() {
        // Directo: los extremos de la amortización son exactos aunque `powd` sea aproximado.
        let t = terms(6, 1500);
        assert_eq!(
            amortized_segment_value(dec(200_000), dec(190_000), Some(&t), 0, 180),
            dec(200_000)
        );
        assert_eq!(
            amortized_segment_value(dec(200_000), dec(190_000), Some(&t), 180, 180),
            dec(190_000)
        );

        // Vía timeline: un pasivo observado en tres snapshots, rejilla = fechas de snapshot.
        let tl = HistoryTimeline {
            dates: vec![d(2024, 1, 1), d(2024, 7, 1), d(2025, 1, 1)],
            items: vec![item(
                HistoryItemKind::Liability,
                vec![
                    obs_liab(200_000, 5, 1500),
                    obs_liab(195_000, 5, 1500),
                    obs_liab(189_000, 5, 1500),
                ],
            )],
        };
        let out = evaluate_timeline(&tl, &tl.dates).unwrap();
        assert_eq!(out[0][0], dec(200_000));
        assert_eq!(out[0][1], dec(195_000));
        assert_eq!(out[0][2], dec(189_000));
    }

    // ---- Amortización francesa ---------------------------------------------------------------

    #[test]
    fn amortization_matches_pure_french_schedule() {
        // Residuo 0: `P_b` es el balance teórico a `N` meses → resultado = curva francesa pura.
        let p_a = dec(200_000);
        let (apr, pay) = (5i64, 1200i64);
        let days_total = 3653i64; // ≈ 10 años
        let n = n_months(days_total);
        let p_b = theo_balance(p_a, apr, pay, n).max(Decimal::ZERO);
        let t = terms(apr, pay);

        // Coincide (dentro de tolerancia de `powd`) con el balance francés recomputado.
        for &ds in &[0i64, 900, 1800, 2700, 3653] {
            let f = Decimal::from(ds) / Decimal::from(days_total);
            let x = f * n;
            let expected = theo_balance(p_a, apr, pay, x).max(Decimal::ZERO);
            let got = amortized_segment_value(p_a, p_b, Some(&t), ds, days_total);
            let diff = (got - expected).abs();
            assert!(
                diff < Decimal::ONE,
                "ds={ds}: got={got}, expected={expected}, diff={diff}"
            );
        }

        // Estrictamente decreciente.
        let vals: Vec<Decimal> = [0i64, 900, 1800, 2700, 3653]
            .iter()
            .map(|&ds| amortized_segment_value(p_a, p_b, Some(&t), ds, days_total))
            .collect();
        for w in vals.windows(2) {
            assert!(w[0] > w[1], "no decreciente: {} !> {}", w[0], w[1]);
        }
    }

    #[test]
    fn amortization_residual_correction_passes_both_endpoints() {
        // `P_b` arbitrario (no el balance teórico) → residuo grande, pero los extremos siguen exactos.
        let t = terms(5, 1200);
        let p_a = dec(200_000);
        let p_b = dec(50_000);
        let days_total = 3653i64;
        assert_eq!(
            amortized_segment_value(p_a, p_b, Some(&t), 0, days_total),
            p_a
        );
        assert_eq!(
            amortized_segment_value(p_a, p_b, Some(&t), days_total, days_total),
            p_b
        );
        // A mitad, el valor cae entre ambos (correlación sana con el residuo).
        let mid = amortized_segment_value(p_a, p_b, Some(&t), days_total / 2, days_total);
        assert!(mid < p_a && mid > p_b, "mid={mid} fuera de [{p_b}, {p_a}]");
    }

    #[test]
    fn amortization_balance_sits_above_linear_chord() {
        // Curva francesa (residuo 0) es convexa decreciente → por encima de la cuerda lineal.
        let p_a = dec(200_000);
        let (apr, pay) = (5i64, 1200i64);
        let days_total = 3653i64;
        let n = n_months(days_total);
        let p_b = theo_balance(p_a, apr, pay, n).max(Decimal::ZERO);
        let t = terms(apr, pay);

        let mid = days_total / 2;
        let amort_mid = amortized_segment_value(p_a, p_b, Some(&t), mid, days_total);
        let f = Decimal::from(mid) / Decimal::from(days_total);
        let linear_mid = p_a + f * (p_b - p_a);
        assert!(
            amort_mid > linear_mid,
            "amort_mid={amort_mid} !> linear_mid={linear_mid}"
        );
    }

    #[test]
    fn fallback_linear_when_payment_leq_interest() {
        // apr 6 % → i = 0.005; P_a·i = 500. Cuota 400 ≤ 500 → no cubre interés → lineal.
        let t = terms(6, 400);
        let p_a = dec(100_000);
        let p_b = dec(90_000);
        let mid = amortized_segment_value(p_a, p_b, Some(&t), 50, 100);
        // Lineal exacto en el punto medio: 100000 + 0.5·(90000 − 100000) = 95000.
        assert_eq!(mid, dec(95_000));
    }

    #[test]
    fn fallback_linear_when_terms_missing() {
        let p_a = dec(100_000);
        let p_b = dec(90_000);
        // Sin términos → lineal exacto.
        assert_eq!(
            amortized_segment_value(p_a, p_b, None, 50, 100),
            dec(95_000)
        );
        // apr = 0 → i ≤ 0 → lineal exacto.
        let t0 = terms(0, 1500);
        assert_eq!(
            amortized_segment_value(p_a, p_b, Some(&t0), 50, 100),
            dec(95_000)
        );
    }

    #[test]
    fn clamp_never_negative_and_endpoints_still_exact() {
        // Cuota enorme → la curva teórica cruza a negativo a mitad de segmento; el resultado se
        // clampa a 0 pero los extremos (P_a y P_b) siguen exactos.
        let t = terms(5, 10_000);
        let p_a = dec(1_000);
        let p_b = dec(0);
        let days_total = 304i64; // N ≈ 10 meses
        assert_eq!(
            amortized_segment_value(p_a, p_b, Some(&t), 0, days_total),
            p_a
        );
        assert_eq!(
            amortized_segment_value(p_a, p_b, Some(&t), days_total, days_total),
            p_b
        );
        // Puntos intermedios nunca negativos.
        for ds in [30i64, 76, 152, 228, 274] {
            let v = amortized_segment_value(p_a, p_b, Some(&t), ds, days_total);
            assert!(v >= Decimal::ZERO, "negativo en ds={ds}: {v}");
        }
    }

    // ---- Reglas de timeline por item ---------------------------------------------------------

    #[test]
    fn item_missing_from_middle_snapshot_is_zero_between_pairs() {
        // Observado en el 1º y 3º snapshot, ausente en el intermedio → 0 entre los pares.
        let tl = HistoryTimeline {
            dates: vec![d(2024, 1, 1), d(2024, 6, 1), d(2024, 12, 1)],
            items: vec![item(
                HistoryItemKind::Asset,
                vec![obs(1000), None, obs(3000)],
            )],
        };
        let grid = vec![
            d(2024, 1, 1),  // d0 → observado (extremo izq de su fecha)
            d(2024, 3, 1),  // entre d0 y d1, sólo obs en d0 → 0
            d(2024, 6, 1),  // d1 (no observado) → 0
            d(2024, 9, 1),  // entre d1 y d2, sólo obs en d2 → 0
            d(2024, 12, 1), // d2 → observado
        ];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        assert_eq!(out[0], vec![dec(1000), dec(0), dec(0), dec(0), dec(3000)]);
    }

    #[test]
    fn item_only_in_one_snapshot_appears_then_disappears() {
        // Observado sólo en el snapshot intermedio → 0 antes, su valor en su fecha, 0 después.
        let tl = HistoryTimeline {
            dates: vec![d(2024, 1, 1), d(2024, 6, 1), d(2024, 12, 1)],
            items: vec![item(HistoryItemKind::Asset, vec![None, obs(500), None])],
        };
        let grid = vec![
            d(2024, 1, 1),
            d(2024, 3, 1),
            d(2024, 6, 1),
            d(2024, 9, 1),
            d(2024, 12, 1),
        ];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        assert_eq!(out[0], vec![dec(0), dec(0), dec(500), dec(0), dec(0)]);
    }

    #[test]
    fn first_month_clamp_evaluates_at_first_snapshot() {
        // Primer snapshot a mitad de mes; el punto de rejilla de ese mismo mes engancha en él.
        let tl = HistoryTimeline {
            dates: vec![d(2025, 3, 15), d(2025, 6, 1)],
            items: vec![item(HistoryItemKind::Asset, vec![obs(1000), obs(1600)])],
        };
        let grid = vec![d(2025, 2, 1), d(2025, 3, 1), d(2025, 4, 1)];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        assert_eq!(out[0][0], dec(0)); // mes anterior al primer snapshot → 0
        assert_eq!(out[0][1], dec(1000)); // mes del primer snapshot → valor observado exacto
        assert!(out[0][2] > dec(1000) && out[0][2] < dec(1600)); // ya interpolando
    }

    #[test]
    fn virtual_today_join_and_deleted_item_goes_zero() {
        // Timeline [pasado, hoy-virtual]. Un item vivo se une a su valor de hoy; un item borrado
        // (ausente en el snapshot virtual de hoy) cae a 0 en el punto de rejilla del mes 0.
        let today = d(2025, 6, 10);
        let tl = HistoryTimeline {
            dates: vec![d(2025, 1, 1), today],
            items: vec![
                HistoryItem {
                    source_item_id: Uuid::from_u128(1),
                    kind: HistoryItemKind::Asset,
                    observations: vec![obs(1000), obs(1600)], // vivo en ambos
                },
                HistoryItem {
                    source_item_id: Uuid::from_u128(2),
                    kind: HistoryItemKind::Asset,
                    observations: vec![obs(500), None], // borrado antes de hoy
                },
            ],
        };
        let anchor = crate::projection::month_first_calendar(today); // 2025-06-01 (grid k = 0)
        let grid = vec![d(2025, 1, 1), anchor];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        // Vivo: se une hacia su valor de hoy (interpolado, cerca de 1600).
        assert_eq!(out[0][0], dec(1000));
        assert!(out[0][1] > dec(1500));
        // Borrado: exacto en su snapshot, 0 en k=0.
        assert_eq!(out[1][0], dec(500));
        assert_eq!(out[1][1], dec(0));
    }

    // ---- Aritmética de calendario ------------------------------------------------------------

    #[test]
    fn month_index_and_add_months_signed_negative_cases() {
        let anchor = d(2026, 7, 1);
        // k = 0
        assert_eq!(add_months_signed(anchor, 0), anchor);
        assert_eq!(month_index_of(anchor, anchor), 0);
        // k = -13 (cruza dos años)
        assert_eq!(add_months_signed(anchor, -13), d(2025, 6, 1));
        assert_eq!(month_index_of(d(2025, 6, 1), anchor), -13);
        // Frontera de año: enero − 1 mes = diciembre del año anterior.
        assert_eq!(add_months_signed(d(2026, 1, 1), -1), d(2025, 12, 1));
        assert_eq!(month_index_of(d(2025, 12, 1), anchor), -7);
        // Ida y vuelta para varios k negativos.
        for k in [-1i32, -5, -12, -13, -24, -37] {
            let back = add_months_signed(anchor, k);
            assert_eq!(month_index_of(back, anchor), k, "round-trip k={k}");
        }
    }

    #[test]
    fn non_ascending_timeline_dates_rejected() {
        // Descendente.
        let tl = HistoryTimeline {
            dates: vec![d(2025, 6, 1), d(2025, 1, 1)],
            items: vec![item(HistoryItemKind::Asset, vec![obs(1), obs(2)])],
        };
        assert!(matches!(
            evaluate_timeline(&tl, &[d(2025, 3, 1)]),
            Err(EngineError::InvalidHistoryTimeline)
        ));
        // Fechas iguales (no estrictamente ascendentes) también se rechazan.
        let tl_eq = HistoryTimeline {
            dates: vec![d(2025, 1, 1), d(2025, 1, 1)],
            items: vec![item(HistoryItemKind::Asset, vec![obs(1), obs(2)])],
        };
        assert!(matches!(
            evaluate_timeline(&tl_eq, &[d(2025, 1, 1)]),
            Err(EngineError::InvalidHistoryTimeline)
        ));
    }
}
