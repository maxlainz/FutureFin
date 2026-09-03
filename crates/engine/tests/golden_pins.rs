//! **PIN DORADO DEL MOTOR 4.15.0** — la red de bit-identidad del refactor 5.0.0.
//!
//! Qué hace: para cada caso de `tests/common/cases.rs` (L1–L6 y P1–P13) canonicaliza a TEXTO
//! *todas* las salidas del motor —hasta el último dígito de cada `Decimal`, vía `Display`— y las
//! resume en un SHA-256 guardado en `tests/fixtures/pins-4.15.json`. Si el refactor mueve un solo
//! dígito de una serie, de una traza de cascada o de un calendario de amortización, el hash del
//! caso cambia y el test falla nombrando el caso y enseñando los escalares que se movieron.
//!
//! Por qué un hash y no `assert_eq!` de las series: P9 solo son ya ~10.000 números; un pin
//! literal sería ilegible y nadie lo revisaría. El hash da la señal binaria («esto cambió») y los
//! escalares del fixture (`net_worth_last`, `liquid_worth_last`, `contributed_last`,
//! `assets_depleted_month_index`) dan el titular legible en el diff — que es justo lo que hace
//! revisable un pin que se mueve. Los ANCLAS de las cifras derivables a mano viven en
//! [`p7_and_p9_are_anchored_by_hand_derived_numbers`]: sin ellas el arnés solo probaría que el
//! motor es reproducible, no que empieza donde debe.
//!
//! **Qué se canonicaliza de cada proyección** (todo lo que el motor publica, nada menos):
//! `net_worth`, `liquid_worth`, `contributed_capital`, `per_asset_series`,
//! `assets_depleted_month_index`, `uncovered_deficit_total`, `unallocated_savings_total`, el
//! `first_month_allocation` completo (base, componentes, sobrante y la traza regla a regla con
//! techo, hueco y motivo de salto) y —para los casos con pasivos— el
//! `liability_amortization_schedule` de cada uno.
//!
//! **Regenerar cuando el cambio es INTENCIONADO** (mismo patrón que `UPDATE_MCP_CATALOG=1` en
//! `apps/api/tests/mcp_http.rs`):
//!
//! ```text
//! UPDATE_ENGINE_PINS=1 cargo test -p futurefin-engine --test golden_pins
//! ```
//!
//! …y **documenta el delta en el CHANGELOG** con la puerta de `futurefin-change-control`. Un pin
//! regenerado sin entrada de CHANGELOG es un cambio de números que nadie declaró.

#[path = "common/cases.rs"]
mod cases;

use cases::{
    liability_cases, projection_cases_5_0, projection_cases_all, projection_cases_audit,
    projection_cases_dumped, ref_date,
};
use futurefin_engine::{
    first_month_allocation, liability_amortization_schedule, project_net_worth_series,
    AllocationSkipReason, EngineWarning, FirstMonthAllocation, LiabilityPayoffAbsence,
    LiabilitySchedule, Phase, ProjectionInput, ProjectionOutput, SpendMode, WithdrawalRule,
};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// =============================================================================================
// Canonicalización
// =============================================================================================

/// `Option<Decimal>` / `Option<u32>` → texto. `-` es la AUSENCIA, y jamás se confunde con `0`
/// porque `0` se escribe `0`: la misma norma que la API aplica a sus `null`.
fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_else(|| "-".to_string())
}

fn absence_tag(a: Option<LiabilityPayoffAbsence>) -> &'static str {
    match a {
        None => "-",
        Some(LiabilityPayoffAbsence::NoPaymentPlan) => "no_payment_plan",
        Some(LiabilityPayoffAbsence::PaymentPlanEndsBeforePayoff) => "plan_ends_before_payoff",
        Some(LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal) => {
            "payment_does_not_reduce_principal"
        }
        Some(LiabilityPayoffAbsence::NotWithinHorizon) => "not_within_horizon",
    }
}

fn skip_tag(r: Option<AllocationSkipReason>) -> &'static str {
    match r {
        None => "-",
        Some(AllocationSkipReason::NoCash) => "no_cash",
        Some(AllocationSkipReason::NotReached) => "not_reached",
        Some(AllocationSkipReason::CapFull) => "cap_full",
        Some(AllocationSkipReason::ZeroAmount) => "zero_amount",
        Some(AllocationSkipReason::InvalidTarget) => "invalid_target",
    }
}

/// Texto canónico de un calendario de amortización. `prefix` distingue el pasivo cuando el
/// calendario cuelga de una proyección con varios.
fn render_schedule(prefix: &str, s: &LiabilitySchedule, out: &mut String) {
    let _ = writeln!(out, "{prefix} horizon {}", s.horizon_months);
    let _ = writeln!(out, "{prefix} opening {}", s.opening_principal);
    for m in &s.months {
        let _ = writeln!(
            out,
            "{prefix} m {} {} {} {} {} {} {} {}",
            m.month_index,
            m.opening_principal,
            m.interest_accrued,
            m.principal_repaid,
            m.extra_principal,
            m.early_repayment_fee,
            m.payment,
            m.closing_principal
        );
    }
    let _ = writeln!(out, "{prefix} final_principal {}", s.final_principal);
    let _ = writeln!(out, "{prefix} total_interest {}", s.total_interest);
    let _ = writeln!(out, "{prefix} total_payments {}", s.total_payments);
    let _ = writeln!(
        out,
        "{prefix} total_extra_principal {}",
        s.total_extra_principal
    );
    let _ = writeln!(
        out,
        "{prefix} total_early_repayment_fee {}",
        s.total_early_repayment_fee
    );
    let _ = writeln!(out, "{prefix} total_cash_out {}", s.total_cash_out);
    let _ = writeln!(out, "{prefix} payoff {}", opt(s.payoff_month_index));
    let _ = writeln!(out, "{prefix} absence {}", absence_tag(s.payoff_absent));
}

fn render_liability_case(name: &str, s: &LiabilitySchedule) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "case {name}");
    let _ = writeln!(out, "kind liability");
    render_schedule("sched", s, &mut out);
    out
}

/// Texto canónico de una proyección. Recibe las tres piezas ya calculadas para que el
/// auto-test de vida ([`the_hash_actually_notices_a_single_moved_decimal`]) pueda mutar una y
/// comprobar que el hash se entera.
fn render_projection_case(
    name: &str,
    o: &ProjectionOutput,
    fma: &FirstMonthAllocation,
    scheds: &[LiabilitySchedule],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "case {name}");
    let _ = writeln!(out, "kind projection");
    let _ = writeln!(out, "months {}", o.net_worth.len());
    let _ = writeln!(out, "assets {}", o.per_asset_series.len());
    for (k, v) in o.net_worth.iter().enumerate() {
        let _ = writeln!(out, "nw {k} {v}");
    }
    for (k, v) in o.liquid_worth.iter().enumerate() {
        let _ = writeln!(out, "liq {k} {v}");
    }
    for (k, v) in o.contributed_capital.iter().enumerate() {
        let _ = writeln!(out, "contrib {k} {v}");
    }
    for (i, serie) in o.per_asset_series.iter().enumerate() {
        for (k, v) in serie.iter().enumerate() {
            let _ = writeln!(out, "asset {i} {k} {v}");
        }
    }
    let _ = writeln!(
        out,
        "assets_depleted_month_index {}",
        opt(o.assets_depleted_month_index)
    );
    let _ = writeln!(out, "uncovered_deficit_total {}", o.uncovered_deficit_total);
    let _ = writeln!(
        out,
        "unallocated_savings_total {}",
        o.unallocated_savings_total
    );
    let _ = writeln!(out, "fma base_cash {}", fma.base_cash);
    let _ = writeln!(out, "fma recurring_net {}", fma.recurring_net);
    let _ = writeln!(out, "fma planning_component {}", fma.planning_component);
    let _ = writeln!(out, "fma debt_service {}", fma.debt_service);
    let _ = writeln!(out, "fma leftover {}", fma.leftover);
    for (i, v) in fma.per_asset.iter().enumerate() {
        let _ = writeln!(out, "fma per_asset {i} {v}");
    }
    for r in &fma.rules {
        let _ = writeln!(
            out,
            "fma rule {} {} {} {} {} {} {}",
            r.rule_index,
            r.target_index,
            r.amount_intent,
            r.amount_resolved,
            opt(r.cap_ceiling),
            opt(r.cap_room),
            skip_tag(r.skipped_reason)
        );
    }
    for (i, s) in scheds.iter().enumerate() {
        render_schedule(&format!("liab{i}"), s, &mut out);
    }
    out
}

fn sha256_hex(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

// =============================================================================================
// Recálculo de la batería
// =============================================================================================

/// Lo que el fixture guarda de un caso: el hash y los escalares que hacen legible un diff.
struct Pin {
    name: String,
    sha256: String,
    horizon_months: u32,
    /// `None` en los casos L* (un calendario de amortización no tiene patrimonio).
    net_worth_last: Option<Decimal>,
    liquid_worth_last: Option<Decimal>,
    contributed_last: Option<Decimal>,
    assets_depleted_month_index: Option<u32>,
    /// Solo casos L*: el titular legible de un calendario.
    final_principal: Option<Decimal>,
    total_interest: Option<Decimal>,
    payoff_month_index: Option<u32>,
}

fn projection_pieces(
    name: &str,
    input: &ProjectionInput,
) -> (
    ProjectionOutput,
    FirstMonthAllocation,
    Vec<LiabilitySchedule>,
) {
    let out = project_net_worth_series(input)
        .unwrap_or_else(|e| panic!("el caso {name} no debe fallar en la simulación: {e}"));
    let fma = first_month_allocation(input)
        .unwrap_or_else(|e| panic!("el caso {name} no debe fallar en first_month_allocation: {e}"));
    // Los calendarios se piden con el MISMO horizonte que la proyección: un calendario servido
    // con otro horizonte no es el que la serie ejecutó.
    let scheds = input
        .liabilities
        .iter()
        .map(|l| liability_amortization_schedule(l, input.ref_date, input.horizon_months))
        .collect();
    (out, fma, scheds)
}

/// Recalcula la batería entera contra el motor VIVO, en el orden del fixture.
fn live_pins() -> Vec<Pin> {
    let mut pins = Vec::new();

    for c in liability_cases() {
        let s = liability_amortization_schedule(&c.liab, ref_date(), c.horizon);
        let text = render_liability_case(c.name, &s);
        pins.push(Pin {
            name: c.name.to_string(),
            sha256: sha256_hex(&text),
            horizon_months: s.horizon_months,
            net_worth_last: None,
            liquid_worth_last: None,
            contributed_last: None,
            assets_depleted_month_index: None,
            final_principal: Some(s.final_principal),
            total_interest: Some(s.total_interest),
            payoff_month_index: s.payoff_month_index,
        });
    }

    for c in projection_cases_all() {
        let (out, fma, scheds) = projection_pieces(c.name, &c.input);
        let text = render_projection_case(c.name, &out, &fma, &scheds);
        pins.push(Pin {
            name: c.name.to_string(),
            sha256: sha256_hex(&text),
            horizon_months: c.input.horizon_months,
            net_worth_last: out.net_worth.last().copied(),
            liquid_worth_last: out.liquid_worth.last().copied(),
            contributed_last: out.contributed_capital.last().copied(),
            assets_depleted_month_index: out.assets_depleted_month_index,
            final_principal: None,
            total_interest: None,
            payoff_month_index: None,
        });
    }

    pins
}

// =============================================================================================
// Fixture
// =============================================================================================

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pins-4.15.json")
}

const DOC: &str = "Pin dorado del motor 4.15.0: SHA-256 del texto canónico de cada caso de \
tests/common/cases.rs (series net_worth, liquid_worth, contributed_capital y per_asset_series \
mes a mes con Decimal completo via Display, mas assets_depleted_month_index, \
uncovered_deficit_total, unallocated_savings_total, el first_month_allocation entero con su \
traza regla a regla, y el liability_amortization_schedule de cada pasivo). GENERADO: no editar \
a mano. El formato exacto del texto hasheado lo define render_projection_case / \
render_liability_case en tests/golden_pins.rs. Regenerar SOLO si el cambio del motor es \
intencionado: UPDATE_ENGINE_PINS=1 cargo test -p futurefin-engine --test golden_pins, y \
documentar el delta en el CHANGELOG (futurefin-change-control). Los escalares de cada caso no \
entran en el hash: estan para que un hash que se mueve tenga un titular legible en el diff. Los \
casos L* son calendarios de amortizacion y no tienen patrimonio: sus tres escalares de \
patrimonio son null y llevan en su lugar final_principal, total_interest y payoff_month_index.";

/// JSON escrito **a mano** y en orden de batería a propósito: `serde_json::Map` ordena sus claves
/// por el tipo de mapa con el que se compiló (`BTreeMap`, o `IndexMap` si alguien activa
/// `preserve_order` en cualquier punto del grafo), así que delegar el orden en él haría depender
/// el fichero de una feature de otro crate. Todos los valores son ASCII seguro (nombres
/// `[A-Za-z0-9_]`, hex y decimales), así que no hay nada que escapar.
fn render_fixture(pins: &[Pin]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    let _ = writeln!(s, "  \"_doc\": \"{DOC}\",");
    s.push_str("  \"engine_version\": \"4.15.0\",\n");
    s.push_str(
        "  \"hash_algo\": \"sha256 over the canonical text described in golden_pins.rs\",\n",
    );
    s.push_str("  \"cases\": {\n");
    for (i, p) in pins.iter().enumerate() {
        let comma = if i + 1 == pins.len() { "" } else { "," };
        let _ = writeln!(s, "    \"{}\": {{", p.name);
        let _ = writeln!(s, "      \"sha256\": \"{}\",", p.sha256);
        let _ = writeln!(s, "      \"horizon_months\": {},", p.horizon_months);
        let _ = writeln!(
            s,
            "      \"net_worth_last\": {},",
            json_dec(p.net_worth_last)
        );
        let _ = writeln!(
            s,
            "      \"liquid_worth_last\": {},",
            json_dec(p.liquid_worth_last)
        );
        let _ = writeln!(
            s,
            "      \"contributed_last\": {},",
            json_dec(p.contributed_last)
        );
        let tail_comma = if p.final_principal.is_some() { "," } else { "" };
        let _ = writeln!(
            s,
            "      \"assets_depleted_month_index\": {}{tail_comma}",
            json_u32(p.assets_depleted_month_index)
        );
        if p.final_principal.is_some() {
            let _ = writeln!(
                s,
                "      \"final_principal\": {},",
                json_dec(p.final_principal)
            );
            let _ = writeln!(
                s,
                "      \"total_interest\": {},",
                json_dec(p.total_interest)
            );
            let _ = writeln!(
                s,
                "      \"payoff_month_index\": {}",
                json_u32(p.payoff_month_index)
            );
        }
        let _ = writeln!(s, "    }}{comma}");
    }
    s.push_str("  }\n");
    s.push_str("}\n");
    s
}

/// Decimales como STRING (`"1234.5600"`): un `Decimal` de 28 dígitos no cabe en un `f64` de JSON,
/// y este repo ya serializa así el dinero en toda la API.
fn json_dec(v: Option<Decimal>) -> String {
    v.map(|d| format!("\"{d}\""))
        .unwrap_or_else(|| "null".into())
}

fn json_u32(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "null".into())
}

// =============================================================================================
// Tests
// =============================================================================================

/// El pin. Recalcula la batería y la compara caso a caso contra `tests/fixtures/pins-4.15.json`.
#[test]
fn golden_pins_match_4_15_0() {
    let pins = live_pins();
    let generated = render_fixture(&pins);

    if std::env::var("UPDATE_ENGINE_PINS").is_ok() {
        std::fs::write(fixture_path(), &generated).expect("escribir el fixture de pins");
        eprintln!(
            "pins-4.15.json regenerado con {} casos — REVISA EL DIFF y documenta el delta en el \
             CHANGELOG antes de mergear.",
            pins.len()
        );
        return;
    }

    let raw = std::fs::read_to_string(fixture_path()).unwrap_or_else(|e| {
        panic!(
            "no se puede leer {}: {e}. Si es la primera vez, genera el fixture con \
             UPDATE_ENGINE_PINS=1 cargo test -p futurefin-engine --test golden_pins",
            fixture_path().display()
        )
    });
    let stored: serde_json::Value =
        serde_json::from_str(&raw).expect("el fixture de pins debe ser JSON válido");

    assert_eq!(
        stored["engine_version"].as_str(),
        Some("4.15.0"),
        "el pin dice ser de otra versión del motor que 4.15.0: o el fichero se regeneró para una \
         versión nueva (entonces esta constante y el CHANGELOG tienen que decirlo) o alguien lo \
         editó a mano"
    );

    let stored_cases = stored["cases"]
        .as_object()
        .expect("el fixture debe tener un objeto `cases`");

    // 1) El CONJUNTO de casos. Un caso que desaparece del fixture es un caso que dejó de estar
    //    pineado, y eso no puede pasar en silencio: un pin que cubre menos no falla, deja de
    //    proteger.
    let mut live_names: Vec<&str> = pins.iter().map(|p| p.name.as_str()).collect();
    let mut stored_names: Vec<&str> = stored_cases.keys().map(|k| k.as_str()).collect();
    live_names.sort_unstable();
    stored_names.sort_unstable();
    assert_eq!(
        live_names, stored_names,
        "la batería de casos y el fixture no cubren los mismos casos. Si has añadido o retirado \
         un caso a propósito, regenera con UPDATE_ENGINE_PINS=1 cargo test -p futurefin-engine \
         --test golden_pins"
    );

    // 2) El HASH de cada caso.
    let mut report = String::new();
    for p in &pins {
        let stored_case = &stored_cases[&p.name];
        let stored_sha = stored_case["sha256"].as_str().unwrap_or("");
        if stored_sha == p.sha256 {
            continue;
        }
        let _ = writeln!(report, "  · caso {}", p.name);
        let _ = writeln!(
            report,
            "      sha256                       {} → {}",
            stored_sha, p.sha256
        );
        for (field, live) in [
            ("net_worth_last", json_dec(p.net_worth_last)),
            ("liquid_worth_last", json_dec(p.liquid_worth_last)),
            ("contributed_last", json_dec(p.contributed_last)),
            (
                "assets_depleted_month_index",
                json_u32(p.assets_depleted_month_index),
            ),
            ("final_principal", json_dec(p.final_principal)),
            ("total_interest", json_dec(p.total_interest)),
            ("payoff_month_index", json_u32(p.payoff_month_index)),
        ] {
            let before = &stored_case[field];
            if before.is_null() && live == "null" {
                continue;
            }
            let before_txt = match before {
                serde_json::Value::String(s) => format!("\"{s}\""),
                other => other.to_string(),
            };
            let marker = if before_txt == live { "  " } else { "≠ " };
            let _ = writeln!(report, "      {marker}{field:<26} {before_txt} → {live}");
        }
    }

    assert!(
        report.is_empty(),
        "el motor cambió un dígito respecto a 4.15.0. Si el cambio es intencional, regenera con \
         UPDATE_ENGINE_PINS=1 y documenta el delta en el CHANGELOG \
         (futurefin-change-control).\n\nCasos que se movieron (guardado → vivo):\n{report}\n\
         Un escalar marcado «≠» dice POR DÓNDE se movió el caso; si todos coinciden y el hash no, \
         el cambio está dentro de las series o de la traza de la cascada, no en sus extremos."
    );
}

/// Prueba de vida del arnés: si mover UN dígito de UNA serie no cambiase el hash, el pin sería
/// decorativo. Es la misma disciplina que el freezer de `f64` aplica a su escáner
/// (`the_scanner_would_actually_catch_the_forbidden_token`): un detector sin control negativo es
/// un test que siempre pasa.
#[test]
fn the_hash_actually_notices_a_single_moved_decimal() {
    let case = projection_cases_all()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 debe existir en la batería");
    let (out, fma, scheds) = projection_pieces(case.name, &case.input);

    let baseline = sha256_hex(&render_projection_case(case.name, &out, &fma, &scheds));

    // 1) Un céntimo de céntimo en un mes cualquiera de la serie de patrimonio.
    let mut mutated = out.clone();
    let k = mutated.net_worth.len() / 2;
    mutated.net_worth[k] += Decimal::new(1, 10);
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_case(case.name, &mutated, &fma, &scheds)),
        "mover 1e-10 en net_worth[{k}] no cambió el hash: la canonicalización está redondeando o \
         perdiendo dígitos, y el pin no protege nada"
    );

    // 2) Un dígito en una serie que NO es `net_worth` — la que un refactor descuidado sí podría
    //    mover sin tocar el titular (el patrimonio total no cambia si sube un activo y baja otro).
    let mut mutated = out.clone();
    mutated.per_asset_series[2][k] += Decimal::new(1, 10);
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_case(case.name, &mutated, &fma, &scheds)),
        "mover un dígito de per_asset_series no cambió el hash"
    );

    // 3) Un dígito dentro de la TRAZA de la cascada: el «por qué» de la asignación también está
    //    pineado, no solo el «cuánto».
    let mut fma_mut = fma.clone();
    fma_mut.rules[0].cap_room = fma_mut.rules[0].cap_room.map(|r| r + Decimal::new(1, 10));
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_case(case.name, &out, &fma_mut, &scheds)),
        "mover el hueco de un tope en la traza no cambió el hash"
    );

    // 4) Un dígito dentro del calendario de amortización de un pasivo.
    let mut scheds_mut = scheds.clone();
    scheds_mut[0].total_interest += Decimal::new(1, 10);
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_case(case.name, &out, &fma, &scheds_mut)),
        "mover el interés total de un pasivo no cambió el hash"
    );
}

/// Los casos que `audit_dump.rs` vuelca son EL PREFIJO de los que el pin cubre, en el mismo
/// orden. Sin esto, reordenar la batería cambiaría el CSV que el oráculo externo consume sin que
/// nada lo delatase: el hash de cada caso viaja con su nombre y sobreviviría a la permutación.
#[test]
fn the_audit_battery_is_the_ordered_prefix_of_the_pinned_battery() {
    let audit: Vec<&str> = projection_cases_audit().iter().map(|c| c.name).collect();
    let all: Vec<&str> = projection_cases_all().iter().map(|c| c.name).collect();
    assert_eq!(
        audit,
        all[..audit.len()].to_vec(),
        "projection_cases_all() ya no empieza por la batería de auditoría en su orden: el CSV de \
         audit_dump es un contrato con un oráculo externo"
    );
    assert_eq!(
        audit.len(),
        7,
        "la batería histórica son los 6 casos P1–P6 más P13 (la regresión de la issue #208, \
         añadida en WP1a de 5.0.0); si de verdad hace falta uno más, el oráculo externo tiene que \
         enterarse"
    );

    // Y lo que el CSV vuelca de verdad: esa batería histórica MÁS los casos de 5.0.0, en ese
    // orden. El CSV creció en WP2 (P14–P17) y esta línea es donde ese crecimiento está declarado:
    // si alguien mete un caso en `projection_cases_5_0()` sin querer volcarlo, aquí se ve.
    let dumped: Vec<&str> = projection_cases_dumped().iter().map(|c| c.name).collect();
    let nuevos: Vec<&str> = projection_cases_5_0().iter().map(|c| c.name).collect();
    assert_eq!(
        dumped,
        [audit.clone(), nuevos.clone()].concat(),
        "el CSV de audit_dump es la batería histórica seguida de la de 5.0.0"
    );
    assert_eq!(
        nuevos,
        vec![
            "P14_techo_numeric",
            "P15_percent_of_balance_ceiling",
            "P16_hybrid_rule_is_spend",
            "P17_guardrails_taxes_es",
            // WP3 (§B.1/§B.3/§B.7): pensión con fecha, puente, media jornada, cruce como
            // lectura, techo de aportación y pausa de ingresos.
            "P18_pension_bridge",
            "P19_pension_perpetuity_covering",
            "P20_partial_media_jornada",
            "P21_retire_at_age_reading_only",
            "P22_solve_required_contribution",
            "P23_income_pause",
        ],
        "los casos de 5.0.0 y su orden también son contrato del CSV"
    );
    // Y ninguno de ellos puede haberse colado en el conjunto que `pins-4.15.json` hashea.
    for n in &nuevos {
        assert!(
            !all.contains(n),
            "{n} está en projection_cases_all(): eso obliga a regenerar pins-4.15.json"
        );
    }
}

/// **Anclas derivadas a mano** de P7 y P9. El hash prueba que el motor es reproducible; esto
/// prueba que arranca donde debe. Las derivaciones completas están en el comentario de cada caso
/// en `tests/common/cases.rs` — aquí van solo los números y su aritmética.
#[test]
fn p7_and_p9_are_anchored_by_hand_derived_numbers() {
    let all = projection_cases_all();
    let get = |name: &str| {
        all.iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} debe existir en la batería"))
    };

    // ---- P7 --------------------------------------------------------------------------------
    let p7 = get("P7_jubilado_pension_impuestos");
    let ft = p7
        .input
        .fire_target
        .as_ref()
        .expect("P7 lleva objetivo FIRE");

    // Objetivo(0) = gross_up_ES((2.000 − 800)·12) / 0,035.
    //   need neta   = 14.400
    //   tramo 19 %  : 6.000 brutos netean 6.000 − 1.140 = 4.860 → faltan 9.540 netos
    //   tramo 21 %  : 9.540 / 0,79 = 12.075,949367…  ⇒ G = 18.075,949367…
    //   objetivo    = 18.075,949367… / 0,035 = 516.455,696202…  → 516.455,70 €
    let target0 = futurefin_engine::fire_target_at_month_index(Some(ft), 0)
        .expect("P7 tiene objetivo en el mes 0");
    assert_eq!(
        target0.round_dp(2),
        Decimal::new(51_645_570, 2),
        "el objetivo FIRE del mes 0 de P7 debería ser 516.455,70 € (gross-up ES de 14.400 € netos \
         anuales, dividido por un SWR del 3,5 %)"
    );
    let liquid0: Decimal = p7.input.assets.iter().map(|a| a.value).sum();
    assert!(
        liquid0 >= target0,
        "P7 deja de estar jubilado en el mes 0 ({liquid0} < {target0}) — el caso perdería \
         exactamente el camino que existe para cubrir"
    );

    // Caja del mes 1 con f(0) = 1, sin deuda ni planning: 800 − 2.000 = −1.200 €.
    let fma7 = first_month_allocation(&p7.input).expect("P7 resuelve su mes 1");
    assert_eq!(fma7.debt_service, Decimal::ZERO, "P7 no tiene pasivos");
    assert_eq!(
        fma7.recurring_net,
        Decimal::from(-1_200),
        "pensión 800 − gasto de jubilación 2.000 = −1.200 €/mes"
    );
    assert_eq!(fma7.base_cash, Decimal::from(-1_200));
    assert_eq!(
        fma7.per_asset,
        vec![Decimal::ZERO],
        "con caja negativa la cascada no reparte nada"
    );
    assert_eq!(fma7.leftover, Decimal::ZERO);

    // ---- P9 --------------------------------------------------------------------------------
    let p9 = get("P9_hogar_realista");
    let fma9 = first_month_allocation(&p9.input).expect("P9 resuelve su mes 1");

    // Servicio de deuda del mes 1:
    //   hipoteca francesa 180.000 € al TIN 2,9 % → interés 180.000·0,029/12 = 435,00 € exactos;
    //   saldo con interés 180.435 € > 900 ⇒ caja = 900 €.
    //   préstamo sin interés 6.000 € con plan vivo ⇒ caja = min(200, 6.000) = 200 €.
    assert_eq!(
        fma9.debt_service,
        Decimal::from(1_100),
        "cuota de la hipoteca (900) + cuota del préstamo al consumo (200)"
    );
    // Neto recurrente = 4.200 − 2.600 − 1.100 = 500 €; planning[0] = 0 ⇒ caja = 500 €.
    assert_eq!(fma9.recurring_net, Decimal::from(500));
    assert_eq!(fma9.planning_component, Decimal::ZERO);
    assert_eq!(fma9.base_cash, Decimal::from(500));
    // Cascada: 300 fijos a bonos (hueco 8.000), 60 % de los 200 restantes a RV (120), resto a la
    // cuenta corriente (80).
    assert_eq!(
        fma9.per_asset,
        vec![
            Decimal::from(80),  // 0 cuenta corriente (remainder)
            Decimal::from(300), // 1 fondo de bonos (fija, con tope)
            Decimal::from(120), // 2 renta variable (60 % del sobrante restante)
            Decimal::ZERO,      // 3 vivienda (ilíquida, ninguna regla la apunta)
            Decimal::ZERO,      // 4 cripto
        ],
        "la cascada del mes 1 de P9"
    );
    assert_eq!(
        fma9.leftover,
        Decimal::ZERO,
        "el `remainder` se lo lleva todo: nada queda varado"
    );
    // El tope de la regla fija es absoluto (20.000 €) y en el mes 1 el fondo vale 12.000 €.
    assert_eq!(fma9.rules[0].cap_ceiling, Some(Decimal::from(20_000)));
    assert_eq!(fma9.rules[0].cap_room, Some(Decimal::from(8_000)));

    // Y el hogar NO está jubilado en el mes 0: 77.000 € líquidos (20.000 de cuenta + 12.000 de
    // bonos + 40.000 de RV + 5.000 de cripto; la vivienda no cuenta, #143) contra un objetivo que
    // además arrastra el término de deuda de dos préstamos.
    let target9 = futurefin_engine::fire_target_at_month_index(p9.input.fire_target.as_ref(), 0)
        .expect("P9 tiene objetivo en el mes 0");
    let liquid9: Decimal = p9
        .input
        .assets
        .iter()
        .filter(|a| a.is_liquid)
        .map(|a| a.value)
        .sum();
    assert_eq!(liquid9, Decimal::from(77_000));
    assert!(
        liquid9 < target9,
        "P9 no puede arrancar jubilado ({liquid9} ≥ {target9})"
    );
}

/// **Anclas derivadas a mano de los casos de 5.0.0** (P14–P17), gemelas de las de P7/P9: el hash
/// prueba que el motor es reproducible; esto prueba que cada caso ejercita LA REGLA QUE DICE
/// ejercitar. Sin ellas, un caso que dejara de recortar —o que nunca disparase un guardarraíl—
/// pasaría el pin perfectamente el día que alguien lo regenerase.
#[test]
fn the_5_0_cases_are_anchored_by_hand_derived_numbers() {
    let all = projection_cases_5_0();
    let get = |name: &str| {
        all.iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} debe existir en la batería de 5.0.0"))
    };
    let dec = |v: &str| v.parse::<Decimal>().unwrap();

    // ---- P14: el techo numérico de #209 ----------------------------------------------------
    // Sin impuestos, `fixed_real` y 1.000 €/mes de déficit: la venta es 1.000 € clavados todos
    // los meses, y lo que este caso demuestra es que los 840 corren — el producto `b·v` de la
    // base de coste desborda hacia el mes 136 y antes de WP2 esto PANICABA.
    let p14 = project_net_worth_series(&get("P14_techo_numeric").input).expect("P14 simula");
    for k in [1usize, 136, 137, 840] {
        assert_eq!(p14.withdrawal[k], Decimal::from(1_000), "P14 mes {k}");
    }
    assert!(p14.withdrawal_shortfall.iter().all(|v| *v == Decimal::ZERO));
    assert_eq!(p14.uncovered_deficit_total, Decimal::ZERO);
    assert!(
        p14.net_worth[840] > Decimal::from(1_000_000_000_000i64),
        "un activo de 1e14 al 20 % durante 70 años no puede acabar pequeño: {}",
        p14.net_worth[840]
    );

    // ---- P15: el techo BRUTO con `g` mixta -------------------------------------------------
    // Mes 1: permitido = 4 %·400.000/12 = **1.333,3333 € BRUTOS** (R9). La venta entera cabe en
    // el primer activo (g = 0,2, capacidad 150.000), así que la base anual gravable es
    // 12·1.333,33·0,2 = 3.200 € — dentro del tramo del 19 % ⇒ impuesto 608 €/año ⇒ neto anual
    // 16.000 − 608 = 15.392 ⇒ **1.282,6667 €/mes**.
    // Necesidad = 2.300 − 900 = 1.400 ⇒ recorte = 1.400 − 1.282,6667 = **117,3333**.
    let p15 =
        project_net_worth_series(&get("P15_percent_of_balance_ceiling").input).expect("P15 simula");
    assert_eq!(p15.retirement_month_index, Some(1));
    assert_eq!(p15.withdrawal[1].round_dp(4), dec("1282.6667"));
    assert_eq!(p15.withdrawal_shortfall[1].round_dp(4), dec("117.3333"));
    assert_eq!(
        p15.withdrawal[1] + p15.withdrawal_shortfall[1],
        Decimal::from(1_400),
        "retirada + recorte = necesidad, exacto"
    );
    assert!(
        p15.withdrawal_excess.iter().all(|v| *v == Decimal::ZERO),
        "ceiling no gasta de más"
    );
    assert_eq!(
        p15.uncovered_deficit_total.round_dp(8),
        Decimal::ZERO,
        "el recorte NO es descubierto: con 400.000 € vendibles no falta un euro por vender"
    );

    // ---- P16: la regla ES el gasto, y el latch de `hybrid` ---------------------------------
    // Mes 1: permitido = 5 %·500.000/12 = **2.083,3333** (sin impuestos, neto = bruto). El hogar
    // tiene SUPERÁVIT (1.800 − 1.500 = +300), así que la necesidad es 0 y todo lo vendido es
    // sobrante que se gasta: `withdrawal_excess[1] == withdrawal[1]`.
    let p16 = project_net_worth_series(&get("P16_hybrid_rule_is_spend").input).expect("P16 simula");
    assert_eq!(p16.withdrawal[1].round_dp(4), dec("2083.3333"));
    assert_eq!(p16.withdrawal_excess[1], p16.withdrawal[1]);
    assert_eq!(p16.withdrawal_shortfall[1], Decimal::ZERO);
    // El latch: la razón retirada/líquido(k−1) pasa de 5 %/12 a 3,5 %/12 UNA sola vez y no vuelve.
    let ratio = |k: usize| (p16.withdrawal[k] / p16.liquid_worth[k - 1]).round_dp(8);
    let inicial = dec("0.00416667"); // 5 %/12
    let final_ = dec("0.00291667"); // 3,5 %/12
    let latch = (1..p16.withdrawal.len())
        .find(|&k| ratio(k) == final_)
        .expect("el latch de hybrid tiene que dispararse dentro del horizonte");
    assert_eq!(latch, 156, "el mes del latch de P16");
    for k in 1..p16.withdrawal.len() {
        assert_eq!(
            ratio(k),
            if k < latch { inicial } else { final_ },
            "P16 mes {k}: la regla solo cambia una vez, en el latch"
        );
    }

    // ---- P17: los guardarraíles ------------------------------------------------------------
    // Mes 1: `W_R` = 4 %·700.000/12 = **2.333,3333 € BRUTOS**; con la escala ES y g = 1 el
    // impuesto anual sobre 28.000 € es 6.000·19 % + 22.000·21 % = 1.140 + 4.620 = 5.760 ⇒ neto
    // anual 22.240 ⇒ **1.853,3333 €/mes**. Necesidad 2.600 ⇒ recorte **746,6667**.
    let p17 = project_net_worth_series(&get("P17_guardrails_taxes_es").input).expect("P17 simula");
    assert_eq!(p17.withdrawal[1].round_dp(4), dec("1853.3333"));
    assert_eq!(p17.withdrawal_shortfall[1].round_dp(4), dec("746.6667"));
    assert_eq!(
        p17.withdrawal[1] + p17.withdrawal_shortfall[1],
        Decimal::from(2_600)
    );
    // La retirada se INDEXA al IPC, así que sube todos los meses… salvo cuando un guardarraíl
    // recorta. El primer recorte es el de capital-preservation, y cae donde la cuenta a mano dice:
    // la tasa efectiva `12·W_R·f(k−1)/L(k−1)` arranca en el 4 % y crece al 2,5 % anual (el líquido
    // apenas se mueve), así que cruza el 4,8 % de la banda hacia el mes 92 — pero las revisiones
    // solo ocurren cada 12 meses desde R = 1, y la primera posterior es **k = 97**.
    let cortes: Vec<usize> = (2..p17.withdrawal.len())
        .filter(|&k| p17.withdrawal[k] < p17.withdrawal[k - 1])
        .collect();
    assert_eq!(
        cortes.first().copied(),
        Some(97),
        "el primer recorte de guardarraíl de P17 (cortes: {cortes:?})"
    );
    for k in &cortes {
        assert_eq!(
            (k - 1) % 12,
            0,
            "un guardarraíl solo puede moverse en una revisión anual desde R = 1 (mes {k})"
        );
    }
    assert!(
        cortes.len() >= 5,
        "P17 existe para pinear MUCHAS revisiones, no una: {cortes:?}"
    );
    assert!(p17.withdrawal_excess.iter().all(|v| *v == Decimal::ZERO));
}

/// **REGRESIÓN de la issue #208** (era DIANA `#[ignore]` en WP0: entonces PANICABA; el arreglo
/// de WP1a la convierte en la red que impide que vuelva).
///
/// `gross_up_mixed_monthly` calcula `(techo_del_tramo_fiscal − base) / g` para topar la venta de
/// un tramo, y solo se protege con `if g > Decimal::ZERO`. Esa guarda no basta: con una `g`
/// positiva pero DENORMAL el cociente desborda el rango de `Decimal` (~7,9e28) y `rust_decimal`
/// **panica** («Division overflowed»). El motor es una función pura y no debe panicar con un
/// input que la API acepta — en producción el pool blocking lo publica como un 400 `task_panic`
/// ininteligible, exactamente el precedente que ya forzó `checked_mul` en el crecimiento de
/// activos (`EngineError::AssetValueOverflow`).
///
/// **La `g` denormal no es rebuscada: el propio motor la fabrica.** Una cuenta al 0 % que la
/// cascada alimenta tiene `b` pegada a `v` (cada euro aportado sube las dos), el drenaje conserva
/// el cociente `b/v` y un 0 % no vuelve a abrir hueco nunca; tras un drenaje fuerte queda
/// `g = 1 − b/v ≈ 1e-27`. El caso P9 con 8.000 € en la cuenta (en vez de los 20.000 € que lleva)
/// panica así en el mes 138 — ver el comentario del activo 0 en `tests/common/cases.rs`.
///
/// Cota medida: con `g = 1e-20` no desborda (200.000/1e-20 = 2e25); con `g = 1e-27`, sí.
///
/// **Arreglo (WP1a):** `checked_div` en los dos topes del solver mixto — un cociente que no cabe
/// en `Decimal` significa «este tope no ata», y el techo real lo sigue poniendo la capacidad del
/// activo (`capacity_monthly`). El caso `P13_cash8k_denormal_g` del pin dorado cubre el mismo
/// mecanismo dentro de una proyección completa de 840 meses.
#[test]
fn mixed_drawdown_must_not_panic_on_a_denormal_gain_ratio() {
    let mut input = cases::base_input(
        3,
        Decimal::ZERO,
        Decimal::from(1_000), // déficit puro: obliga a drenar el primer mes
        vec![
            // g = 1 − b/v = 1e-27: positiva, pero denormal.
            cases::mk_asset_with_basis(
                1,
                Decimal::ONE,
                true,
                None,
                Decimal::ONE - Decimal::new(1, 27),
            ),
            // Segundo activo con otra `g` (sin coste declarado ⇒ el escalar): sin mezcla el
            // motor cortocircuita a la vía escalar y el solver mixto ni se llama.
            cases::mk_asset(2, Decimal::from(10_000), true, Some(Decimal::from(5))),
        ],
        vec![],
    );
    input.tax_brackets = cases::es_tax_brackets_2025_26();
    input.taxes_enabled = true;

    let out = project_net_worth_series(&input)
        .expect("una gain ratio denormal no debe hacer fallar la simulación");
    assert_eq!(out.net_worth.len(), 4);

    // Y no basta con «no panica»: el drenaje TIENE que ocurrir. Un `checked_div` que devolviese
    // `None` y abortase el tramo dejaría la venta en cero y el test seguiría verde en su
    // ausencia de pánico mientras el hogar se queda sin cubrir su gasto.
    assert_eq!(
        out.per_asset_series[0][1],
        Decimal::ZERO,
        "el activo denormal (1 €) se vende entero el primer mes"
    );
    for k in 1..=3usize {
        assert!(
            out.per_asset_series[1][k] < out.per_asset_series[1][k - 1],
            "el activo grande tiene que ADELGAZAR cada mes (mes {k}: {} → {})",
            out.per_asset_series[1][k - 1],
            out.per_asset_series[1][k]
        );
        assert!(
            out.net_worth[k] < out.net_worth[k - 1],
            "el patrimonio tiene que bajar cada mes (mes {k})"
        );
    }
    assert_eq!(
        out.uncovered_deficit_total,
        Decimal::ZERO,
        "con 10.001 € vendibles y 1.000 €/mes de déficit no puede quedar descubierto"
    );
    assert_eq!(
        out.assets_depleted_month_index, None,
        "y la cartera no se agota en 3 meses"
    );
}

// =============================================================================================
// Pin ADITIVO de las salidas nuevas de 5.0.0 (§B.8)
// =============================================================================================
//
// **Fixture aparte a propósito.** `pins-4.15.json` existe para demostrar UNA cosa: que el
// refactor no movió las salidas que 4.15.0 ya publicaba. Meter en su hash los campos nuevos lo
// haría cambiar en cada WP que añada una lectura, y entonces dejaría de poder decir «esto es
// idéntico a 4.15.0» — que es todo lo que se le pide. Por eso las lecturas de fase viven en
// `pins-5.0-outputs.json`, con su propio hash y su propia variable de regeneración.
//
// Regenerar cuando el cambio es INTENCIONADO (y solo este fichero; `UPDATE_ENGINE_PINS` sigue
// siendo el de 4.15.0 y no se toca):
//
// ```text
// UPDATE_ENGINE_PINS_5_0=1 cargo test -p futurefin-engine --test golden_pins
// ```
//
// Cubre los casos de PROYECCIÓN (P*): los L* son calendarios de amortización y no tienen fases.

fn phase_tag(p: Phase) -> &'static str {
    match p {
        Phase::Accumulating => "accumulating",
        Phase::Partial => "partial",
        Phase::Retired => "retired",
    }
}

/// **Capa WP1b/WP2** del texto canónico: las lecturas de fase y las tres series de retirada, mes
/// a mes con el `Decimal` COMPLETO (`Display`).
///
/// Se mantiene como función APARTE cuando WP3 amplió la canonicalización, y no por estética: es
/// lo que permite demostrar que los campos VIEJOS no se movieron aunque el hash del fichero sí lo
/// haya hecho (ver `the_5_0_canonicalization_grew_without_moving_the_old_fields`). Un pin que
/// crece y se regenera sin ese control no distingue «añadí campos» de «cambié números».
fn render_projection_outputs_wp2(name: &str, o: &ProjectionOutput) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "case {name}");
    let _ = writeln!(out, "kind projection_outputs_5_0");
    let _ = writeln!(
        out,
        "retirement_month_index {}",
        opt(o.retirement_month_index)
    );
    let _ = writeln!(
        out,
        "liquid_crossing_month_index {}",
        opt(o.liquid_crossing_month_index)
    );
    let _ = writeln!(
        out,
        "pension_start_month_index {}",
        opt(o.pension_start_month_index)
    );
    let _ = writeln!(
        out,
        "partial_retirement_month_index {}",
        opt(o.partial_retirement_month_index)
    );
    for (i, (phase, k)) in o.phase_transitions.iter().enumerate() {
        let _ = writeln!(out, "phase {i} {} {k}", phase_tag(*phase));
    }
    for (k, v) in o.withdrawal.iter().enumerate() {
        let _ = writeln!(out, "wd {k} {v}");
    }
    for (k, v) in o.withdrawal_shortfall.iter().enumerate() {
        let _ = writeln!(out, "wds {k} {v}");
    }
    for (k, v) in o.withdrawal_excess.iter().enumerate() {
        let _ = writeln!(out, "wde {k} {v}");
    }
    let _ = writeln!(out, "warnings {}", o.warnings.len());
    for w in &o.warnings {
        let _ = writeln!(out, "warning {w:?}");
    }
    out
}

/// El texto canónico COMPLETO: la capa WP1b/WP2 **seguida** de la de WP3 (§B.3/§B.7). El orden es
/// contrato: la capa vieja es un PREFIJO exacto del texto nuevo, y de eso vive el control de
/// «creció sin moverse».
fn render_projection_outputs_5_0(name: &str, o: &ProjectionOutput) -> String {
    let mut out = render_projection_outputs_wp2(name, o);
    let _ = writeln!(
        out,
        "bridge_effective_withdrawal_pct {}",
        opt(o.bridge_effective_withdrawal_pct)
    );
    let _ = writeln!(
        out,
        "pension_coverage_ratio {}",
        opt(o.pension_coverage_ratio)
    );
    let _ = writeln!(out, "partial_gap_target {}", opt(o.partial_gap_target));
    let _ = writeln!(
        out,
        "partial_phase_capital_growing {}",
        o.partial_phase_capital_growing
    );
    let _ = writeln!(out, "disposable_total {}", o.disposable_cash_total);
    for (k, v) in o.disposable_cash.iter().enumerate() {
        let _ = writeln!(out, "dc {k} {v}");
    }
    out
}

/// Lo que el fixture de 5.0.0 guarda de un caso: el hash y los titulares legibles de un diff.
struct Pin50 {
    name: String,
    sha256: String,
    retirement_month_index: Option<u32>,
    liquid_crossing_month_index: Option<u32>,
    /// Σ de la serie de retirada: un titular que se mueve si el drenaje cambia de importe aunque
    /// no cambie de mes.
    withdrawal_total: Decimal,
    /// El texto de las fases, tal cual entra en el hash («accumulating@0|retired@37»).
    phases: String,
    // ---- WP3 (§B.3/§B.7) ----------------------------------------------------------------
    pension_start_month_index: Option<u32>,
    partial_retirement_month_index: Option<u32>,
    disposable_cash_total: Decimal,
    /// Los literales públicos de los avisos, separados por `|` («retire_at_age_underfunded»).
    warnings: String,
    /// El hash de la capa WP1b/WP2 SOLA. No entra en el hash del caso: existe para que el diff
    /// diga si lo que se movió son los campos viejos o solo los nuevos.
    sha256_wp2: String,
}

/// La batería del pin de 5.0.0 es la de 4.15.0 **más** los casos que WP2 añadió
/// (`projection_cases_5_0`). Crece por aquí y solo por aquí: `projection_cases_all()` no puede
/// crecer sin regenerar `pins-4.15.json`, que existe para no moverse.
fn cases_5_0() -> Vec<cases::ProjCase> {
    let mut out = projection_cases_all();
    out.extend(projection_cases_5_0());
    out
}

fn live_pins_5_0() -> Vec<Pin50> {
    cases_5_0()
        .into_iter()
        .map(|c| {
            let out = project_net_worth_series(&c.input)
                .unwrap_or_else(|e| panic!("el caso {} no debe fallar: {e}", c.name));
            let text = render_projection_outputs_5_0(c.name, &out);
            Pin50 {
                name: c.name.to_string(),
                sha256: sha256_hex(&text),
                retirement_month_index: out.retirement_month_index,
                liquid_crossing_month_index: out.liquid_crossing_month_index,
                withdrawal_total: out.withdrawal.iter().copied().sum(),
                phases: out
                    .phase_transitions
                    .iter()
                    .map(|(p, k)| format!("{}@{k}", phase_tag(*p)))
                    .collect::<Vec<_>>()
                    .join("|"),
                pension_start_month_index: out.pension_start_month_index,
                partial_retirement_month_index: out.partial_retirement_month_index,
                disposable_cash_total: out.disposable_cash_total,
                warnings: out
                    .warnings
                    .iter()
                    .map(|w| w.code())
                    .collect::<Vec<_>>()
                    .join("|"),
                sha256_wp2: sha256_hex(&render_projection_outputs_wp2(c.name, &out)),
            }
        })
        .collect()
}

fn fixture_path_5_0() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pins-5.0-outputs.json")
}

const DOC_5_0: &str = "Pin ADITIVO de las salidas nuevas del motor 5.0.0 (WP1b, seccion B.8 del \
plan de la issue #207): SHA-256 del texto canonico de retirement_month_index, \
liquid_crossing_month_index, pension_start_month_index, partial_retirement_month_index, \
phase_transitions y las tres series withdrawal / withdrawal_shortfall / withdrawal_excess mes a \
mes con el Decimal completo via Display, mas el recuento de warnings. Cubre los casos de \
PROYECCION de tests/common/cases.rs (los L* son calendarios de amortizacion y no tienen fases). \
Vive APARTE de pins-4.15.json a proposito: aquel demuestra que las salidas de 4.15.0 no se \
movieron y dejaria de poder demostrarlo si creciera con cada lectura nueva. GENERADO: no editar \
a mano. Regenerar SOLO si el cambio es intencionado: UPDATE_ENGINE_PINS_5_0=1 cargo test -p \
futurefin-engine --test golden_pins, y documentar el delta en el CHANGELOG \
(futurefin-change-control). Cubre projection_cases_all() (los casos de 4.15.0) MAS \
projection_cases_5_0() (P14-P17: el techo numerico de la issue #209 y las tres reglas de retirada \
nuevas; P18-P23: pension con fecha, objetivo puente, media jornada, cruce como lectura, techo de \
aportacion y pausa de ingresos). En los casos con fixed_real withdrawal_shortfall y \
withdrawal_excess son cero por construccion — el permitido ES la necesidad — y ahi es donde este \
pin demuestra que WP2 no movio la semantica de 4.15.0. WP3 AMPLIO la canonicalizacion con \
bridge_effective_withdrawal_pct, pension_coverage_ratio, partial_gap_target, \
partial_phase_capital_growing y la serie disposable_cash mes a mes: por eso el sha256 de los 17 \
casos anteriores cambio SIN que cambiara ningun numero suyo, y quien lo demuestra es el test \
the_5_0_canonicalization_grew_without_moving_the_old_fields (rehashea la capa vieja sola contra \
los SHA-256 de antes de WP3).";

fn render_fixture_5_0(pins: &[Pin50]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    let _ = writeln!(s, "  \"_doc\": \"{DOC_5_0}\",");
    s.push_str("  \"engine_version\": \"5.0.0\",\n");
    s.push_str(
        "  \"hash_algo\": \"sha256 over the canonical text described in golden_pins.rs\",\n",
    );
    s.push_str("  \"cases\": {\n");
    for (i, p) in pins.iter().enumerate() {
        let comma = if i + 1 == pins.len() { "" } else { "," };
        let _ = writeln!(s, "    \"{}\": {{", p.name);
        let _ = writeln!(s, "      \"sha256\": \"{}\",", p.sha256);
        let _ = writeln!(
            s,
            "      \"retirement_month_index\": {},",
            json_u32(p.retirement_month_index)
        );
        let _ = writeln!(
            s,
            "      \"liquid_crossing_month_index\": {},",
            json_u32(p.liquid_crossing_month_index)
        );
        let _ = writeln!(
            s,
            "      \"withdrawal_total\": {},",
            json_dec(Some(p.withdrawal_total))
        );
        let _ = writeln!(s, "      \"phases\": \"{}\",", p.phases);
        let _ = writeln!(
            s,
            "      \"pension_start_month_index\": {},",
            json_u32(p.pension_start_month_index)
        );
        let _ = writeln!(
            s,
            "      \"partial_retirement_month_index\": {},",
            json_u32(p.partial_retirement_month_index)
        );
        let _ = writeln!(
            s,
            "      \"disposable_cash_total\": {},",
            json_dec(Some(p.disposable_cash_total))
        );
        let _ = writeln!(s, "      \"warnings\": \"{}\"", p.warnings);
        let _ = writeln!(s, "    }}{comma}");
    }
    s.push_str("  }\n");
    s.push_str("}\n");
    s
}

/// El pin de las salidas nuevas. Misma mecánica que [`golden_pins_match_4_15_0`], fichero aparte.
#[test]
fn golden_pins_5_0_outputs_match() {
    let pins = live_pins_5_0();
    let generated = render_fixture_5_0(&pins);

    if std::env::var("UPDATE_ENGINE_PINS_5_0").is_ok() {
        std::fs::write(fixture_path_5_0(), &generated).expect("escribir el fixture de pins 5.0");
        eprintln!(
            "pins-5.0-outputs.json regenerado con {} casos — REVISA EL DIFF y documenta el delta \
             en el CHANGELOG antes de mergear.",
            pins.len()
        );
        return;
    }

    let raw = std::fs::read_to_string(fixture_path_5_0()).unwrap_or_else(|e| {
        panic!(
            "no se puede leer {}: {e}. Si es la primera vez, genera el fixture con \
             UPDATE_ENGINE_PINS_5_0=1 cargo test -p futurefin-engine --test golden_pins",
            fixture_path_5_0().display()
        )
    });
    let stored: serde_json::Value =
        serde_json::from_str(&raw).expect("el fixture de pins 5.0 debe ser JSON válido");

    let stored_cases = stored["cases"]
        .as_object()
        .expect("el fixture debe tener un objeto `cases`");

    let mut live_names: Vec<&str> = pins.iter().map(|p| p.name.as_str()).collect();
    let mut stored_names: Vec<&str> = stored_cases.keys().map(|k| k.as_str()).collect();
    live_names.sort_unstable();
    stored_names.sort_unstable();
    assert_eq!(
        live_names, stored_names,
        "la batería de casos y el fixture de 5.0.0 no cubren los mismos casos. Si has añadido o \
         retirado un caso a propósito, regenera con UPDATE_ENGINE_PINS_5_0=1"
    );

    let mut report = String::new();
    for p in &pins {
        let stored_case = &stored_cases[&p.name];
        if stored_case["sha256"].as_str() == Some(p.sha256.as_str()) {
            continue;
        }
        let _ = writeln!(report, "  · caso {}", p.name);
        let _ = writeln!(
            report,
            "      sha256                       {} → {}",
            stored_case["sha256"].as_str().unwrap_or(""),
            p.sha256
        );
        for (field, live) in [
            ("retirement_month_index", json_u32(p.retirement_month_index)),
            (
                "liquid_crossing_month_index",
                json_u32(p.liquid_crossing_month_index),
            ),
            ("withdrawal_total", json_dec(Some(p.withdrawal_total))),
            ("phases", format!("\"{}\"", p.phases)),
            (
                "pension_start_month_index",
                json_u32(p.pension_start_month_index),
            ),
            (
                "partial_retirement_month_index",
                json_u32(p.partial_retirement_month_index),
            ),
            (
                "disposable_cash_total",
                json_dec(Some(p.disposable_cash_total)),
            ),
            ("warnings", format!("\"{}\"", p.warnings)),
        ] {
            let before = &stored_case[field];
            let before_txt = match before {
                serde_json::Value::String(s) => format!("\"{s}\""),
                other => other.to_string(),
            };
            let marker = if before_txt == live { "  " } else { "≠ " };
            let _ = writeln!(report, "      {marker}{field:<26} {before_txt} → {live}");
        }
    }

    assert!(
        report.is_empty(),
        "las LECTURAS de fase de 5.0.0 cambiaron. Si el cambio es intencional, regenera con \
         UPDATE_ENGINE_PINS_5_0=1 y documenta el delta en el CHANGELOG.\n\nCasos que se movieron \
         (guardado → vivo):\n{report}"
    );
}

/// **La canonicalización de 5.0.0 CRECIÓ en WP3, y este test demuestra que solo creció.**
///
/// `pins-5.0-outputs.json` se regeneró al añadir §B.3/§B.7 (pensión, puente, media jornada, caja
/// disponible), así que su hash por caso cambió para los 17 casos que ya existían. Un pin
/// regenerado no distingue por sí solo «añadí campos» de «moví números»: sin este control, el día
/// que alguien rompa el drenaje y regenere el fichero, el diff dirá exactamente lo mismo que hoy.
///
/// La prueba es en DOS ETAPAS: el texto canónico nuevo tiene el viejo como **prefijo exacto**
/// (`render_projection_outputs_5_0` = `render_projection_outputs_wp2` + la capa WP3), así que se
/// vuelve a hashear la capa vieja sola y se compara contra los SHA-256 que el fichero guardaba
/// **antes** de WP3, copiados aquí literalmente. Si los campos de WP1b/WP2 se hubieran movido un
/// dígito, esta lista no cuadraría.
///
/// Los seis casos de WP3 (P18–P23) no aparecen: no existían antes, no tienen valor anterior, y
/// ponerles uno inventado sería exactamente el número sin verificar que esta casa no publica.
#[test]
fn the_5_0_canonicalization_grew_without_moving_the_old_fields() {
    // Copiado de `pins-5.0-outputs.json` en el commit anterior a WP3.
    const BEFORE_WP3: &[(&str, &str)] = &[
        ("P1_deficit_cronico", "cd6cf9782c9c3acd96ad570bb083329cac0b31423ca307766d78c4f8234de6ed"),
        ("P2_fire_mes0", "d605c5913cbfae026ada0b8f2a091ecd1b872e034d430616cbc94baf75bafbdd"),
        ("P3_superavit_jubilacion", "d503198bec6b501f9d51190088829998a181734865ed58cdd2d930b53999a3af"),
        ("P4_ret_menos100", "00c7d139b6ebe6028e536877cff5eccdb043fd80070693243209b6ddcf43422d"),
        ("P5_flat_nominal_30y", "9d7f4c00daa1631505ff9bb3ac17744e641fc1bf7d177a9482346a17a35a667f"),
        ("P6_venc_saldo_vivo_proj", "20ca98734ba7e5046ebe66c27977b79a8e747ad929f3481f4b5c208502d54c9f"),
        ("P13_cash8k_denormal_g", "8a0506535261b28d1def11871bf651ea7cbc4041ebd46949d85329f294939fad"),
        ("P7_jubilado_pension_impuestos", "5976365c10d39b9c97d170c11282395f15132089b0928b9f8e49cdf744ed4a13"),
        ("P8_drenaje_g_mixta", "aa13136c4690ae18f0db0f4141226ec71b82d5c63f82190f72533d1c6944c19a"),
        ("P9_hogar_realista", "ece489a9a4d555464c2530c75df39768dc0b5bafdcceac2023634a2e87d7ceba"),
        ("P10_jubilacion_forzada", "44ec6ec48a0e1911e3b264f103be52913f7c4ee2bc8ce923f8a83953d83974c8"),
        ("P11_deflacion_negativa", "e279575d83d2a3265cf199fe5ac1acf279736d27a050fe54a73ad523f8a2a5ba"),
        ("P12_topes_de_cascada", "876813e073579eb47b4cfa3f5592be2c0c2bf7d897ae3adfaac39b5d351a37d4"),
        ("P14_techo_numeric", "0a8803038f38fc4ee531ad5c6845d8b19c642a1311e78b993ffa09bf10b6fa87"),
        ("P15_percent_of_balance_ceiling", "9fe43f5bc701e3077961cd405de4b0deeefcc4886ace32df95f269b098113e74"),
        ("P16_hybrid_rule_is_spend", "16e21d418e47d904ea6e5d20b50bbdf862697706d1ae431f1f4afee377a90c38"),
        ("P17_guardrails_taxes_es", "12af97cd287d66ef58d3ea050f2ceca0667c3fcbf1caa481f37a22f545407b05"),
    ];

    let live = live_pins_5_0();
    let mut moved = Vec::new();
    for (name, before) in BEFORE_WP3 {
        let p = live
            .iter()
            .find(|p| p.name == *name)
            .unwrap_or_else(|| panic!("{name} debe seguir en la batería"));
        if p.sha256_wp2 != *before {
            moved.push(format!("  · {name}: {before} → {}", p.sha256_wp2));
        }
    }
    assert!(
        moved.is_empty(),
        "WP3 movió campos de la canonicalización VIEJA (fases y series de retirada), no solo \
         añadió los suyos. Eso no es «el pin creció»: es una regresión.\n{}",
        moved.join("\n")
    );

    // Y el control de vida: la capa vieja tiene que ser de verdad un PREFIJO de la nueva. Sin
    // esto, cambiar el orden de las secciones haría que este test comparase otra cosa y siguiera
    // pasando — la deriva silenciosa del grep vacío, aplicada a un hash.
    let case = cases_5_0()
        .into_iter()
        .find(|c| c.name == "P9_hogar_realista")
        .expect("P9 en la batería");
    let out = project_net_worth_series(&case.input).expect("P9 simula");
    let wp2 = render_projection_outputs_wp2(case.name, &out);
    let full = render_projection_outputs_5_0(case.name, &out);
    assert!(
        full.starts_with(&wp2),
        "el texto de WP3 tiene que empezar por el de WP2, o este test no compara lo que dice"
    );
    assert!(full.len() > wp2.len(), "y tiene que haber crecido");
}

/// Control negativo del pin nuevo, gemelo de [`the_hash_actually_notices_a_single_moved_decimal`]:
/// un detector sin control negativo es un test que siempre pasa.
#[test]
fn the_5_0_hash_notices_a_moved_withdrawal_and_a_moved_phase() {
    let case = projection_cases_all()
        .into_iter()
        .find(|c| c.name == "P10_jubilacion_forzada")
        .expect("P10 debe existir en la batería");
    let out = project_net_worth_series(&case.input).expect("P10 simula");
    let baseline = sha256_hex(&render_projection_outputs_5_0(case.name, &out));

    // 1) Un céntimo de céntimo en la serie de retirada.
    let mut mutated = out.clone();
    let k = mutated.withdrawal.len() / 2;
    mutated.withdrawal[k] += Decimal::new(1, 10);
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_outputs_5_0(case.name, &mutated)),
        "mover 1e-10 en withdrawal[{k}] no cambió el hash"
    );

    // 2) El mes de jubilación.
    let mut mutated = out.clone();
    mutated.retirement_month_index = mutated.retirement_month_index.map(|m| m + 1);
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_outputs_5_0(case.name, &mutated)),
        "mover el mes de jubilación no cambió el hash"
    );

    // 3) Una fase de más (lo que WP3 insertará entre las dos de hoy).
    let mut mutated = out.clone();
    mutated.phase_transitions.insert(1, (Phase::Partial, 12));
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_outputs_5_0(case.name, &mutated)),
        "insertar una fase no cambió el hash"
    );

    // 4) Un recorte donde hoy hay cero: la serie informativa está pineada, no solo declarada.
    let mut mutated = out;
    mutated.withdrawal_shortfall[k] += Decimal::new(1, 10);
    assert_ne!(
        baseline,
        sha256_hex(&render_projection_outputs_5_0(case.name, &mutated)),
        "mover el recorte no cambió el hash"
    );
}

/// **Invariante de las lecturas de fase en WP1b** (§C del plan, versión reducida a dos fases).
/// El pin dice «esto no se ha movido»; esto dice «esto significa lo que dice que significa» —
/// y sin él, un `retirement_month_index` que se quedase pegado a `None` pasaría el pin
/// perfectamente el día que alguien lo regenerase.
#[test]
fn the_phase_readings_agree_with_the_series_they_describe() {
    for c in cases_5_0() {
        let out = project_net_worth_series(&c.input)
            .unwrap_or_else(|e| panic!("el caso {} no debe fallar: {e}", c.name));
        let name = c.name;

        // 1) Las tres series de retirada tienen la longitud de la serie de patrimonio, y el mes 0
        //    (estado inicial, no simulado) es cero en las tres.
        for (label, serie) in [
            ("withdrawal", &out.withdrawal),
            ("withdrawal_shortfall", &out.withdrawal_shortfall),
            ("withdrawal_excess", &out.withdrawal_excess),
        ] {
            assert_eq!(
                serie.len(),
                out.net_worth.len(),
                "{name}: {label} no tiene la longitud de net_worth"
            );
            assert_eq!(serie[0], Decimal::ZERO, "{name}: {label}[0] debe ser 0");
        }

        // 2) Las tres magnitudes de B.1.5, según la regla y el modo:
        //    · `fixed_real` no recorta ni gasta de más — el permitido ES la necesidad, en
        //      CUALQUIERA de los dos modos (es lo que mantiene 4.15.0 bit-idéntico);
        //    · en `ceiling` la retirada nunca supera la necesidad ⇒ sobrante 0 exacto;
        //    · el recorte y el sobrante nunca son negativos: son magnitudes, no diferencias.
        let fixed_real = matches!(c.input.phase_plan.withdrawal, WithdrawalRule::FixedReal);
        if fixed_real {
            assert!(
                out.withdrawal_shortfall.iter().all(|v| *v == Decimal::ZERO),
                "{name}: fixed_real no puede recortar"
            );
        }
        if fixed_real || c.input.phase_plan.spend_mode == SpendMode::Ceiling {
            assert!(
                out.withdrawal_excess.iter().all(|v| *v == Decimal::ZERO),
                "{name}: sin regla que gastar por encima de la necesidad, el sobrante es 0"
            );
        }
        assert!(
            out.withdrawal_shortfall.iter().all(|v| *v >= Decimal::ZERO)
                && out.withdrawal_excess.iter().all(|v| *v >= Decimal::ZERO),
            "{name}: ni el recorte ni el sobrante pueden ser negativos"
        );

        // 3) Retirada ≥ 0 SIEMPRE, y solo puede haberla en un mes de déficit: si el patrimonio
        //    líquido no bajó ese mes por otra vía, es que se vendió algo.
        assert!(
            out.withdrawal.iter().all(|v| *v >= Decimal::ZERO),
            "{name}: una retirada negativa sería una aportación disfrazada"
        );

        // 4) El descubierto total no puede ser negativo **más allá de la cola de redondeo
        //    declarada**. `MonthSale::account` conserva a propósito la expresión LITERAL de
        //    4.15.0 —`need_net − obtained_net`, sin `max(0, ·)`— porque meter el clamp movería
        //    casos pineados; y `after_tax(gross_up(n))` recupera `n` salvo el último dígito de
        //    28, así que en una jubilación larga con impuestos esos dígitos se ACUMULAN y el
        //    total puede quedar unas unidades de 1e-25 por debajo de cero.
        //
        //    Medido en esta batería: P18 (336 meses jubilado con la escala ES y `g` mixta) cae en
        //    −5e-25 €, P21 sube a +1,6e-24 €. Son 0,0000000000000000000000005 euros: por debajo
        //    de cualquier unidad monetaria representable, pero NO cero — y el umbral se escribe
        //    aquí para que un descubierto negativo DE VERDAD (un euro, un céntimo) siga cazándose.
        assert!(
            out.uncovered_deficit_total >= -Decimal::new(1, 20),
            "{name}: descubierto negativo más allá de la cola de redondeo: {}",
            out.uncovered_deficit_total
        );

        // 5) Fases (§B.1): siempre se arranca acumulando en el mes 0, la secuencia es
        //    ESTRICTAMENTE creciente en el mes y monótona en la fase, y cada índice publicado
        //    coincide con la entrada correspondiente de `phase_transitions`. WP3 metió `Partial`
        //    entre las dos de WP1b, así que el invariante se escribe sobre la lista, no sobre
        //    posiciones fijas.
        assert_eq!(
            out.phase_transitions.first().copied(),
            Some((Phase::Accumulating, 0)),
            "{name}: toda simulación arranca acumulando en el mes 0"
        );
        let rank = |p: Phase| match p {
            Phase::Accumulating => 0u8,
            Phase::Partial => 1,
            Phase::Retired => 2,
        };
        for w in out.phase_transitions.windows(2) {
            assert!(
                rank(w[1].0) > rank(w[0].0) && w[1].1 > w[0].1,
                "{name}: las fases son monótonas y no se repiten: {:?}",
                out.phase_transitions
            );
        }
        let phase_month = |p: Phase| {
            out.phase_transitions
                .iter()
                .find(|(q, _)| *q == p)
                .map(|(_, k)| *k)
        };
        assert_eq!(
            phase_month(Phase::Retired),
            out.retirement_month_index,
            "{name}: la fase jubilada empieza en el mes efectivo, y no existe si no hay"
        );
        assert_eq!(
            phase_month(Phase::Partial),
            out.partial_retirement_month_index,
            "{name}: la media jornada solo se publica si se pisó de verdad"
        );
        for k in [out.retirement_month_index, out.partial_retirement_month_index]
            .into_iter()
            .flatten()
        {
            assert!(
                k >= 1 && k <= c.input.horizon_months,
                "{name}: mes {k} fuera del horizonte"
            );
        }

        // 6) El cruce es una LECTURA. Dos regímenes, y la bandera de D17 los separa:
        //    · `crossing_is_reading_only = false` (el de 4.15.0): el cruce jubila, así que la
        //      jubilación efectiva es `min(cruce, forzado)` y nunca puede ir DESPUÉS del cruce;
        //    · `crossing_is_reading_only = true`: el cruce no jubila nada y puede quedarse solo,
        //      sin `retirement_month_index` detrás.
        if let Some(x) = out.liquid_crossing_month_index {
            if !c.input.phase_plan.crossing_is_reading_only {
                let eff = out
                    .retirement_month_index
                    .expect("si el cruce jubila y hubo cruce, hubo jubilación");
                assert!(
                    eff <= x,
                    "{name}: la jubilación efectiva ({eff}) no puede ser posterior al cruce ({x})"
                );
                if c.input
                    .phase_plan
                    .retirement_trigger
                    .forced_month()
                    .is_none()
                {
                    assert_eq!(
                        eff, x,
                        "{name}: sin trigger forzado, jubilación efectiva y cruce son el mismo mes"
                    );
                }
            }
        }

        // 7) Pensión con fecha: el mes publicado es `start_index + 1` (rejilla 0-based → bucle
        //    1-based) y solo existe si cae dentro del horizonte. Sin pensión, `None`.
        let expected_pension = c.input.phase_plan.pension.and_then(|p| {
            let m = p.start_index + 1;
            (m <= c.input.horizon_months).then_some(m)
        });
        assert_eq!(out.pension_start_month_index, expected_pension, "{name}");

        // 8) Las lecturas del puente solo existen con pensión con fecha; y la caja disponible
        //    solo con techo de aportación. Un `None` aquí NO es un cero (norma de la casa).
        if c.input.phase_plan.pension.is_none() {
            assert_eq!(out.pension_coverage_ratio, None, "{name}");
            assert_eq!(out.bridge_effective_withdrawal_pct, None, "{name}");
        }
        assert_eq!(
            out.disposable_cash.len(),
            out.net_worth.len(),
            "{name}: disposable_cash no tiene la longitud de net_worth"
        );
        assert_eq!(out.disposable_cash[0], Decimal::ZERO, "{name}");
        assert_eq!(
            out.disposable_cash.iter().copied().sum::<Decimal>(),
            out.disposable_cash_total,
            "{name}: el total es la suma de la serie"
        );
        if c.input.phase_plan.contribution_cap_monthly.is_none()
            && c.input.phase_plan.contributions_stop_month.is_none()
        {
            assert_eq!(
                out.disposable_cash_total,
                Decimal::ZERO,
                "{name}: sin techo no puede sobrar caja"
            );
        }

        // 9) `partial_phase_capital_growing` es `true` SOLO si hubo fase parcial, y el aviso de
        //    capital menguante es exactamente su negación dentro de esa fase.
        if out.partial_retirement_month_index.is_none() {
            assert!(!out.partial_phase_capital_growing, "{name}");
            assert!(
                !out.warnings
                    .contains(&EngineWarning::PartialPhaseCapitalShrinking),
                "{name}: no se puede menguar en una fase que no ocurrió"
            );
        } else {
            assert_eq!(
                out.partial_phase_capital_growing,
                !out.warnings
                    .contains(&EngineWarning::PartialPhaseCapitalShrinking),
                "{name}: crecer y el aviso de menguar son complementarios"
            );
        }
    }
}
