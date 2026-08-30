//! Shared primitives for financial correctness and stable identifiers across services.
//!
//! Currency amounts use [`rust_decimal::Decimal`] (no `f64` for currency or ledger-like values).

pub use rust_decimal::Decimal;
pub use uuid::Uuid;

/// Authenticated account (`AUTH_MODEL.md`: User — not a domain [`Person`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct UserId(pub Uuid);

impl UserId {
    #[must_use]
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for UserId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod no_f64_in_domain_code {
    //! FREEZER `f64` — «Money is Decimal, never `f64`» deja de ser prosa y pasa a ser un test.
    //!
    //! El contrato (CLAUDE.md, y D4 de `futurefin-architecture-contract`) es que la frontera con
    //! `f64` vive **solo** en la capa de publicación de series de `apps/api` — donde un chart
    //! recibe números y la precisión decimal ya no significa nada. En `crates/` **jamás**: aquí
    //! está el dominio y el motor puro, y un `f64` en medio de una cascada de asignación o de una
    //! amortización no produce un error, produce un **número plausible y equivocado**, que es
    //! exactamente el modo de fallo que este repo no sabe detectar de otra forma.
    //!
    //! **Por qué un test y no clippy**: clippy y rustfmt están instalados pero comentados en CI a
    //! propósito (el repo nunca ha pasado ninguno de los dos, y una CI permanentemente roja enseña
    //! a ignorar la CI). Un test que corre en `cargo test --workspace` sí es un gate vivo hoy.
    //!
    //! **Alcance**: solo `crates/domain/src`. `crates/engine` lleva su propio gemelo
    //! (`crates/engine/src/lib.rs`), para que cada crate se vigile a sí mismo con `CARGO_MANIFEST_DIR`
    //! y ningún test dependa de una ruta relativa que cruce crates. `apps/api/src` queda FUERA a
    //! conciencia: allí la excepción de publicación es legítima y matizada, y un barrido ciego solo
    //! produciría una allow-list que nadie mantiene.

    use std::fs;
    use std::path::{Path, PathBuf};

    /// El token prohibido, **ensamblado en dos trozos a propósito**.
    ///
    /// Este fichero se escanea a sí mismo. Escribir el token entero en un literal de cadena —o en
    /// el mensaje de error que explica el fallo— haría que el freezer se cazara a sí mismo y
    /// obligara a inventar una excepción por fichero, que es justo el agujero por el que se cuela
    /// el siguiente de verdad. Es el «comando que se cuenta a sí mismo» que la norma de la casa ya
    /// tiene fichado, y aquí no es una anécdota: pasó al escribir este test.
    ///
    /// Consecuencia práctica: **ningún mensaje ni ningún ejemplo de este módulo puede escribir el
    /// token literal**; todos lo interpolan desde aquí.
    const NEEDLE: &str = concat!("f", "64");

    /// Quita los comentarios de línea (`//`, `///`, `//!`) de cada línea.
    ///
    /// **No hay comentarios de bloque en `crates/*/src`** — verificado con
    /// `grep -rn '/\*' crates/domain/src crates/engine/src` (vacío), y si alguien introduce uno con
    /// el token dentro, este test fallará: es el lado seguro del error (falso positivo ruidoso, no
    /// falso negativo silencioso).
    ///
    /// Tampoco intenta respetar cadenas: un `"//"` dentro de un literal truncaría la línea. Hoy no
    /// ocurre, y de nuevo el sesgo del error es hacia dejar de mirar código, no hacia dejar pasar
    /// el token… salvo que estuviera a la derecha de ese literal. Si algún día importa, arréglalo
    /// con un tokenizador de verdad; NO relajes la aserción.
    fn strip_line_comments(source: &str) -> String {
        source
            .lines()
            .map(|line| match line.find("//") {
                Some(idx) => &line[..idx],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// [`NEEDLE`] como TOKEN, no como subcadena: sin los bordes de palabra, un identificador que lo
    /// contenga (`…64` sufijado, o un prefijo alfanumérico) contaría como violación.
    fn contains_f64_token(code: &str) -> bool {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut from = 0usize;
        while let Some(rel) = code[from..].find(NEEDLE) {
            let start = from + rel;
            let end = start + NEEDLE.len();
            let before_ok = start == 0 || !is_word(code[..start].chars().next_back().unwrap());
            let after_ok = end == code.len() || !is_word(code[end..].chars().next().unwrap());
            if before_ok && after_ok {
                return true;
            }
            from = end;
        }
        false
    }

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("se puede listar el directorio de fuentes") {
            let path = entry.expect("entrada de directorio legible").path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Recorre los `.rs` de `dir` y devuelve los `path:line` con el token fuera de comentario.
    fn f64_outside_comments(dir: &Path) -> Vec<String> {
        let mut files = Vec::new();
        rust_sources(dir, &mut files);
        assert!(
            !files.is_empty(),
            "no se encontró ningún .rs bajo {}: el freezer estaría pasando por vacío \
             (un barrido que no barre nada es deriva silenciosa, no una victoria)",
            dir.display()
        );
        files.sort();

        let mut hits = Vec::new();
        for file in files {
            let source = fs::read_to_string(&file).expect("fuente legible como UTF-8");
            for (i, line) in strip_line_comments(&source).lines().enumerate() {
                if contains_f64_token(line) {
                    hits.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
                }
            }
        }
        hits
    }

    /// El mismo mensaje existe, copiado, en el gemelo de `crates/engine`: un `mod` con `cfg(test)`
    /// no cruza la frontera de crate, y montar un crate de utilidades de test solo para compartir
    /// treinta líneas costaría más de lo que ahorra. Si tocas uno, toca el otro.
    fn failure_message(crate_label: &str, hits: &[String]) -> String {
        format!(
            "`{NEEDLE}` fuera de comentario en {crate_label} ({} sitio(s)):\n  {}\n\n\
             El dinero es SIEMPRE `rust_decimal::Decimal` en el dominio. La excepción D4 del \
             contrato de arquitectura es EXCLUSIVAMENTE la capa de publicación de series de \
             `apps/api` (handlers/projection.rs, handlers/history.rs), donde el chart consume \
             números y la precisión decimal ya no aporta nada: JAMÁS en `crates/`.\n\n\
             Aquí no da un error, da un número plausible y equivocado — el único modo de fallo que \
             este repo no detecta de ninguna otra forma. Si de verdad necesitas coma flotante en \
             el dominio (p. ej. un método numérico), la conversación es de diseño y pasa por \
             `futurefin-architecture-contract` + `futurefin-proof-and-analysis-toolkit`, no por \
             añadir una excepción a este test.",
            hits.len(),
            hits.join("\n  ")
        )
    }

    #[test]
    fn crates_domain_src_has_no_f64_outside_comments() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let hits = f64_outside_comments(&src);
        assert!(
            hits.is_empty(),
            "{}",
            failure_message("crates/domain/src", &hits)
        );
    }

    #[test]
    fn the_scanner_would_actually_catch_the_forbidden_token() {
        // Prueba de vida del detector. Sin esto, un `strip_line_comments` roto (o un `find` que
        // deja de casar) convierte el freezer en un test que siempre pasa — la «deriva silenciosa»
        // de los greps vacíos que la norma de la casa ya tiene fichada.
        //
        // Los ejemplos se construyen con `NEEDLE` interpolado: escribir el token literal aquí lo
        // convertiría en una violación real de este mismo fichero (ver el doc de `NEEDLE`).
        assert!(contains_f64_token(&format!("let x: {NEEDLE} = 0.0;")));
        assert!(contains_f64_token(&format!("(v as {NEEDLE})")));
        assert!(
            !contains_f64_token(&format!("let a{NEEDLE} = 1;")),
            "subcadena a la izquierda, no token"
        );
        assert!(
            !contains_f64_token(&format!("let {NEEDLE}0 = 1;")),
            "subcadena a la derecha, no token"
        );
        assert!(
            !contains_f64_token(&strip_line_comments(&format!("// nunca uses {NEEDLE} aquí"))),
            "un comentario de línea no cuenta"
        );
        assert!(
            !contains_f64_token(&strip_line_comments(&format!("//! sin `{NEEDLE}`."))),
            "un doc-comment de módulo no cuenta"
        );
        assert!(
            contains_f64_token(&strip_line_comments(&format!(
                "let x: {NEEDLE} = 0.0; // ojo"
            ))),
            "el código a la izquierda de un comentario SÍ cuenta"
        );
    }
}
