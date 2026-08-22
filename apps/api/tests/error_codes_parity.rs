//! Paridad backend↔frontend del catálogo de códigos de error. **No necesita Postgres.**
//!
//! El backend devuelve `ErrorBody.code`: un código estable sacado del prefijo `snake_code:` del
//! mensaje (ver `apps/api/src/error.rs`). La SPA lo traduce con
//! `apps/web/src/lib/errorMessages.ts`. Un código sin entrada en ese catálogo NO explota: cae a
//! una frase genérica, y el usuario ve «Los datos enviados no son válidos» donde podría haber
//! visto «Ese nombre de usuario ya está registrado». Ese fallo es silencioso, y por eso hay un
//! test: sin él, cada código nuevo degrada la UI sin que nadie se entere.
//!
//! Este test extrae los códigos del propio código fuente y los compara con
//! `tests/fixtures/error-codes.json`. El JSON es la fuente de verdad compartida; lo consume
//! también `apps/web/src/lib/errorMessages.test.ts` (mismo patrón que `fire-parity.json`).
//!
//! Al añadir o cambiar un código:
//!   UPDATE_ERROR_CODES=1 cargo test -p futurefin-api --test error_codes_parity
//! …y después añade su frase en español en `errorMessages.ts`, o el test del frontend fallará.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Literales que TIENEN forma de código pero nunca viajan como `ErrorBody.code`. Uno por línea,
/// con el porqué: sin la explicación, quien venga después no sabrá si puede quitarlo.
const NOT_ERROR_CODES: &[&str] = &[
    // Error de protocolo MCP (`ErrorData::internal_error`), no una respuesta HTTP con cuerpo.
    "serialization",
];

/// Clases HTTP que `derive_error_code` usa cuando el mensaje no trae prefijo. Espejo del enum
/// `ErrorCode` con `rename_all = "snake_case"`.
const HTTP_CLASSES: &[&str] = &[
    "bad_request",
    "unprocessable",
    "unauthorized",
    "forbidden",
    "not_found",
    "conflict",
    "unavailable",
    "internal",
];

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("listar el directorio de fuentes") {
        let path = entry.expect("entrada de directorio").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Mismo criterio que `derive_error_code` en `src/error.rs`. Si divergen, este test deja de
/// describir la realidad — mantenlos alineados a mano (son cinco líneas cada uno).
fn code_of(message: &str) -> Option<String> {
    let (prefix, _) = message.split_once(": ")?;
    let ok = (3..=64).contains(&prefix.len())
        && prefix.starts_with(|c: char| c.is_ascii_lowercase())
        && prefix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    ok.then(|| prefix.to_string())
}

/// Elimina los bloques `#[cfg(test)] mod … { … }` —sus literales son casos de prueba, no códigos
/// que la API pueda devolver— **contando llaves**, no cortando por el primero.
///
/// Cortar por el primero costó diez códigos: `handlers/projection.rs` tiene dos módulos de tests
/// EN MEDIO del fichero, así que todo el código real que venía después desaparecía del barrido y
/// `task_panic`, `months_out_of_range` y los cinco `one_off_*` se quedaban sin traducir sin que
/// el test protestara. Un extractor que se calla es peor que no tener extractor.
fn without_test_modules(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        // Saltar hasta la llave de apertura del módulo y consumir hasta su pareja.
        match after.find('{') {
            None => return out,
            Some(brace) => {
                let mut depth = 0i32;
                let mut end = None;
                for (off, c) in after[brace..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(brace + off + 1);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                match end {
                    Some(e) => rest = &after[e..],
                    None => return out,
                }
            }
        }
    }
    out.push_str(rest);
    out
}

/// Extrae TODO literal con forma de `snake_code: …`, venga del constructor que venga.
///
/// Deliberadamente generoso, y no al revés. La primera versión solo miraba los constructores de
/// `ApiError` y se dejaba seis códigos reales de `backup_user/`, porque allí el error nace como
/// `CryptoError::Bad(String)` o como un `Err(String)` de `parse_payload` y solo se convierte a
/// `ApiError::BadRequest` más arriba. Capturar de más cuesta una línea en `NOT_ERROR_CODES`;
/// capturar de menos deja un código sin traducir y el usuario ve un mensaje genérico sin que
/// nadie se entere — que es justo lo que este test existe para impedir.
fn extract_codes() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&src_root(), &mut files);
    let mut codes = BTreeSet::new();
    for file in files {
        let src = fs::read_to_string(&file).expect("leer fuente");
        for lit in string_literals(&without_test_modules(&src)) {
            if let Some(code) = code_of(&lit) {
                if !NOT_ERROR_CODES.contains(&code.as_str()) {
                    codes.insert(code);
                }
            }
        }
    }
    codes
}

/// Literales de cadena de un fuente Rust. No pretende ser un lexer: basta con no cortar en una
/// comilla escapada y con ignorar los comentarios de línea, que es donde viven los ejemplos.
fn string_literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut lit = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(next) = chars.next() {
                            lit.push(next);
                        }
                    }
                    '"' => break,
                    _ => lit.push(c),
                }
            }
            out.push(lit);
        }
    }
    out
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/error-codes.json")
}

#[test]
fn error_codes_fixture_matches_the_source() {
    let codes: Vec<String> = extract_codes().into_iter().collect();
    let classes: Vec<String> = HTTP_CLASSES.iter().map(|s| s.to_string()).collect();

    let doc = "Códigos estables de ErrorBody.code. GENERADO — no editar a mano: \
               UPDATE_ERROR_CODES=1 cargo test -p futurefin-api --test error_codes_parity. \
               Lo consumen apps/api/tests/error_codes_parity.rs y \
               apps/web/src/lib/errorMessages.test.ts (toda entrada necesita frase en español).";
    let generated = serde_json::json!({
        "_doc": doc,
        "http_classes": classes,
        "codes": codes,
    });
    let pretty = format!("{}\n", serde_json::to_string_pretty(&generated).unwrap());

    if std::env::var("UPDATE_ERROR_CODES").is_ok() {
        fs::write(fixture_path(), &pretty).expect("escribir el fixture");
        eprintln!("fixture regenerado con {} códigos", codes.len());
        return;
    }

    let current = fs::read_to_string(fixture_path()).unwrap_or_default();
    if current != pretty {
        let old: BTreeSet<String> = serde_json::from_str::<serde_json::Value>(&current)
            .ok()
            .and_then(|v| v.get("codes").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let now: BTreeSet<String> = codes.iter().cloned().collect();
        let added: Vec<_> = now.difference(&old).collect();
        let removed: Vec<_> = old.difference(&now).collect();
        panic!(
            "tests/fixtures/error-codes.json no coincide con el código fuente.\n\
             nuevos:      {added:?}\n\
             desaparecidos: {removed:?}\n\
             Regenera con: UPDATE_ERROR_CODES=1 cargo test -p futurefin-api --test error_codes_parity\n\
             y añade la frase en español de cada código nuevo en apps/web/src/lib/errorMessages.ts."
        );
    }
}

#[test]
fn every_code_is_lowercase_snake_and_specific() {
    for code in extract_codes() {
        assert!(
            code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "código no snake_case: {code}"
        );
        assert!(
            !HTTP_CLASSES.contains(&code.as_str()),
            "'{code}' repite el nombre de una clase HTTP: un código explícito debe decir MÁS que \
             la clase, o no vale la pena"
        );
    }
}
