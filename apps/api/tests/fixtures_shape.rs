//! Los fixtures CSV siguen ejercitando lo que los tests de import dan por hecho — sin base de datos.
//!
//! Existe porque en agosto de 2026 hubo que **rehacer** `n26_junio.csv` y `myinvestor_junio.csv`:
//! eran extractos bancarios reales (IBAN, nombre, nómina) y salían del repositorio antes de
//! hacerlo público (skill `futurefin-data-hygiene`). Reescribir un fixture es fácil; reescribirlo
//! conservando cada caso que un test asume, no. Este fichero fija esos casos contra las funciones
//! puras del parser, así que falla en segundos y sin Postgres si alguien vuelve a tocarlos.
//!
//! `transactions_import.rs` sigue siendo el test de verdad (preview/confirm reales); esto es el
//! contrato del material de entrada.

use futurefin_api::handlers::transactions::csv_presets::{
    decode_bytes, is_savings_hint, resolve_preset, transfer_flags, ParsedRow,
};
use futurefin_api::handlers::transactions::schema::derive_rule_pattern;
use rust_decimal::Decimal;
use std::str::FromStr;

fn parse_fixture(name: &str) -> (&'static str, Vec<ParsedRow>) {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let text = decode_bytes(&bytes);
    let preset = resolve_preset("auto", &text).expect("autodetección del preset");
    let rows = preset.parse(&text).expect("parseo del fixture");
    (preset.id(), rows)
}

/// Primera fila cuyo concepto contiene `needle` — misma semántica que `row_by_concept`
/// en `transactions_import.rs`, para que ambos ficheros hablen de la misma fila.
fn first<'a>(rows: &'a [ParsedRow], needle: &str) -> (usize, &'a ParsedRow) {
    rows.iter()
        .enumerate()
        .find(|(_, r)| r.concept.contains(needle))
        .unwrap_or_else(|| panic!("ninguna fila contiene '{needle}'"))
}

// ---------------------------------------------------------------------------
// Higiene: los fixtures son fabricados, no exportaciones reales
// ---------------------------------------------------------------------------

#[test]
fn fixtures_carry_no_iban_shaped_string() {
    for name in ["n26_junio.csv", "myinvestor_junio.csv", "myinvestor_win1252.csv"] {
        let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
        let text = decode_bytes(&std::fs::read(&path).unwrap());
        for (i, line) in text.lines().enumerate() {
            // Dos letras + dos dígitos + ≥11 alfanuméricos en mayúsculas = forma de IBAN.
            let bytes: Vec<char> = line.chars().collect();
            for w in bytes.windows(15) {
                let looks_like_iban = w[0].is_ascii_uppercase()
                    && w[1].is_ascii_uppercase()
                    && w[2].is_ascii_digit()
                    && w[3].is_ascii_digit()
                    && w[4..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
                assert!(
                    !looks_like_iban,
                    "{name}:{} parece llevar un IBAN: {line}",
                    i + 1
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MyInvestor
// ---------------------------------------------------------------------------

#[test]
fn myinvestor_fixture_keeps_the_cases_the_import_tests_assume() {
    let (id, rows) = parse_fixture("myinvestor_junio.csv");
    assert_eq!(id, "myinvestor", "autodetección por cabecera");
    assert!(rows.len() > 20, "el test exige >20 filas, hay {}", rows.len());

    // Nómina positiva → income (sin regla ni hint de ahorro que lo desvíe).
    let (_, nomina) = first(&rows, "Nomina Juny");
    assert!(nomina.amount > Decimal::ZERO, "la nómina debe ser positiva");
    assert!(!is_savings_hint(&nomina.concept), "la nómina no es hint de ahorro");

    // Cargo negativo genérico → expense.
    let (_, vending) = first(&rows, "VENDING");
    assert!(vending.amount < Decimal::ZERO);
    assert!(!is_savings_hint(&vending.concept));

    // Heurística de ahorro por token CARTERA/APORTACION.
    let (_, aport) = first(&rows, "Aportacion automatica cartera");
    assert!(is_savings_hint(&aport.concept), "aportación a cartera → savings");
    assert!(aport.amount < Decimal::ZERO);

    // Tokens de transferencia: TRANSFERENCIA, ENVIADA DESDE y ESTALVI.
    let flags = transfer_flags(&rows);
    for needle in ["Transferencia desde MyInvestor", "Enviada desde N26", "estalvi"] {
        let (i, _) = first(&rows, needle);
        assert!(flags[i], "'{needle}' debe sugerirse como transferencia");
    }

    // Decimal español con coma y sufijo de referencia con dígitos (patrón de regla aprendida).
    let (_, periodo) = first(&rows, "PERIODO");
    assert_eq!(periodo.amount, Decimal::from_str("3.51").unwrap());
    assert_eq!(derive_rule_pattern(&periodo.concept), "PERIODO");
}

// ---------------------------------------------------------------------------
// N26
// ---------------------------------------------------------------------------

#[test]
fn n26_fixture_keeps_the_cases_the_import_tests_assume() {
    let (id, rows) = parse_fixture("n26_junio.csv");
    assert_eq!(id, "n26", "autodetección por cabecera");
    let flags = transfer_flags(&rows);

    // Par opuesto el mismo día, con la escala rara de N26 (-26.000000000 → -26.0000).
    let (i_cena, cena) = first(&rows, "Cena");
    assert_eq!(cena.amount, Decimal::from_str("-26.0000").unwrap());
    assert_eq!(cena.amount.scale(), 4, "el parser redondea a 4 decimales");
    assert!(flags[i_cena], "el par opuesto ±26 debe marcarse");

    // Partner «Cuenta de Ahorro» → transferencia aunque no haya token en el concepto.
    let (i_ahorro, ahorro) = first(&rows, "Cuenta de Ahorro");
    assert_eq!(ahorro.partner_name.as_deref(), Some("Cuenta de Ahorro"));
    assert!(flags[i_ahorro]);

    // Token de transferencia entre bancos.
    let (i_cross, _) = first(&rows, "Transferencia desde MyInvestor");
    assert!(flags[i_cross]);

    assert!(
        flags.iter().filter(|f| **f).count() >= 5,
        "el test de import exige ≥5 transferencias sugeridas, hay {}",
        flags.iter().filter(|f| **f).count()
    );

    // Varias filas del mismo comercio con sufijo numérico variable → UNA sola regla aprendida.
    let supers: Vec<&ParsedRow> = rows.iter().filter(|r| r.concept.contains("SUPERMERCADO")).collect();
    assert!(supers.len() >= 3, "hacen falta ≥3 filas de supermercado, hay {}", supers.len());
    let patterns: std::collections::BTreeSet<String> =
        supers.iter().map(|r| derive_rule_pattern(&r.concept)).collect();
    assert_eq!(
        patterns.len(),
        1,
        "todas deben colapsar en un patrón; salieron {patterns:?}"
    );
    assert!(patterns.iter().next().unwrap().contains("SUPERMERCADO"));

    // Concepto = Partner + Reference, y hay filas SIN partner (Debit Transfer suelto).
    assert!(
        rows.iter().any(|r| r.partner_name.is_none()),
        "hace falta al menos una fila sin Partner Name"
    );
}
