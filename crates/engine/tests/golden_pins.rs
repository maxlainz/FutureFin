//! **PIN DORADO DEL MOTOR 4.15.0** — la red de bit-identidad del refactor 5.0.0.
//!
//! Qué hace: para cada caso de `tests/common/cases.rs` (L1–L6 y P1–P12) canonicaliza a TEXTO
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

use cases::{liability_cases, projection_cases_all, projection_cases_audit, ref_date};
use futurefin_engine::{
    first_month_allocation, liability_amortization_schedule, project_net_worth_series,
    AllocationSkipReason, FirstMonthAllocation, LiabilityPayoffAbsence, LiabilitySchedule,
    ProjectionInput, ProjectionOutput,
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
        6,
        "la batería que audit_dump vuelca son los 6 casos P1–P6; si de verdad hace falta uno más \
         en el CSV, el oráculo externo tiene que enterarse"
    );
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

/// **DIANA, no regresión: hoy PANICA y por eso va `#[ignore]`.** Bug vivo encontrado montando
/// WP0 de 5.0.0 (pendiente de issue).
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
/// **Cuando el motor deje de panicar, quita el `#[ignore]`**: este test pasa a ser la regresión.
#[test]
#[ignore = "bug vivo del motor: gross_up_mixed_monthly desborda con una gain ratio denormal"]
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
}
