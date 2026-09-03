//! Arnés de auditoría del modelo financiero: vuelca en CSV (stdout) las series del engine para
//! una batería de casos límite, de modo que un oráculo EXTERNO (reimplementación independiente)
//! pueda compararlas mes a mes sin pasar por la API ni por una base de datos.
//!
//! No afirma nada por sí mismo — las afirmaciones viven en los tests de regresión normales.
//! Uso: `cargo test -p futurefin-engine --test audit_dump -- --nocapture > dump.csv`
//!
//! Formato de las líneas:
//!   `LIABM,<caso>,<mes>,<opening>,<interes>,<principal_amortizado>,<cuota>,<closing>`
//!   `LIABS,<caso>,<payoff_mes|->,<ausencia|->,<interes_total>,<principal_final>,<cuotas_total>`
//!   `PROJ,<caso>,<mes>,<net_worth>,<contributed_capital>`
//!
//! **Los casos ya no viven aquí**: desde WP0 de 5.0.0 son [`cases`]
//! (`tests/common/cases.rs`), compartidos con `golden_pins.rs` para que el pin de bit-identidad
//! y este volcado hablen exactamente de las mismas entradas. Este fichero vuelca lo que
//! `projection_cases_dumped()` declara (L1–L6, P1–P6, P13 y los P14–P23 de 5.0.0) **y solo eso**:
//! su CSV es un contrato con el oráculo externo y no crece cuando el pin gana casos. Ha crecido
//! tres veces, las tres declaradas: WP1a de 5.0.0 (P13 `P13_cash8k_denormal_g`, la regresión de
//! #208 — gross-up mixto con una `g` denormal), WP2 (P14 `P14_techo_numeric`, el desbordamiento
//! de la base de coste de #209, y P15–P17, las tres reglas de retirada nuevas) y WP3 (P18–P23:
//! pensión con fecha, objetivo puente, media jornada, cruce como lectura, techo de aportación y
//! pausa de ingresos). El criterio no cambia: semántica nueva merece oráculo externo, no solo un
//! hash interno.

#[path = "common/cases.rs"]
mod cases;

use cases::{liability_cases, projection_cases_dumped, ref_date};
use futurefin_engine::{
    liability_amortization_schedule, project_net_worth_series, LiabilityPayoffAbsence,
    ProjectionInput, ProjectionLiabilityInput,
};

fn dump_schedule(case: &str, liab: &ProjectionLiabilityInput, horizon: u32) {
    let s = liability_amortization_schedule(liab, ref_date(), horizon);
    for m in &s.months {
        println!(
            "LIABM,{case},{},{},{},{},{},{}",
            m.month_index,
            m.opening_principal,
            m.interest_accrued,
            m.principal_repaid,
            m.payment,
            m.closing_principal
        );
    }
    let absence = match s.payoff_absent {
        None => "-".to_string(),
        Some(LiabilityPayoffAbsence::NoPaymentPlan) => "no_payment_plan".to_string(),
        Some(LiabilityPayoffAbsence::PaymentPlanEndsBeforePayoff) => {
            "plan_ends_before_payoff".to_string()
        }
        Some(LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal) => {
            "payment_does_not_reduce_principal".to_string()
        }
        Some(LiabilityPayoffAbsence::NotWithinHorizon) => "not_within_horizon".to_string(),
    };
    println!(
        "LIABS,{case},{},{absence},{},{},{}",
        s.payoff_month_index
            .map(|k| k.to_string())
            .unwrap_or_else(|| "-".to_string()),
        s.total_interest,
        s.final_principal,
        s.total_payments
    );
}

fn dump_projection(case: &str, input: &ProjectionInput) {
    let out = project_net_worth_series(input).expect("la simulación del caso no debe fallar");
    for (k, nw) in out.net_worth.iter().enumerate() {
        // Columna 5 (4.12.1): el ahorro varado — escalar final repetido por fila para que el
        // oráculo pueda cerrar la identidad sin cambiar de forma.
        println!(
            "PROJ,{case},{k},{nw},{},{}",
            out.contributed_capital[k], out.unallocated_savings_total
        );
    }
}

/// Batería de calendarios de amortización (casos L*).
#[test]
fn audit_dump_liability_schedules() {
    for c in liability_cases() {
        dump_schedule(c.name, &c.liab, c.horizon);
    }
}

/// Batería de proyecciones (casos P*): la histórica MÁS los casos de 5.0.0
/// (`projection_cases_dumped`).
#[test]
fn audit_dump_projection_series() {
    for c in projection_cases_dumped() {
        dump_projection(c.name, &c.input);
    }
}
