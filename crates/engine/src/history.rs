//! Interpolación pura de la **serie histórica** de patrimonio a partir de snapshots manuales.
//!
//! El motor no hace I/O ni conoce el reloj: recibe una [`HistoryTimeline`] (fechas de snapshot
//! ascendentes — la última puede ser una observación «hoy» virtual que añade el llamante; el
//! motor no sabe ni le importa cuáles son virtuales) y una rejilla de fechas (`grid_dates`,
//! normalmente los primeros-de-mes con `month_index ≤ 0`), y devuelve, por item, la serie de
//! valores evaluada en cada punto de la rejilla.
//!
//! Reglas de evaluación (plan §3.4, revisadas en 4.7.0): antes del primer snapshot → 0, salvo
//! el punto de rejilla del propio mes del primer snapshot (se «engancha» y evalúa en él).
//! Dentro de un segmento `[s_a, s_{a+1}]`: observado en ambos extremos → interpola (activos:
//! lineal en días civiles o anclada a cash-flow; pasivos: por la ley del MODELO capturado,
//! [`amortized_segment_value`], #129); observado en un solo extremo → se ARRASTRA el último
//! valor observado (LOCF, #130) — vale cero la ausencia del ledger vivo (`last_is_live_ledger`:
//! borrado/vendido de verdad) y el punto EXACTO de la última fecha del timeline (rama
//! `a == m−1`, también sobre una captura manual final que omita el item). Se garantiza
//! **exactitud en cada fecha de snapshot presente** (el total suma exactamente lo observado).
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

/// Términos del préstamo copiados en la observación de un pasivo. `apr_percent` es el TIN nominal
/// (5 = 5 %/año); `monthly_payment` es la cuota mensual (el llamante ya normaliza `weekly → ×52/12`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoanTerms {
    pub apr_percent: Decimal,
    pub monthly_payment: Decimal,
    /// Modelo de amortización que tenía el pasivo CUANDO SE CAPTURÓ el snapshot (#129, 4.7.0).
    /// `None` = snapshot anterior: se reinterpreta como el default de entonces
    /// (`fixed_payments`) ⇒ ley LINEAL, que es exactamente la curva que ese snapshot describía.
    #[serde(default)]
    pub repayment_model: Option<crate::projection::RepaymentModel>,
}

/// Valor observado de un item en un snapshot concreto, con los términos del préstamo si es un
/// pasivo (los activos llevan `terms = None`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryObservation {
    pub value: Decimal,
    pub terms: Option<LoanTerms>,
}

/// Serde de `NaiveDate` como cadena ISO-8601 `YYYY-MM-DD`. El motor declara chrono con
/// `default-features = false` (sólo `alloc`, sin la feature `serde` — decisión de pureza), así que
/// la conversión vive aquí, sin ampliar la superficie de dependencias/features del crate puro y sin
/// depender del formateador de chrono: sólo `Datelike` + `from_ymd_opt`.
mod date_ymd {
    use chrono::{Datelike, NaiveDate};
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(date: &NaiveDate, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month(),
            date.day()
        ))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDate, D::Error> {
        let s = String::deserialize(d)?;
        let mut parts = s.splitn(3, '-');
        let y = parts.next().and_then(|p| p.parse::<i32>().ok());
        let m = parts.next().and_then(|p| p.parse::<u32>().ok());
        let day = parts.next().and_then(|p| p.parse::<u32>().ok());
        match (y, m, day) {
            (Some(y), Some(m), Some(day)) => NaiveDate::from_ymd_opt(y, m, day)
                .ok_or_else(|| D::Error::custom("fecha fuera de rango")),
            _ => Err(D::Error::custom("se esperaba YYYY-MM-DD")),
        }
    }
}

/// Un movimiento de cash-flow datado que **moldea** la curva de un activo dentro de su segmento
/// (tier-2: nunca contradice los snapshots — la curva anclada pasa exacta por ellos, decisión 12).
/// `delta` viene YA normalizado en signo por el llamante: **positivo sube** el valor del activo
/// (pata cuenta = `+amount`; pata destino de un ahorro = `−amount`). El motor no interpreta signos
/// ni fuentes: sólo suma `delta` en el intervalo semiabierto `(seg_start, ·]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashFlowEntry {
    #[serde(with = "date_ymd")]
    pub date: NaiveDate,
    pub delta: Decimal,
}

/// Un item (`source_item_id`) a lo largo del timeline. `observations[j]` es la observación en el
/// snapshot con fecha `HistoryTimeline::dates[j]` (o `None` si el item no aparece en ese snapshot).
/// Un vector más corto que `dates` se trata como `None` en los índices que falten.
///
/// `cashflow` (opcional, `#[serde(default)]`) son los movimientos datados que moldean la curva del
/// item **entre** snapshots. Vacío ⇒ comportamiento idéntico al histórico previo (interpolación
/// lineal / amortización francesa, bit a bit). Sólo se consulta en el brazo activo-observado-en-
/// ambos-extremos; los pasivos y los items observados en un solo extremo lo ignoran (fase 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub source_item_id: Uuid,
    pub kind: HistoryItemKind,
    pub observations: Vec<Option<HistoryObservation>>,
    #[serde(default)]
    pub cashflow: Vec<CashFlowEntry>,
}

/// Un timeline es el conjunto de snapshots de un `(owner_user_id, kind)`: fechas **estrictamente
/// ascendentes** (la última puede ser una observación «hoy» virtual) compartidas por todos los
/// items, más los propios items con sus observaciones paralelas a `dates`.
#[derive(Debug, Clone)]
pub struct HistoryTimeline {
    pub dates: Vec<NaiveDate>,
    pub items: Vec<HistoryItem>,
    /// ¿El ÚLTIMO punto de `dates` es la lectura del ledger vivo («hoy» virtual)? (#130). Un
    /// item ausente ahí está BORRADO/vendido — la única ausencia que significa cero. En
    /// cualquier otro punto, ausente = «esta captura no lo incluyó» y el valor se arrastra
    /// (LOCF) hasta la siguiente observación.
    pub last_is_live_ledger: bool,
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

/// Valor de un **activo** dentro de un segmento `[seg_start, seg_end]` **anclado** a los deltas de
/// cash-flow (tier-2), con la curva pasando **exacta** por ambos snapshots:
///
/// `v(t) = Va + C(a→t) + f(t)·(Vb − Va − C_total)`
///
/// donde:
/// - `C(a→t)` = Σ de `delta` de las entradas en el intervalo **semiabierto** `(seg_start, eval_date]`
///   (una txn fechada en `seg_start` pertenece al segmento anterior; una fechada en `seg_end` sí
///   cuenta — misma frontera que usa el brazo `evaluate_item_at` para elegir esta rama).
/// - `C_total = C(a→b)` = Σ de `delta` en `(seg_start, seg_end]`.
/// - `f(t) = days_from_start / days_total`, lineal en días civiles — **idéntica base** que
///   [`interpolate_linear`] (mismo `clamp` y misma división).
///
/// **Exactitud en los extremos** (independiente del cash-flow):
/// - `f = 0` (⇒ `days_from_start = 0`, `eval_date = seg_start`): `C(a→a) = 0` (intervalo vacío) y el
///   sumando residual es `0` ⇒ `v = Va` **exacto**.
/// - `f = 1` (⇒ `days_from_start = days_total`, `eval_date = seg_end`): `C(a→b) = C_total` y
///   `v = Va + C_total + (Vb − Va − C_total) = Vb` **exacto** (sin división residual; `f = n/n = 1`).
///
/// Con `cashflow` vacío degenera a `Va + f·(Vb − Va)` = [`interpolate_linear`]; aun así, el llamante
/// evita esta función cuando no hay deltas en `(seg_start, seg_end]` y usa `interpolate_linear`
/// textual, para garantía de **identidad bit a bit** con el histórico previo (propiedad P3).
///
/// **Implementación:** barrido lineal `O(n)` sobre `cf` por punto de evaluación (sin asumir orden,
/// sin sumas-prefijo). Elegido frente a búsqueda binaria / sumas-prefijo porque la firma recibe `cf`
/// y `eval_date` **por punto** (encaja en el diseño per-punto de `evaluate_item_at`, sin estado
/// per-item), el volumen es diminuto (decenas de entradas × decenas de meses, sub-ms como el resto
/// del módulo) y el barrido es robusto a cualquier orden de entrada (una búsqueda binaria exigiría
/// un contrato «ordenado» que fallaría en silencio si se incumpliera). Sin `f64`.
pub fn anchored_cashflow_segment_value(
    v_a: Decimal,
    v_b: Decimal,
    cf: &[CashFlowEntry],
    seg_start: NaiveDate,
    seg_end: NaiveDate,
    eval_date: NaiveDate,
    days_from_start: i64,
    days_total: i64,
) -> Decimal {
    if days_total <= 0 {
        return v_a;
    }
    let f = Decimal::from(days_from_start.clamp(0, days_total)) / Decimal::from(days_total);

    // `C_total` sobre `(seg_start, seg_end]` y `C(a→t)` sobre `(seg_start, eval_date]`, ambos en un
    // solo barrido. Intervalo semiabierto por la izquierda: `date > seg_start` (nunca `>=`).
    let mut c_partial = Decimal::ZERO;
    let mut c_total = Decimal::ZERO;
    for entry in cf {
        if entry.date > seg_start && entry.date <= seg_end {
            c_total += entry.delta;
            if entry.date <= eval_date {
                c_partial += entry.delta;
            }
        }
    }

    v_a + c_partial + f * (v_b - v_a - c_total)
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
    // #129: la ley la elige el MODELO capturado, no una francesa universal.
    // - French → recurrencia compuesta (abajo), exacta en los extremos.
    // - Revolving → la CUERDA (verificación adversarial de la ola): su caja real es
    //   max(pct·saldo, suelo) y el snapshot solo guarda la cuota DECLARADA, que desde #144 no
    //   gobierna nada — la curva compuesta con esa cuota producía una V no monótona con errores
    //   interiores de hasta −70 %. Sin los mínimos en la foto, la cuerda es lo único honesto.
    // - FixedPayments → lineal EXACTA, no aproximada: la cuota va íntegra a principal, la
    //   pendiente es constante (−M/mes) y la cuerda entre P_a y P_b ES la curva.
    // - InterestOnly → el principal es constante por contrato; cualquier diferencia entre
    //   extremos vino de algo que el modelo no conoce y la cuerda es la interpolación menos
    //   comprometida que pasa por ambos snapshots.
    // - None (snapshot pre-4.7.0) → no se sabe qué era; la cuerda es la ley menos comprometida.
    //   OJO, matiz honesto: lo que esos snapshots RENDERIZABAN hasta 4.6.0 era la curva francesa
    //   universal — para un pasivo genuinamente francés la curva vieja era la correcta y aquí
    //   pierde ~300 €/50 k€ de forma interior (los extremos siguen exactos). Es el precio de
    //   dejar de aplicarle esa misma curva al default mayoritario (fixed), donde era el bug.
    match terms.repayment_model {
        Some(crate::projection::RepaymentModel::French) => {}
        Some(crate::projection::RepaymentModel::Revolving)
        | Some(crate::projection::RepaymentModel::FixedPayments)
        | Some(crate::projection::RepaymentModel::InterestOnly)
        | None => return linear(p_a, p_b, f),
    }
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
fn evaluate_item_at(
    dates: &[NaiveDate],
    item: &HistoryItem,
    g: NaiveDate,
    last_is_live_ledger: bool,
) -> Decimal {
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
                // ÚNICA rama nueva: si hay algún movimiento de cash-flow en el intervalo
                // semiabierto `(d_a, d_b]`, la curva se ancla a ellos (pasa exacta por ambos
                // snapshots). Sin movimientos en el segmento ⇒ MISMA ruta de hoy
                // (`interpolate_linear` textual) para identidad bit a bit (P3).
                let has_cashflow = item
                    .cashflow
                    .iter()
                    .any(|entry| entry.date > d_a && entry.date <= d_b);
                if has_cashflow {
                    anchored_cashflow_segment_value(
                        lo.value,
                        ro.value,
                        &item.cashflow,
                        d_a,
                        d_b,
                        e,
                        days_from_start,
                        days_total,
                    )
                } else {
                    interpolate_linear(lo.value, ro.value, days_from_start, days_total)
                }
            }
            HistoryItemKind::Liability => {
                let terms = lo.terms.as_ref().or(ro.terms.as_ref());
                amortized_segment_value(lo.value, ro.value, terms, days_from_start, days_total)
            }
        },
        // INVERTIDO en 4.7.0 (#130). Hasta 4.6.0 la ausencia en un extremo valía 0 y una
        // captura que simplemente no incluyó un item desplomaba el agregado (−40.000 € falsos
        // en el escenario del issue). La regla nueva: ausente = «esta captura no lo incluyó» ⇒
        // se ARRASTRA el último valor observado (LOCF). La ÚNICA ausencia que significa cero es
        // la del ledger vivo («hoy» virtual): ahí el item está borrado/vendido de verdad.
        (Some(lo), None) => {
            let deleted_now = a + 1 == m - 1 && last_is_live_ledger;
            if deleted_now {
                if e == d_a {
                    lo.value
                } else {
                    Decimal::ZERO
                }
            } else {
                lo.value
            }
        }
        // `e < d_b` estricto en esta rama (el binary_search habría dado `a = a+1` con e == d_b):
        // se arrastra lo último observado en o antes de `a`; 0 si nunca se observó.
        (None, Some(_ro)) => carried(item, a),
        (None, None) => {
            let deleted_now = a + 1 == m - 1 && last_is_live_ledger;
            if deleted_now {
                Decimal::ZERO
            } else {
                carried(item, a)
            }
        }
    }
}

/// Último valor observado en o antes del índice `upto` (#130). Antes del primer snapshot no hay
/// nada que arrastrar ⇒ 0.
fn carried(item: &HistoryItem, upto: usize) -> Decimal {
    item.observations[..=upto]
        .iter()
        .rev()
        .find_map(|o| o.as_ref())
        .map(|o| o.value)
        .unwrap_or(Decimal::ZERO)
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
            series.push(evaluate_item_at(&timeline.dates, item, g, timeline.last_is_live_ledger));
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

    /// Términos de un snapshot que SABE que su pasivo era francés (#129): es lo que capturan
    /// las fotos desde 4.7.0 y lo que ejercitan los tests de la curva compuesta.
    fn terms(apr: i64, pay: i64) -> LoanTerms {
        LoanTerms {
            apr_percent: dec(apr),
            monthly_payment: dec(pay),
            repayment_model: Some(crate::projection::RepaymentModel::French),
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
            cashflow: vec![],
        }
    }

    fn cf_e(date: NaiveDate, delta: i64) -> CashFlowEntry {
        CashFlowEntry {
            date,
            delta: dec(delta),
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
            last_is_live_ledger: false,
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

    /// #129, a mano: P_a = 50.000 € (día 0) → P_b = 26.000 € (día 365), TIN 15 %, cuota 2.000.
    /// El MODELO capturado elige la ley del segmento:
    /// - `french` (y `revolving`): la curva compuesta de siempre — al día 182, 38.361,7468 €
    ///   (recurrencia theo + corrección residual, verificada aparte a 40 dígitos);
    /// - `fixed_payments`, `interest_only` y `None` (snapshot pre-4.7.0): la CUERDA exacta,
    ///   50.000 − 24.000·(182/365) = **38.032,8767 €**. Para cuota fija no es una aproximación:
    ///   la pendiente es constante y la cuerda ES la curva. Hasta 4.6.0 todo pasivo interpolaba
    ///   como francés (Δ ≈ 329 € aquí, y un quiebro de pendiente falso al llegar a «hoy»).
    #[test]
    fn the_captured_model_chooses_the_interpolation_law() {
        let base = terms(15, 2000); // repayment_model: Some(French)
        let french = amortized_segment_value(dec(50_000), dec(26_000), Some(&base), 182, 365);
        let expected_french: Decimal = "38361.7468".parse().unwrap();
        assert!(
            (french - expected_french).abs() < "0.01".parse::<Decimal>().unwrap(),
            "french: {french}"
        );

        let chord: Decimal = "38032.8767".parse().unwrap();
        for model in [
            Some(crate::projection::RepaymentModel::FixedPayments),
            Some(crate::projection::RepaymentModel::InterestOnly),
            // Revolving también por la cuerda: el snapshot no guarda sus mínimos y la cuota
            // declarada no gobierna su caja desde #144 (la compuesta daba una V no monótona).
            Some(crate::projection::RepaymentModel::Revolving),
            None,
        ] {
            let mut t = base.clone();
            t.repayment_model = model;
            let got = amortized_segment_value(dec(50_000), dec(26_000), Some(&t), 182, 365);
            assert_eq!(got.round_dp(4), chord, "{model:?} interpola por la cuerda");
        }
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
            last_is_live_ledger: false,
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
    fn item_missing_from_middle_snapshot_carries_its_last_value() {
        // INVERTIDO en 4.7.0 (#130): hasta 4.6.0 la ausencia en la captura intermedia valía 0
        // ([1000, 0, 0, 0, 3000]) — un item que simplemente no se incluyó en una foto desplomaba
        // el agregado. Ahora se ARRASTRA el último valor observado (LOCF) hasta la siguiente
        // observación; solo la ausencia en el ledger vivo significa cero (test de al lado).
        let tl = HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d(2024, 1, 1), d(2024, 6, 1), d(2024, 12, 1)],
            items: vec![item(
                HistoryItemKind::Asset,
                vec![obs(1000), None, obs(3000)],
            )],
        };
        let grid = vec![
            d(2024, 1, 1),  // d0 → observado
            d(2024, 3, 1),  // entre d0 y d1 → arrastra 1000
            d(2024, 6, 1),  // d1 (no observado) → arrastra 1000
            d(2024, 9, 1),  // entre d1 y d2 → sigue arrastrando 1000
            d(2024, 12, 1), // d2 → observado
        ];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        assert_eq!(
            out[0],
            vec![dec(1000), dec(1000), dec(1000), dec(1000), dec(3000)]
        );
    }

    #[test]
    fn item_only_in_one_snapshot_appears_then_carries() {
        // INVERTIDO en 4.7.0 (#130): antes era «aparece y desaparece» ([0,0,500,0,0]). Ahora el
        // valor observado se arrastra hacia delante (LOCF); antes del primer avistamiento no hay
        // nada que arrastrar (0), y el ÚLTIMO punto sigue siendo 0 por la rama exacta
        // `a == m-1` (una captura final que no lo incluye tampoco lo resucita en su fecha).
        let tl = HistoryTimeline {
            last_is_live_ledger: false,
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
        assert_eq!(out[0], vec![dec(0), dec(0), dec(500), dec(500), dec(0)]);
    }

    #[test]
    fn first_month_clamp_evaluates_at_first_snapshot() {
        // Primer snapshot a mitad de mes; el punto de rejilla de ese mismo mes engancha en él.
        let tl = HistoryTimeline {
            last_is_live_ledger: false,
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
            last_is_live_ledger: true,
            dates: vec![d(2025, 1, 1), today],
            items: vec![
                HistoryItem {
                    source_item_id: Uuid::from_u128(1),
                    kind: HistoryItemKind::Asset,
                    observations: vec![obs(1000), obs(1600)], // vivo en ambos
                    cashflow: vec![],
                },
                HistoryItem {
                    source_item_id: Uuid::from_u128(2),
                    kind: HistoryItemKind::Asset,
                    observations: vec![obs(500), None], // borrado antes de hoy
                    cashflow: vec![],
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

    /// La rejilla que el handler produce **desde 4.0.0**: el último punto es `today`, no su
    /// primero-de-mes.
    ///
    /// Es el mismo timeline del test de arriba con la rejilla nueva, y el contraste es el arreglo:
    /// donde antes salía un interpolado «cerca de 1600», ahora sale **1600 exacto**, porque `today`
    /// es la fecha de la observación virtual y `evaluate_item_at` cae en la rama del último
    /// extremo. El item borrado sigue en 0: eso no cambia.
    #[test]
    fn virtual_today_grid_point_evaluated_at_today_is_exact() {
        let today = d(2025, 6, 10);
        let tl = HistoryTimeline {
            last_is_live_ledger: true,
            dates: vec![d(2025, 1, 1), today],
            items: vec![
                HistoryItem {
                    source_item_id: Uuid::from_u128(1),
                    kind: HistoryItemKind::Asset,
                    observations: vec![obs(1000), obs(1600)],
                    cashflow: vec![],
                },
                HistoryItem {
                    source_item_id: Uuid::from_u128(2),
                    kind: HistoryItemKind::Asset,
                    observations: vec![obs(500), None],
                    cashflow: vec![],
                },
            ],
        };
        let grid = vec![d(2025, 1, 1), today];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        assert_eq!(out[0][0], dec(1000));
        assert_eq!(out[0][1], dec(1600), "en `today` el valor vivo es EXACTO, sin interpolar");
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
            last_is_live_ledger: false,
            dates: vec![d(2025, 6, 1), d(2025, 1, 1)],
            items: vec![item(HistoryItemKind::Asset, vec![obs(1), obs(2)])],
        };
        assert!(matches!(
            evaluate_timeline(&tl, &[d(2025, 3, 1)]),
            Err(EngineError::InvalidHistoryTimeline)
        ));
        // Fechas iguales (no estrictamente ascendentes) también se rechazan.
        let tl_eq = HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d(2025, 1, 1), d(2025, 1, 1)],
            items: vec![item(HistoryItemKind::Asset, vec![obs(1), obs(2)])],
        };
        assert!(matches!(
            evaluate_timeline(&tl_eq, &[d(2025, 1, 1)]),
            Err(EngineError::InvalidHistoryTimeline)
        ));
    }

    // ---- Anclaje de cash-flow (B1) -----------------------------------------------------------

    /// P1 — `v(seg_start) == Va` para cash-flow arbitrario.
    #[test]
    fn anchored_p1_start_equals_va() {
        let seg_start = d(2025, 1, 1);
        let seg_end = d(2025, 3, 1);
        let days_total = (seg_end - seg_start).num_days();
        // Deltas variados dentro del segmento (positivos y negativos, no suman cero).
        let cf = vec![
            cf_e(d(2025, 1, 10), 500),
            cf_e(d(2025, 1, 20), -200),
            cf_e(d(2025, 2, 15), 1000),
        ];
        let v = anchored_cashflow_segment_value(
            dec(1000),
            dec(7777),
            &cf,
            seg_start,
            seg_end,
            seg_start, // eval en el arranque
            0,
            days_total,
        );
        assert_eq!(v, dec(1000));
    }

    /// P2 — `v(seg_end) == Vb` EXACTO para cash-flow arbitrario (incluidos deltas que no suman cero,
    /// un delta grande, y un delta fechado EN `seg_end`).
    #[test]
    fn anchored_p2_end_equals_vb_exact() {
        let seg_start = d(2025, 1, 1);
        let seg_end = d(2025, 3, 1);
        let days_total = (seg_end - seg_start).num_days();
        let cases: Vec<Vec<CashFlowEntry>> = vec![
            vec![],                                              // vacío
            vec![cf_e(d(2025, 1, 15), 500), cf_e(d(2025, 2, 10), 500)], // suma +1000
            vec![cf_e(d(2025, 1, 15), 500), cf_e(d(2025, 2, 10), -500)], // suma 0
            vec![cf_e(d(2025, 2, 28), 123_456)],                 // un delta enorme
            vec![cf_e(seg_end, 999)],                            // delta EN seg_end (cuenta)
            vec![cf_e(seg_start, 4242)],                         // delta EN seg_start (no cuenta)
        ];
        for cf in &cases {
            let v = anchored_cashflow_segment_value(
                dec(1000),
                dec(3000),
                cf,
                seg_start,
                seg_end,
                seg_end, // eval en el cierre
                days_total,
                days_total,
            );
            assert_eq!(v, dec(3000), "Vb debe ser exacto con cash-flow {cf:?}");
        }
    }

    /// P3a — cash-flow vacío ⇒ idéntico a `interpolate_linear` en múltiples fechas de evaluación.
    #[test]
    fn anchored_p3_empty_matches_interpolate_linear_pointwise() {
        let seg_start = d(2025, 1, 1);
        let seg_end = d(2025, 4, 1);
        let days_total = (seg_end - seg_start).num_days();
        let empty: Vec<CashFlowEntry> = vec![];
        for dfs in [0i64, 7, 15, 30, 45, 60, 80, days_total] {
            let eval = seg_start + chrono::Duration::days(dfs);
            let anchored = anchored_cashflow_segment_value(
                dec(1000),
                dec(2000),
                &empty,
                seg_start,
                seg_end,
                eval,
                dfs,
                days_total,
            );
            let linear = interpolate_linear(dec(1000), dec(2000), dfs, days_total);
            assert_eq!(anchored, linear, "dfs={dfs}");
        }
    }

    /// P3b — un timeline completo evaluado con y sin el campo `cashflow` vacío produce Vecs
    /// idénticos (bit a bit): el campo por defecto no altera nada.
    #[test]
    fn anchored_p3_timeline_identical_with_empty_cashflow_field() {
        let dates = vec![d(2025, 1, 1), d(2025, 4, 1), d(2025, 8, 1)];
        let observations = vec![obs(1000), obs(1600), obs(3000)];
        let grid = vec![
            d(2024, 12, 1),
            d(2025, 1, 1),
            d(2025, 2, 1),
            d(2025, 3, 1),
            d(2025, 5, 1),
            d(2025, 6, 1),
            d(2025, 8, 1),
            d(2025, 9, 1),
        ];
        // (a) construido con el helper `item` (cashflow por defecto = vacío)
        let tl_default = HistoryTimeline {
            last_is_live_ledger: false,
            dates: dates.clone(),
            items: vec![item(HistoryItemKind::Asset, observations.clone())],
        };
        // (b) cashflow explícitamente vacío
        let tl_explicit_empty = HistoryTimeline {
            last_is_live_ledger: false,
            dates: dates.clone(),
            items: vec![HistoryItem {
                source_item_id: Uuid::from_u128(1),
                kind: HistoryItemKind::Asset,
                observations: observations.clone(),
                cashflow: vec![],
            }],
        };
        let a = evaluate_timeline(&tl_default, &grid).unwrap();
        let b = evaluate_timeline(&tl_explicit_empty, &grid).unwrap();
        assert_eq!(a, b);
    }

    /// P4 — snapshots planos (`Va == Vb`) + un depósito +100 en el día `d`: salto justo después de
    /// `d` por encima de `Va`, `v(seg_end) == Va` EXACTO, y forma que decae linealmente hacia `Va`.
    /// Segmento de 100 días → `f = dfs/100` exacto → aritmética entera bit a bit.
    #[test]
    fn anchored_p4_flat_snapshots_deposit_jumps_then_decays_linearly() {
        let seg_start = d(2025, 1, 1);
        let days_total = 100i64;
        let seg_end = seg_start + chrono::Duration::days(days_total);
        let dep_date = seg_start + chrono::Duration::days(10);
        let cf = vec![cf_e(dep_date, 100)];
        let (va, vb) = (dec(1000), dec(1000)); // plano

        let eval = |offset: i64| {
            let eval_date = seg_start + chrono::Duration::days(offset);
            anchored_cashflow_segment_value(va, vb, &cf, seg_start, seg_end, eval_date, offset, days_total)
        };

        // `v(n) = 1000 − n` antes del depósito; `1100 − n` a partir de él.
        let just_before = eval(9); // 991
        let at_dep = eval(10); // 1090
        assert_eq!(just_before, dec(991));
        assert_eq!(at_dep, dec(1090));
        // Salto: justo tras el depósito el valor supera Va y salta respecto al día anterior.
        assert!(at_dep > va, "at_dep={at_dep} !> Va={va}");
        assert!(at_dep > just_before, "sin salto: {at_dep} !> {just_before}");
        // v(seg_end) == Va exacto (el snapshot plano manda; el ingreso se reabsorbe).
        assert_eq!(eval(days_total), va);
        // Decaimiento lineal tras el depósito: diferencias iguales entre puntos equiespaciados.
        let (v20, v40, v60) = (eval(20), eval(40), eval(60));
        assert_eq!(v20, dec(1080));
        assert_eq!(v40, dec(1060));
        assert_eq!(v60, dec(1040));
        assert_eq!(v20 - v40, v40 - v60); // colinealidad exacta
    }

    /// P5 — frontera semiabierta `(seg_start, seg_end]`: un delta fechado en `seg_start` NO cuenta
    /// (curva = lineal pura); uno fechado un día después SÍ; uno en `seg_end` SÍ. Vía
    /// `evaluate_timeline` (ejercita también la selección de rama por `has_cashflow`).
    #[test]
    fn anchored_p5_semiopen_boundary() {
        let d_a = d(2025, 1, 1);
        let d_b = d(2025, 2, 1);
        let interior = d(2025, 1, 15);
        let grid = vec![d_a, interior, d_b];

        let build = |cashflow: Vec<CashFlowEntry>| HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d_a, d_b],
            items: vec![HistoryItem {
                source_item_id: Uuid::from_u128(1),
                kind: HistoryItemKind::Asset,
                observations: vec![obs(1000), obs(1000)], // plano
                cashflow,
            }],
        };

        // Delta en seg_start: NO cuenta → rama lineal pura → interior == 1000.
        let at_start = evaluate_timeline(&build(vec![cf_e(d_a, 500)]), &grid).unwrap();
        assert_eq!(at_start[0], vec![dec(1000), dec(1000), dec(1000)]);

        // Delta un día DESPUÉS de seg_start: SÍ cuenta → curva anclada. El ingreso ya ocurrió
        // en el punto interior (2025-01-15 > 2025-01-02) → `C(t)=500` → interior SALTA por encima
        // de Va. Contraste directo con el caso anterior (mismo delta, un día antes, ignorado).
        let after_start =
            evaluate_timeline(&build(vec![cf_e(d(2025, 1, 2), 500)]), &grid).unwrap();
        assert_eq!(after_start[0][0], dec(1000)); // extremo exacto
        assert_eq!(after_start[0][2], dec(1000)); // extremo exacto
        assert!(
            after_start[0][1] > dec(1000),
            "interior={} debería superar Va (el ingreso ya ocurrió y cuenta)",
            after_start[0][1]
        );

        // Delta en seg_end: SÍ cuenta (intervalo cerrado por la derecha). Aún no ha ocurrido en el
        // interior (2025-01-15 < 2025-02-01) → `C(t)=0` → interior PREDECLINA por debajo de Va.
        let at_end = evaluate_timeline(&build(vec![cf_e(d_b, 500)]), &grid).unwrap();
        assert_eq!(at_end[0][0], dec(1000)); // extremo exacto
        assert_eq!(at_end[0][2], dec(1000)); // último snapshot exacto
        assert!(
            at_end[0][1] < dec(1000),
            "interior={} debería predeclinar (< Va) por el ingreso en seg_end",
            at_end[0][1]
        );
    }

    /// Extra — un delta hacia un activo con un único snapshot / sin segmento no rompe nada (sin
    /// panic, comportamiento actual); y el brazo `(Some, None)` y los pasivos ignoran el cash-flow.
    #[test]
    fn anchored_extra_single_snapshot_and_non_asset_paths_ignore_cashflow() {
        // (1) Un único snapshot + cash-flow: se sirve el valor exacto, el cash-flow se ignora.
        let tl = HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d(2025, 3, 1)],
            items: vec![HistoryItem {
                source_item_id: Uuid::from_u128(1),
                kind: HistoryItemKind::Asset,
                observations: vec![obs(1000)],
                cashflow: vec![cf_e(d(2025, 2, 20), 500), cf_e(d(2025, 3, 10), -100)],
            }],
        };
        let grid = vec![d(2025, 2, 1), d(2025, 3, 1), d(2025, 4, 1)];
        let out = evaluate_timeline(&tl, &grid).unwrap();
        assert_eq!(out[0], vec![dec(0), dec(1000), dec(0)]);

        // (2) Item presente en un solo extremo `(Some, None)` + cash-flow: la rama anclada NO
        // aplica (sólo `(Some, Some)` + Asset) y el cash-flow se ignora. Desde #130 el valor se
        // ARRASTRA (LOCF) en vez de caer a 0 — sin rampas: 1000 plano hasta el último punto,
        // que sigue siendo 0 por la rama exacta `a == m-1` (ausente en la captura final).
        let tl2 = HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d(2025, 1, 1), d(2025, 6, 1)],
            items: vec![HistoryItem {
                source_item_id: Uuid::from_u128(2),
                kind: HistoryItemKind::Asset,
                observations: vec![obs(1000), None],
                cashflow: vec![cf_e(d(2025, 3, 1), 999)],
            }],
        };
        let grid2 = vec![d(2025, 1, 1), d(2025, 3, 1), d(2025, 6, 1)];
        let out2 = evaluate_timeline(&tl2, &grid2).unwrap();
        assert_eq!(out2[0], vec![dec(1000), dec(1000), dec(0)]);

        // (3) Pasivo observado en ambos extremos + cash-flow: intacto (amortización pura, sin
        // inyectar cuotas). El resultado debe ser idéntico al del mismo pasivo sin cash-flow.
        let liab_cf = HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d(2024, 1, 1), d(2025, 1, 1)],
            items: vec![HistoryItem {
                source_item_id: Uuid::from_u128(3),
                kind: HistoryItemKind::Liability,
                observations: vec![obs_liab(200_000, 5, 1500), obs_liab(190_000, 5, 1500)],
                cashflow: vec![cf_e(d(2024, 6, 1), 12345)],
            }],
        };
        let liab_plain = HistoryTimeline {
            last_is_live_ledger: false,
            dates: vec![d(2024, 1, 1), d(2025, 1, 1)],
            items: vec![item(
                HistoryItemKind::Liability,
                vec![obs_liab(200_000, 5, 1500), obs_liab(190_000, 5, 1500)],
            )],
        };
        let grid3 = vec![d(2024, 1, 1), d(2024, 6, 1), d(2025, 1, 1)];
        assert_eq!(
            evaluate_timeline(&liab_cf, &grid3).unwrap(),
            evaluate_timeline(&liab_plain, &grid3).unwrap(),
        );
    }
}
