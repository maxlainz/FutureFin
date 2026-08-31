//! Monthly net-worth projection.

mod history;
mod net_return;
mod projection;
mod runway;

pub use history::{
    add_months_signed, amortized_segment_value, anchored_cashflow_segment_value, evaluate_timeline,
    month_index_of, CashFlowEntry, HistoryItem, HistoryItemKind, HistoryObservation,
    HistoryTimeline, LoanTerms,
};
pub use net_return::{net_return_percentages, NetReturn};
pub use projection::{
    fire_target_at_month_index, first_month_allocation,
    first_month_per_asset_contribution_nominals, liability_amortization_schedule,
    liability_interest_accrues,
    present_value_of_payments, project_net_worth_series, resolve_cap_ceiling, AllocationCap,
    AllocationKind,
    AllocationRule, AllocationSkipReason, EngineError, FireTarget, FirstMonthAllocation,
    EarlyRepaymentEffect, LiabilityPayoffAbsence, LiabilitySchedule, LiabilityScheduleMonth,
    ProjectionInput,
    ProjectionLiabilityInput, ProjectionOutput, RepaymentModel, RuleOutcome, SimAsset,
    MAX_LIABILITY_SCHEDULE_MONTHS,
};
pub use runway::{liquid_runway_months, RunwayOutcome, MAX_RUNWAY_MONTHS};

#[cfg(test)]
mod no_f64_in_domain_code {
    //! FREEZER `f64` — gemelo del de `crates/domain/src/lib.rs`, aplicado al motor.
    //!
    //! Aquí es donde más muerde: la cascada de asignación, la amortización francesa, el gross-up y
    //! el runway son aritmética encadenada sobre cientos de meses. Un `f64` en cualquiera de esos
    //! bucles no falla, **acumula**: devuelve un patrimonio final creíble y equivocado, y ningún
    //! test de rango lo caza. El contrato (CLAUDE.md, D4 de `futurefin-architecture-contract`) es
    //! que la frontera con `f64` vive SOLO en la capa de publicación de series de `apps/api`.
    //!
    //! Cada crate se vigila a sí mismo con su propio `CARGO_MANIFEST_DIR`: ningún test cruza una
    //! ruta relativa hacia otro crate. El código de abajo está duplicado a propósito respecto al de
    //! `crates/domain` — un `mod` con `cfg(test)` no cruza la frontera de crate. **Si tocas uno,
    //! toca el otro.**

    use std::fs;
    use std::path::{Path, PathBuf};

    /// El token prohibido, **ensamblado en dos trozos a propósito**.
    ///
    /// Este fichero se escanea a sí mismo: escribir el token entero en un literal —o en el mensaje
    /// de error que explica el fallo— hace que el freezer se cace a sí mismo y obligue a inventar
    /// una excepción por fichero, que es justo el agujero por el que se cuela el siguiente de
    /// verdad. Es el «comando que se cuenta a sí mismo» que la norma de la casa ya tiene fichado, y
    /// aquí no es una anécdota: pasó al escribir este test.
    ///
    /// Consecuencia: **ningún mensaje ni ningún ejemplo de este módulo escribe el token literal**;
    /// todos lo interpolan desde aquí.
    const NEEDLE: &str = concat!("f", "64");

    /// Quita los comentarios de línea (`//`, `///`, `//!`).
    ///
    /// No hay comentarios de bloque en `crates/*/src` (verificado con
    /// `grep -rn '/\*' crates/domain/src crates/engine/src`, vacío). Si alguien mete uno con el
    /// token dentro, este test falla: falso positivo ruidoso en vez de falso negativo silencioso.
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
    /// contenga contaría como violación.
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

    #[test]
    fn crates_engine_src_has_no_f64_outside_comments() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let hits = f64_outside_comments(&src);
        assert!(
            hits.is_empty(),
            "`{NEEDLE}` fuera de comentario en crates/engine/src ({} sitio(s)):\n  {}\n\n\
             El dinero es SIEMPRE `rust_decimal::Decimal` en el motor. La excepción D4 del \
             contrato de arquitectura es EXCLUSIVAMENTE la capa de publicación de series de \
             `apps/api` (handlers/projection.rs, handlers/history.rs), donde el chart consume \
             números y la precisión decimal ya no aporta nada: JAMÁS en `crates/`.\n\n\
             En un bucle de 840 meses el error de coma flotante no aparece como error: aparece \
             como un patrimonio final creíble y equivocado. Si de verdad hace falta coma flotante \
             (un método numérico, por ejemplo), la conversación es de diseño y pasa por \
             `futurefin-architecture-contract` + `futurefin-proof-and-analysis-toolkit`, no por \
             añadir una excepción a este test.",
            hits.len(),
            hits.join("\n  ")
        );
    }

    #[test]
    fn the_scanner_would_actually_catch_the_forbidden_token() {
        // Prueba de vida del detector: sin esto, un `strip_line_comments` roto convierte el
        // freezer en un test que siempre pasa — la deriva silenciosa del grep vacío.
        //
        // Los ejemplos se construyen interpolando `NEEDLE`: escribir el token literal aquí sería
        // una violación real de este mismo fichero (ver el doc de `NEEDLE`).
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
