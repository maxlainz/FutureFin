//! `GET /v1/allocation-rules/goals` — el **cuándo** de los topes de la cascada.
//!
//! No hay tabla `goals`: el tope YA es el objetivo (`months_expense` con valor 6 es literalmente
//! «fondo de emergencia de 6 meses»; `amount` es un objetivo en euros). Lo único que faltaba era
//! la fecha, y sale de cruzar la serie por activo del MISMO motor que sirve `/v1/projection/series`
//! con el techo del tope.
//!
//! Lo que estos tests fijan, en orden de importancia:
//!
//! 1. El techo contra el que se calcula el ETA es **el mismo número** que la cascada resuelta
//!    publica como `cap_ceiling` (una sola implementación en el lado API).
//! 2. El ETA es el primer mes en que el activo alcanza ese techo, con la aritmética hecha a mano
//!    en el propio test (predict-then-run, no «lo que salga»).
//! 3. Las tres ausencias se declaran con motivo y no se confunden entre sí.

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
            json!({
                "category_id": cat, "name": name, "current_value": value,
                "is_liquid": true, "expected_annual_return_percent": "0"
            }),
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

fn goal_of<'a>(body: &'a Value, rule_id: &str) -> &'a Value {
    body["goals"]
        .as_array()
        .expect("goals array")
        .iter()
        .find(|g| g["rule_id"] == rule_id)
        .unwrap_or_else(|| panic!("no goal for rule {rule_id} in {body}"))
}

/// PREDICCIÓN, hecha antes de correr nada: ingreso 3.000, gasto 1.000 → sobrante 2.000 €/mes.
/// El fondo de emergencia parte de 0 y recibe 500 €/mes con tope `amount = 1500`; con
/// rentabilidad 0 %, los saldos de cierre son 500, 1.000 y 1.500 → **cruza en el mes 3**
/// (`per_asset_series[3] = 1500 >= 1500`), y el progreso de hoy es 0/1500 = 0 %.
///
/// El colchón de `months_expense = 2` sobre un gasto de 1.000 y sin deuda tiene techo 2.000 y
/// arranca ya en 2.500 → `already_reached`, sin fecha.
#[tokio::test]
async fn goals_cross_the_projection_series_at_the_predicted_month() {
    let app = TestApp::spawn().await;
    let owner: LoggedInOwner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "3000").await;
    budget(&app, &owner.cookie, &cat_exp, "1000").await;

    let emergencia = asset(&app, &owner.cookie, &cat_ast, "Emergencia", "0").await;
    let colchon = asset(&app, &owner.cookie, &cat_ast, "Colchón", "2500").await;
    let sumidero = asset(&app, &owner.cookie, &cat_ast, "Indexado", "0").await;

    let r_emergencia = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": emergencia, "kind": "fixed", "amount": "500",
                "cap_kind": "amount", "cap_value": "1500" }),
    )
    .await;
    let r_colchon = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": colchon, "kind": "fixed", "amount": "100",
                "cap_kind": "months_expense", "cap_value": "2" }),
    )
    .await;
    // #150: "Emergencia" fue el primer activo del owner → ya sembró el sumidero (apuntando a
    // ella). Lo retargeteamos al activo pensado como sumidero en vez de crear uno segundo.
    let r_sumidero = app.sink_rule_id(&owner.cookie).await;
    let retarget = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{r_sumidero}"),
            json!({ "target_asset_id": sumidero }),
            &owner.cookie,
        )
        .await;
    assert_eq!(retarget.status, http::StatusCode::OK, "{retarget:?}");

    let resp = app
        .get_with_cookie("/v1/allocation-rules/goals", &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let b = resp.json();

    assert_eq!(b["view"], "household", "{b}");
    assert_eq!(b["covers_deletions"], Value::Null, "campo ajeno a este endpoint");
    assert_eq!(
        b["rules_without_cap"], 1,
        "el sumidero no es un objetivo y se cuenta aparte: {b}"
    );
    assert_eq!(b["goals"].as_array().unwrap().len(), 2, "{b}");
    assert!(
        !b["goals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["rule_id"] == r_sumidero.as_str()),
        "una regla sin tope no puede aparecer como objetivo: {b}"
    );

    let g1 = goal_of(&b, &r_emergencia);
    assert_eq!(g1["cap_kind"], "amount");
    assert_eq!(g1["ceiling_basis"], "fixed_amount");
    assert_eq!(dec(&g1["ceiling"]), 1500.0, "{g1}");
    assert_eq!(dec(&g1["current_value"]), 0.0, "{g1}");
    assert_eq!(dec(&g1["progress_pct"]), 0.0, "{g1}");
    assert_eq!(
        g1["eta_month_index"], 3,
        "500 €/mes sobre 0 con tope 1.500 y 0 % de rentabilidad cruza en el mes 3: {g1}"
    );
    assert!(g1["eta_month_ymd"].is_string(), "{g1}");
    assert_eq!(g1["eta_absent_reason"], Value::Null, "{g1}");

    let g2 = goal_of(&b, &r_colchon);
    assert_eq!(g2["cap_kind"], "months_expense");
    assert_eq!(
        g2["ceiling_basis"], "months_expense_today",
        "el techo relativo declara que se resuelve con los escalares de hoy: {g2}"
    );
    assert_eq!(dec(&g2["ceiling"]), 2000.0, "2 × gasto 1.000: {g2}");
    assert_eq!(g2["eta_absent_reason"], "already_reached", "{g2}");
    assert_eq!(g2["eta_month_index"], Value::Null, "{g2}");
    assert_eq!(g2["eta_month_ymd"], Value::Null, "{g2}");
}

/// El techo del objetivo y el `cap_ceiling` que el engine resuelve en la cascada del mes son el
/// MISMO número. Si algún día se separan, la app enseñaría un objetivo y el ETA hablaría de otro.
#[tokio::test]
async fn goal_ceilings_match_the_engine_resolution() {
    let app = TestApp::spawn().await;
    let owner: LoggedInOwner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "2400").await;
    budget(&app, &owner.cookie, &cat_exp, "900").await;

    let a_amount = asset(&app, &owner.cookie, &cat_ast, "Meta €", "100").await;
    let a_months = asset(&app, &owner.cookie, &cat_ast, "Meta meses", "100").await;
    let a_income = asset(&app, &owner.cookie, &cat_ast, "Meta ingresos", "100").await;
    let a_sink = asset(&app, &owner.cookie, &cat_ast, "Resto", "0").await;

    rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": a_amount, "kind": "fixed", "amount": "100",
                "cap_kind": "amount", "cap_value": "4321" }),
    )
    .await;
    rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": a_months, "kind": "fixed", "amount": "100",
                "cap_kind": "months_expense", "cap_value": "3.5" }),
    )
    .await;
    rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": a_income, "kind": "fixed", "amount": "100",
                "cap_kind": "income_multiple", "cap_value": "1.25" }),
    )
    .await;
    // #150: "Meta €" fue el primer activo del owner → ya sembró el sumidero; lo retargeteamos.
    let seeded = app.sink_rule_id(&owner.cookie).await;
    let retarget = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{seeded}"),
            json!({ "target_asset_id": a_sink }),
            &owner.cookie,
        )
        .await;
    assert_eq!(retarget.status, http::StatusCode::OK, "{retarget:?}");

    let goals = app
        .get_with_cookie("/v1/allocation-rules/goals", &owner.cookie)
        .await
        .json();
    let resolution = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await
        .json();

    let mut compared = 0;
    for g in goals["goals"].as_array().unwrap() {
        let rid = g["rule_id"].as_str().unwrap();
        let r = resolution["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["rule_id"] == rid)
            .unwrap_or_else(|| panic!("la resolución no trae la regla {rid}: {resolution}"));
        assert_eq!(
            dec(&g["ceiling"]),
            dec(&r["cap_ceiling"]),
            "techo del objetivo != cap_ceiling del engine para {rid}: {g} vs {r}"
        );
        compared += 1;
    }
    assert_eq!(compared, 3, "los tres tipos de tope se comparan: {goals}");
}

/// Un tope inalcanzable dentro del horizonte NO se publica como «nunca»: se publica como
/// `not_within_horizon`, con el horizonte al lado para que se pueda leer literalmente.
#[tokio::test]
async fn an_unreachable_ceiling_says_not_within_horizon() {
    let app = TestApp::spawn().await;
    let owner: LoggedInOwner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "1100").await;
    budget(&app, &owner.cookie, &cat_exp, "1000").await;

    let lejano = asset(&app, &owner.cookie, &cat_ast, "Casa", "0").await;
    let sink = asset(&app, &owner.cookie, &cat_ast, "Resto", "0").await;
    let r_lejano = rule(
        &app,
        &owner.cookie,
        json!({ "target_asset_id": lejano, "kind": "fixed", "amount": "10",
                "cap_kind": "amount", "cap_value": "100000000" }),
    )
    .await;
    // #150: "Casa" fue el primer activo del owner → ya sembró el sumidero; lo retargeteamos.
    let seeded = app.sink_rule_id(&owner.cookie).await;
    let retarget = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{seeded}"),
            json!({ "target_asset_id": sink }),
            &owner.cookie,
        )
        .await;
    assert_eq!(retarget.status, http::StatusCode::OK, "{retarget:?}");

    let b = app
        .get_with_cookie("/v1/allocation-rules/goals", &owner.cookie)
        .await
        .json();
    let g = goal_of(&b, &r_lejano);
    assert_eq!(g["eta_absent_reason"], "not_within_horizon", "{g}");
    assert_eq!(g["eta_month_index"], Value::Null, "{g}");
    assert!(
        b["horizon_months"].as_u64().unwrap() >= 12,
        "el horizonte viaja para poder leer «no dentro de N meses»: {b}"
    );
    assert!(b["horizon_basis"].is_string(), "{b}");
}
