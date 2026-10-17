//! Traducción de un plan a las entradas del motor. En F2-1 solo vive aquí la **regla única de
//! meses**; el resto (`apply_plan`) llega con F2-4 (`.claude/rfc/planes-6.0-f2.md`).

use chrono::{Datelike, NaiveDate};

/// Mes del bucle de simulación en que cae `date`, contando desde `today`:
/// `12·Δaños + Δmeses + 1`. Una fecha del mes en curso → 1; una fecha pasada → 1.
///
/// Es la misma rejilla que el mapeo legacy de los flujos de Simulación (hueco `idx` = mes del
/// bucle `idx + 1`) y que F1a (lo que empieza en `s` aparece en el índice `s − 1` de la serie).
/// `end_date` (inclusivo) usa el mismo mapeo.
#[allow(dead_code)] // F2-4 la consume desde `apply_plan`.
pub(crate) fn loop_month_of(today: NaiveDate, date: NaiveDate) -> u32 {
    let delta = 12 * (date.year() - today.year()) + date.month() as i32 - today.month() as i32;
    if delta < 0 {
        1
    } else {
        delta as u32 + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn current_month_is_loop_month_one() {
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2026, 10, 1)), 1);
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2026, 10, 31)), 1);
    }

    #[test]
    fn months_and_years_add_up() {
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2026, 11, 1)), 2);
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2027, 4, 1)), 7);
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2027, 10, 1)), 13);
    }

    #[test]
    fn past_dates_clamp_to_month_one() {
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2020, 1, 1)), 1);
        assert_eq!(loop_month_of(ymd(2026, 10, 17), ymd(2026, 9, 30)), 1);
    }
}
