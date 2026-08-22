//! Escala de salida de los importes monetarios de la API.
//!
//! Los importes se guardan en `NUMERIC(18,4)`, pero varias cifras derivadas nacen de una
//! **división** y arrastran la escala completa de `rust_decimal`: `(amount × 52) / 12` para una
//! cuota semanal da `433.33333333333333333333333333`, y el promedio de N meses reales
//! (`sum / n`) hace lo mismo. Esas cadenas viajaban tal cual en el JSON: ruido para la UI y,
//! para un consumidor conversacional, precisión inventada — el síntoma que la auditoría MCP
//! reportó como «cifras con veintidós decimales».
//!
//! Esta función es el **punto único**. Antes había dos copias, una en `handlers/projection.rs`
//! y otra en `handlers/transactions/summary.rs`, y solo la segunda mataba el cero negativo: la
//! duplicación era el bug, porque un arreglo se aplicó a una copia y no a la otra.
//!
//! REGLA: se aplica al construir la **respuesta**, nunca a la copia que alimenta al motor.
//! Redondear `FireTarget.base_amount` antes de simular movería el mes del cruce FIRE.

use rust_decimal::Decimal;

/// Cuatro decimales, escala fija, sin cero negativo.
pub fn money_out(d: Decimal) -> Decimal {
    let mut v = d.round_dp(4);
    if v.is_zero() {
        // `impl Neg` voltea el bit de signo también sobre el cero, así que sin esto un importe
        // que sale de negar cero se serializa como `-0.0000`.
        v = Decimal::ZERO;
    }
    v.rescale(4);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn fija_la_escala_y_mata_el_cero_negativo() {
        assert_eq!(money_out(Decimal::from(2000)).to_string(), "2000.0000");
        assert_eq!(
            money_out(Decimal::from_str("433.33333333333333333333333333").unwrap()).to_string(),
            "433.3333"
        );
        assert_eq!(money_out(-Decimal::ZERO).to_string(), "0.0000");
        assert_eq!(money_out(Decimal::from_str("-0.00001").unwrap()).to_string(), "0.0000");
    }
}
