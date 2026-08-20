//! `GET /v1/allocation-rules/resolution` (3.8.0): la cascada RESUELTA del mes en curso.
//!
//! Cierra el hueco de observabilidad del issue #4: sin este endpoint era imposible, desde el chat,
//! distinguir «la cascada reparte de más» (falso) de «la base incluye un tramo transitorio de
//! planning que cambia cada día» (lo que realmente pasaba).

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn budget(app: &TestApp, cookie: &str, cat: &str, amount: &str) {
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({ "category_id": cat, "amount": amount }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "budget: {r:?}");
}

async fn asset(app: &TestApp, cookie: &str, cat: &str, name: &str, value: &str) -> String {
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": name, "current_value": value, "is_liquid": true }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset {name}: {r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

async fn rule(app: &TestApp, cookie: &str, body: Value) -> String {
    let r = app
        .post_json_with_cookie("/v1/allocation-rules", body.clone(), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "rule {body}: {r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

fn dec(v: &Value) -> f64 {
    v.as_str().unwrap().parse().unwrap()
}

/// PREDICCIÓN: ingreso 3000, gasto 1000 → recurrente 2000, sin planning. Reglas: 150 fijo a un
/// activo ya en su techo (cap 700 con valor 704 → `cap_full`), 40 % al indexado (800) y sumidero
/// (1200). Identidades: `base_cash = recurring_net + planning_component` y
/// `Σ per_asset + leftover = base_cash`.
#[tokio::test]
async fn resolution_explains_the_cascade_and_holds_the_identities() {
    let app = TestApp::spawn().await;
    let owner: LoggedInOwner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "3000").await;
    budget(&app, &owner.cookie, &cat_exp, "1000").await;

    let ahorro = asset(&app, &owner.cookie, &cat_ast, "Cartera Ahorro", "704").await;
    let indexado = asset(&app, &owner.cookie, &cat_ast, "Indexado", "50000").await;
    let sumidero = asset(&app, &owner.cookie, &cat_ast, "Sumidero", "0").await;

    let r_full = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": ahorro, "kind": "fixed", "amount": "150",
                "cap_kind": "amount", "cap_value": "700" }),
    )
    .await;
    let r_pct = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": indexado, "kind": "percent", "amount": "40" }),
    )
    .await;
    let r_rem = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": sumidero, "kind": "remainder" }),
    )
    .await;

    let resp = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let b = resp.json();

    assert_eq!(dec(&b["recurring_net"]), 2000.0, "{b}");
    assert_eq!(dec(&b["planning_component"]), 0.0, "{b}");
    assert_eq!(dec(&b["base_cash"]), 2000.0, "{b}");
    assert_eq!(b["base_includes_transient"], false, "{b}");
    assert_eq!(
        dec(&b["base_cash"]),
        dec(&b["recurring_net"]) + dec(&b["planning_component"]),
        "identidad base = recurrente + planning: {b}"
    );

    let rules = b["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 3, "una entrada por regla, incluidas las saltadas: {b}");

    let by_id = |id: &str| -> Value {
        rules
            .iter()
            .find(|r| r["rule_id"] == json!(id))
            .unwrap_or_else(|| panic!("regla {id} ausente: {b}"))
            .clone()
    };

    // La regla del activo ya en su techo se salta, y lo dice.
    let full = by_id(&r_full);
    assert_eq!(full["skipped_reason"], "cap_full", "{full}");
    assert_eq!(dec(&full["amount_resolved"]), 0.0, "{full}");
    assert_eq!(dec(&full["cap_ceiling"]), 700.0, "{full}");
    assert_eq!(dec(&full["cap_room"]), 0.0, "{full}");
    assert_eq!(full["target_asset_name"], "Cartera Ahorro", "{full}");

    // 40 % de 2000 = 800.
    let pct = by_id(&r_pct);
    assert!(pct["skipped_reason"].is_null(), "{pct}");
    assert_eq!(dec(&pct["amount_intent"]), 800.0, "{pct}");
    assert_eq!(dec(&pct["amount_resolved"]), 800.0, "{pct}");

    // El sumidero se lleva lo que queda: 1200.
    let rem = by_id(&r_rem);
    assert_eq!(dec(&rem["amount_resolved"]), 1200.0, "{rem}");

    // Identidad de reparto y sobrante.
    let per_asset_total: f64 = b["per_asset"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| dec(&a["amount"]))
        .sum();
    assert_eq!(per_asset_total, dec(&b["allocated_total"]), "{b}");
    assert_eq!(
        per_asset_total + dec(&b["leftover_to_surplus_cash"]),
        dec(&b["base_cash"]),
        "Σ per_asset + leftover debe cuadrar con base_cash: {b}"
    );
    assert_eq!(dec(&b["leftover_to_surplus_cash"]), 0.0, "{b}");
}

/// El caso del issue: con un planning flow SIN fecha, `base_cash` deja de ser el neto recurrente y
/// el endpoint lo **declara** (`base_includes_transient`) en vez de dejar al lector concluir que la
/// cascada reparte de más.
#[tokio::test]
async fn resolution_flags_the_transient_planning_tranche() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "3000").await;
    budget(&app, &owner.cookie, &cat_exp, "1000").await;
    let sumidero = asset(&app, &owner.cookie, &cat_ast, "Sumidero", "0").await;
    rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": sumidero, "kind": "remainder" }),
    )
    .await;

    let plan = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            json!({ "category_id": cat_inc, "title": "Devolucion renta",
                    "expected_amount": "900" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(plan.status, http::StatusCode::CREATED, "{plan:?}");

    let b = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await
        .json();

    assert_eq!(dec(&b["recurring_net"]), 2000.0, "la parte estable no se mueve: {b}");
    assert!(dec(&b["planning_component"]) > 0.0, "{b}");
    assert_eq!(b["base_includes_transient"], true, "{b}");
    assert_eq!(
        dec(&b["base_cash"]),
        dec(&b["recurring_net"]) + dec(&b["planning_component"]),
        "{b}"
    );
    // El tramo es múltiplo exacto de 900/90 = 10 €/día.
    let tranche = dec(&b["planning_component"]);
    assert!((tranche / 10.0).fract().abs() < 1e-9, "tramo {tranche}: {b}");

    // Y cuadra con lo que `/v1/assets` publica como aportación del mes 1.
    let assets = app.get_with_cookie("/v1/assets", &owner.cookie).await.json();
    let nominal = dec(&assets[0]["contribution_nominal_monthly"]);
    let recurring = dec(&assets[0]["contribution_recurring_monthly"]);
    assert_eq!(nominal, dec(&b["base_cash"]), "{assets} vs {b}");
    assert_eq!(nominal - recurring, tranche, "{assets} vs {b}");
}

/// Las reglas posteriores al corte por caja se emiten con `not_reached`, no desaparecen: si no, un
/// lector concluiría que la regla se borró.
#[tokio::test]
async fn resolution_reports_rules_the_cascade_never_reached() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "1200").await;
    budget(&app, &owner.cookie, &cat_exp, "1000").await;

    let a = asset(&app, &owner.cookie, &cat_ast, "Primero", "0").await;
    let b_id = asset(&app, &owner.cookie, &cat_ast, "Segundo", "0").await;
    // El fijo se lleva los 200 enteros; la segunda regla nunca llega a evaluarse.
    rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": a, "kind": "fixed", "amount": "500" }),
    )
    .await;
    let r_second = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": b_id, "kind": "remainder" }),
    )
    .await;

    let b = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await
        .json();
    assert_eq!(dec(&b["base_cash"]), 200.0, "{b}");
    let second = b["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["rule_id"] == json!(r_second))
        .unwrap();
    assert_eq!(
        second["skipped_reason"], "not_reached",
        "«no llegué a evaluarla» y «no sobra dinero» son diagnósticos distintos: {b}"
    );
    // Y la primera fue RECORTADA (pidió 500, se llevó 200), que no es lo mismo que saltada.
    let first = b["rules"].as_array().unwrap()[0].clone();
    assert!(first["skipped_reason"].is_null(), "{first}");
    assert_eq!(dec(&first["amount_intent"]), 500.0, "{first}");
    assert_eq!(dec(&first["amount_resolved"]), 200.0, "{first}");
}
