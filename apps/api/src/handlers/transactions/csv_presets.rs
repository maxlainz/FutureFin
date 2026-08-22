//! Parser de CSV bancario (A3). Presets hardcoded con autodetección por cabecera; extensible =
//! nuevo `impl CsvPreset`. Pipeline: bytes → strip BOM → UTF-8 válido o decode WINDOWS_1252 →
//! detección → `preset.parse()`. Ningún `f64`: todos los importes son `Decimal`.

use crate::error::ApiError;
use crate::handlers::transactions::schema::{fold_diacritics_upper, normalize_concept};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::str::FromStr;

/// Fila parseada de un CSV, antes de categorizar. Los campos `bank_type`/`partner_name` son
/// metadata usada solo por las heurísticas de transferencia (N26).
#[derive(Debug, Clone)]
pub struct ParsedRow {
    pub op_date: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub concept: String,
    pub amount: Decimal,
    pub currency: String,
    pub bank_type: Option<String>,
    pub partner_name: Option<String>,
}

/// Un formato de CSV bancario reconocible.
pub trait CsvPreset: Sync {
    /// Identificador estable del preset (`myinvestor`, `n26`), devuelto en el preview.
    fn id(&self) -> &'static str;
    /// ¿La primera línea (cabecera) corresponde a este banco?
    fn detect(&self, header_line: &str) -> bool;
    /// Parsea el texto completo (incluida la cabecera) a filas.
    fn parse(&self, text: &str) -> Result<Vec<ParsedRow>, ApiError>;
}

struct MyInvestorPreset;
struct N26Preset;

static MYINVESTOR: MyInvestorPreset = MyInvestorPreset;
static N26: N26Preset = N26Preset;

/// Registro estático de presets, en orden de prioridad de detección.
pub fn all_presets() -> [&'static dyn CsvPreset; 2] {
    [&MYINVESTOR, &N26]
}

/// Decodifica los bytes crudos del CSV a `String`: strip del BOM UTF-8, luego UTF-8 válido o,
/// como fallback, WINDOWS_1252 (exports antiguos con acentos/€).
pub fn decode_bytes(bytes: &[u8]) -> String {
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(stripped) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _had_errors) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(stripped);
            cow.into_owned()
        }
    }
}

/// Resuelve el preset a partir del `source` pedido (`auto`|`myinvestor`|`n26`) y el texto.
pub fn resolve_preset(source: &str, text: &str) -> Result<&'static dyn CsvPreset, ApiError> {
    let first_line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    match source.trim() {
        "auto" | "" => all_presets()
            .into_iter()
            .find(|p| p.detect(first_line))
            .ok_or_else(|| {
                ApiError::BadRequest(
                    "csv_preset_unrecognized: could not auto-detect a known bank CSV format".into(),
                )
            }),
        "myinvestor" => Ok(&MYINVESTOR),
        "n26" => Ok(&N26),
        other => Err(ApiError::BadRequest(format!(
            "csv_preset_unrecognized: unknown source '{other}' (use auto, myinvestor or n26)"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Helpers de parseo
// ---------------------------------------------------------------------------

fn parse_date(s: &str, fmt: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(s.trim(), fmt)
        .map_err(|_| ApiError::BadRequest(format!("csv_date_invalid: could not parse date '{s}'")))
}

/// Decimal en formato español: quitar separadores de miles `.`, coma decimal → punto.
///
/// El `replace('.', "")` es correcto para lo que este preset recibe —importes españoles, donde
/// el punto SIEMPRE es separador de millar— y catastrófico para cualquier otra cosa: `2100.00`
/// se convertía en `210000` y el movimiento entraba **cien veces más grande**, con un 200 y sin
/// una sola señal. Ningún test lo cubría porque ningún importe de los fixtures lleva punto.
///
/// Un separador de millar siempre agrupa de tres en tres, así que un punto seguido de un número
/// de dígitos distinto de tres no es un millar: es un decimal de otro formato, y eso significa
/// que el fichero no es el que el preset cree. Se rechaza en vez de reinterpretarlo — misma
/// disciplina que el resto de la deserialización del repo: rechazar, no caer al default.
fn parse_spanish_decimal(s: &str) -> Result<Decimal, ApiError> {
    let t = s.trim();
    let entera = t.split(',').next().unwrap_or(t);
    let grupos: Vec<&str> = entera.split('.').collect();
    // Un grupo de millar tiene exactamente tres dígitos y el primero no empieza por cero:
    // `0.075` son setenta y cinco milésimas, no setenta y cinco.
    let primero_valido = matches!(grupos[0].as_bytes().first(), Some(b'1'..=b'9'))
        || matches!(grupos[0].as_bytes(), [b'+' | b'-', b'1'..=b'9', ..]);
    if grupos.len() > 1
        && (!primero_valido
            || !grupos[1..]
                .iter()
                .all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit())))
    {
        return Err(ApiError::BadRequest(format!(
            "csv_amount_ambiguous: amount '{s}' uses '.' as a decimal separator; this preset \
             expects Spanish format (thousands '.', decimals ',')"
        )));
    }
    let cleaned = t.replace('.', "").replace(',', ".");
    Decimal::from_str(&cleaned)
        .map_err(|_| ApiError::BadRequest(format!("csv_amount_invalid: could not parse amount '{s}'")))
}

#[cfg(test)]
mod decimal_tests {
    use super::*;

    /// REGRESIÓN — un importe con punto decimal no puede entrar multiplicado por cien.
    #[test]
    fn spanish_decimal_parses_thousands_and_rejects_a_dot_decimal() {
        for (entrada, esperado) in [
            ("1.234,56", "1234.56"),
            ("2.100", "2100"),
            ("1.234.567,89", "1234567.89"),
            ("-26,00", "-26.00"),
            ("450,25", "450.25"),
            ("300", "300"),
        ] {
            assert_eq!(
                parse_spanish_decimal(entrada).expect(entrada).to_string(),
                esperado,
                "entrada {entrada}"
            );
        }
        for entrada in ["2100.00", "0.075", "1.23.4"] {
            let err = parse_spanish_decimal(entrada).unwrap_err();
            assert!(
                format!("{err:?}").contains("csv_amount_ambiguous"),
                "'{entrada}' debe rechazarse, no entrar multiplicado por cien: {err:?}"
            );
        }
    }
}

/// Decimal con punto decimal (N26): parseo directo (soporta `-26.000000000`).
fn parse_dot_decimal(s: &str) -> Result<Decimal, ApiError> {
    Decimal::from_str(s.trim())
        .map_err(|_| ApiError::BadRequest(format!("csv_amount_invalid: could not parse amount '{s}'")))
}

/// Concepto de display: trim, fallback si vacío, truncado a 500 chars (CHECK de la tabla).
fn clean_concept(raw: &str) -> String {
    let t = raw.trim();
    let base = if t.is_empty() { "(sin concepto)" } else { t };
    base.chars().take(500).collect()
}

// ---------------------------------------------------------------------------
// MyInvestor: `;`, %d/%m/%Y, decimal español, negativos = cargos
// ---------------------------------------------------------------------------

impl CsvPreset for MyInvestorPreset {
    fn id(&self) -> &'static str {
        "myinvestor"
    }

    fn detect(&self, header_line: &str) -> bool {
        // Prefijo ASCII robusto frente a codificación (evita el `ó` de "operación").
        header_line.contains("Fecha de operaci") && header_line.contains(';')
    }

    fn parse(&self, text: &str) -> Result<Vec<ParsedRow>, ApiError> {
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(b';')
            .flexible(true)
            .has_headers(true)
            .from_reader(text.as_bytes());
        let mut rows = Vec::new();
        for result in rdr.records() {
            let rec =
                result.map_err(|e| ApiError::BadRequest(format!("csv_parse_error: {e}")))?;
            if rec.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            let op = rec.get(0).unwrap_or("");
            let val = rec.get(1).unwrap_or("").trim();
            let concept = rec.get(2).unwrap_or("");
            let amount_s = rec.get(3).unwrap_or("");
            let divisa = rec.get(4).unwrap_or("").trim();

            let op_date = parse_date(op, "%d/%m/%Y")?;
            let value_date = if val.is_empty() {
                None
            } else {
                Some(parse_date(val, "%d/%m/%Y")?)
            };
            let amount = parse_spanish_decimal(amount_s)?.round_dp(4);
            let currency = if divisa.is_empty() {
                "EUR".to_string()
            } else {
                divisa.to_uppercase()
            };
            rows.push(ParsedRow {
                op_date,
                value_date,
                concept: clean_concept(concept),
                amount,
                currency,
                bank_type: None,
                partner_name: None,
            });
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// N26: RFC-4180 con comillas, `,`, %Y-%m-%d, decimal punto, concepto = Partner + Reference
// ---------------------------------------------------------------------------

impl CsvPreset for N26Preset {
    fn id(&self) -> &'static str {
        "n26"
    }

    fn detect(&self, header_line: &str) -> bool {
        header_line.contains("Booking Date") && header_line.contains("Amount (EUR)")
    }

    fn parse(&self, text: &str) -> Result<Vec<ParsedRow>, ApiError> {
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(b',')
            .flexible(true)
            .has_headers(true)
            .from_reader(text.as_bytes());
        let mut rows = Vec::new();
        for result in rdr.records() {
            let rec =
                result.map_err(|e| ApiError::BadRequest(format!("csv_parse_error: {e}")))?;
            if rec.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            let booking = rec.get(0).unwrap_or("");
            let value = rec.get(1).unwrap_or("").trim();
            let partner = rec.get(2).unwrap_or("").trim();
            let ttype = rec.get(4).unwrap_or("").trim();
            let reference = rec.get(5).unwrap_or("").trim();
            let amount_s = rec.get(7).unwrap_or("");

            let op_date = parse_date(booking, "%Y-%m-%d")?;
            let value_date = if value.is_empty() {
                None
            } else {
                Some(parse_date(value, "%Y-%m-%d")?)
            };
            let amount = parse_dot_decimal(amount_s)?.round_dp(4);

            // Concepto = Partner Name + Payment Reference (hay Debit Transfer sin Partner Name).
            let mut parts: Vec<&str> = Vec::new();
            if !partner.is_empty() {
                parts.push(partner);
            }
            if !reference.is_empty() {
                parts.push(reference);
            }
            let concept_raw = if parts.is_empty() {
                if ttype.is_empty() {
                    "(sin concepto)".to_string()
                } else {
                    ttype.to_string()
                }
            } else {
                parts.join(" ")
            };

            rows.push(ParsedRow {
                op_date,
                value_date,
                concept: clean_concept(&concept_raw),
                amount,
                currency: "EUR".into(),
                bank_type: (!ttype.is_empty()).then(|| ttype.to_string()),
                partner_name: (!partner.is_empty()).then(|| partner.to_string()),
            });
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Heurísticas (solo SUGIEREN; el usuario decide en review)
// ---------------------------------------------------------------------------

const TRANSFER_TOKENS: &[&str] = &[
    "TRASPASO",
    "TRANSFERENCIA",
    "TRANSFER",
    "ENVIADA DESDE",
    "AHORRO",
    "ESTALVI",
];
const SAVINGS_TOKENS: &[&str] = &["APORTACION", "INVERSION", "CARTERA"];

/// Heurística savings (ahorro/inversión): tokens `APORTACION/INVERSION/CARTERA` en el concepto,
/// comparados tras plegar los diacríticos → «Aportación», «inversión» (grafía real acentuada) también
/// disparan la sugerencia.
pub fn is_savings_hint(concept: &str) -> bool {
    let n = fold_diacritics_upper(&normalize_concept(concept));
    SAVINGS_TOKENS.iter().any(|t| n.contains(t))
}

/// Heurística de transferencia interna, paralela a `rows`. Marca por: tokens de transferencia,
/// `partner = "Cuenta de Ahorro"`, o par opuesto de igual |importe| a ±3 días en el mismo batch.
pub fn transfer_flags(rows: &[ParsedRow]) -> Vec<bool> {
    let n = rows.len();
    let mut flags = vec![false; n];
    for (i, r) in rows.iter().enumerate() {
        let norm = normalize_concept(&r.concept);
        if TRANSFER_TOKENS.iter().any(|t| norm.contains(t)) {
            flags[i] = true;
            continue;
        }
        if r
            .partner_name
            .as_deref()
            .map(|p| normalize_concept(p).contains("CUENTA DE AHORRO"))
            .unwrap_or(false)
        {
            flags[i] = true;
        }
    }
    // Par opuesto de igual |importe| a ±3 días.
    for i in 0..n {
        for j in (i + 1)..n {
            if rows[i].amount != Decimal::ZERO && rows[i].amount == -rows[j].amount {
                let diff = (rows[i].op_date - rows[j].op_date).num_days().abs();
                if diff <= 3 {
                    flags[i] = true;
                    flags[j] = true;
                }
            }
        }
    }
    flags
}
