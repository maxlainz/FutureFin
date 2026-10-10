//! Interruptor por flujo de la pestaña «Simulación» (antes «Próximos», 5.0.0).
//!
//! Contrato: `planning_flows.enabled = false` ⇒ el flujo sigue existiendo y se lista, pero NO
//! entra en la proyección ni en ningún KPI — los `upcoming_*` del Resumen, el tramo `planning`
//! de la resolución de reparto y los `events` del chart. Y como `enabled` es un input del motor,
//! cambiarlo invalida la cache de proyección igual que cualquier otra edición del flujo.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_engine::add_months_signed;
use serde_json::{json, Value};

fn dec(v: &Value) -> f64 {
    v.as_str().unwrap_or_else(|| panic!("decimal string: {v}")).parse().unwrap()
}

async fn get(app: &TestApp, cookie: &str, uri: &str) -> Value {
    let r = app.get_with_cookie(uri, cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "GET {uri}: {r:?}");
    r.json()
}

async fn create_flow(app: &TestApp, owner: &LoggedInOwner, body: Value) -> String {
    let r = app.post_json_with_cookie("/v1/planning/flows", body, &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create flow: {r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

async fn set_enabled(app: &TestApp, owner: &LoggedInOwner, id: &str, enabled: bool) -> Value {
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/planning/flows/{id}"),
            json!({ "enabled": enabled }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "PATCH enabled={enabled}: {r:?}");
    r.json()
}

fn net_worth_series(proj: &Value) -> Vec<f64> {
    proj["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|p| p["net_worth"].as_f64().expect("net_worth"))
        .collect()
}

/// PREDICCIÓN: con los tres flujos apagados, la serie de patrimonio es BIT A BIT la de antes de
/// crearlos, los `upcoming_*` valen 0 (count incluido), el tramo `planning` es 0 sin aviso de
/// transitorio y no hay eventos; el listado sigue devolviendo los tres, con `enabled: false`.
/// Al volver a encenderlos, todo vuelve a moverse.
#[tokio::test]
async fn a_disabled_flow_reaches_no_reader_but_stays_listed() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    for (cat, amount) in [(&cat_inc, "3000"), (&cat_exp, "1000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({ "category_id": cat, "amount": amount }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "budget: {r:?}");
    }
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat_ast, "name": "Fondo", "current_value": "10000", "is_liquid": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset: {r:?}");

    let baseline = get(&app, &owner.cookie, "/v1/projection/series?density=hybrid").await;
    let baseline_nw = net_worth_series(&baseline);

    let anchor = chrono::Utc::now().date_naive();
    let reforma = add_months_signed(anchor, 5).format("%Y-%m-%d").to_string();
    let alquiler_start = add_months_signed(anchor, 2).format("%Y-%m-%d").to_string();
    let ids = [
        // Sin fecha → tramo `planning` de la resolución (90 días desde el día 1).
        create_flow(&app, &owner, json!({ "category_id": cat_inc, "title": "Devolucion",
                                          "expected_amount": "900" })).await,
        // Con fecha → evento del chart.
        create_flow(&app, &owner, json!({ "category_id": cat_exp, "title": "Reforma",
                                          "expected_amount": "5000", "due_date": reforma })).await,
        // Recurrente → `upcoming_recurring_*`.
        create_flow(&app, &owner, json!({ "category_id": cat_exp, "title": "Alquiler",
                                          "expected_amount": "400", "amount_basis": "per_month",
                                          "window_start_date": alquiler_start })).await,
    ];

    // --- Encendidos (default del alta): todo se mueve ----------------------------------------
    let fh = get(&app, &owner.cookie, "/v1/summary").await["financial_health"].clone();
    assert_eq!(fh["upcoming_flows_count"], 3, "{fh}");
    assert_eq!(fh["upcoming_recurring_count"], 1, "{fh}");
    assert_eq!(dec(&fh["upcoming_inflows_total"]), 900.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_outflows_total"]), 5000.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_recurring_monthly_outflow"]), 400.0, "{fh}");
    let res = get(&app, &owner.cookie, "/v1/allocation-rules/resolution").await;
    assert!(dec(&res["planning_component"]) > 0.0, "{res}");
    let proj = get(&app, &owner.cookie, "/v1/projection/series?density=hybrid").await;
    assert_eq!(proj["events"].as_array().unwrap().len(), 1, "{}", proj["events"]);
    assert_ne!(net_worth_series(&proj), baseline_nw, "encendidos, los flujos mueven la serie");

    // --- Apagados: ningún lector los ve ------------------------------------------------------
    for id in &ids {
        let flow = set_enabled(&app, &owner, id, false).await;
        assert_eq!(flow["enabled"], false, "{flow}");
    }
    let fh = get(&app, &owner.cookie, "/v1/summary").await["financial_health"].clone();
    assert_eq!(fh["upcoming_flows_count"], 0, "un apagado no cuenta: {fh}");
    assert_eq!(fh["upcoming_recurring_count"], 0, "{fh}");
    assert_eq!(dec(&fh["upcoming_inflows_total"]), 0.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_outflows_total"]), 0.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_recurring_monthly_inflow"]), 0.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_recurring_monthly_outflow"]), 0.0, "{fh}");
    assert!(fh.get("upcoming_last_due_date_ymd").is_none() || fh["upcoming_last_due_date_ymd"].is_null(), "{fh}");

    let res = get(&app, &owner.cookie, "/v1/allocation-rules/resolution").await;
    assert_eq!(dec(&res["planning_component"]), 0.0, "{res}");
    assert_eq!(res["base_includes_transient"], false, "{res}");

    let proj = get(&app, &owner.cookie, "/v1/projection/series?density=hybrid").await;
    assert!(proj["events"].as_array().unwrap().is_empty(), "{}", proj["events"]);
    assert_eq!(
        net_worth_series(&proj),
        baseline_nw,
        "apagados, la serie es exactamente la de antes de crearlos"
    );

    let listed = get(&app, &owner.cookie, "/v1/planning/flows").await;
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 3, "apagado ≠ borrado: sigue listado");
    assert!(listed.iter().all(|f| f["enabled"] == false), "{listed:?}");

    // --- Re-encendido: vuelve a contar --------------------------------------------------------
    set_enabled(&app, &owner, &ids[1], true).await;
    let fh = get(&app, &owner.cookie, "/v1/summary").await["financial_health"].clone();
    assert_eq!(fh["upcoming_flows_count"], 1, "{fh}");
    assert_eq!(dec(&fh["upcoming_outflows_total"]), 5000.0, "{fh}");
    let proj = get(&app, &owner.cookie, "/v1/projection/series?density=hybrid").await;
    assert_eq!(proj["events"].as_array().unwrap().len(), 1, "{}", proj["events"]);
}

/// `enabled` es un input del motor: el PATCH que solo lo cambia invalida la cache (FULL), no es
/// `patch_empty`, y el alta acepta nacer apagada.
#[tokio::test]
async fn toggling_enabled_invalidates_the_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    let cat = app.create_category(&owner, "expense", "Vida").await;

    let id = create_flow(&app, &owner, json!({ "category_id": cat, "title": "Coche",
                                               "expected_amount": "9000", "enabled": false })).await;
    let listed = get(&app, &owner.cookie, "/v1/planning/flows").await;
    assert_eq!(listed[0]["enabled"], false, "el alta respeta enabled=false: {listed}");

    app.warm_default_view(&owner.cookie, &key).await;
    set_enabled(&app, &owner, &id, true).await;
    app.assert_invalidated(&key, "PATCH enabled=true").await;

    app.warm_default_view(&owner.cookie, &key).await;
    set_enabled(&app, &owner, &id, false).await;
    app.assert_invalidated(&key, "PATCH enabled=false").await;

    // Un body vacío sigue siendo `patch_empty`; con solo `enabled` no lo es (probado arriba).
    let r = app
        .patch_json_with_cookie(&format!("/v1/planning/flows/{id}"), json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert!(r.json()["message"].as_str().unwrap().starts_with("patch_empty"), "{r:?}");
}

/// D21 sigue mandando: un viewer ve el estado pero no lo cambia, y otro miembro tampoco puede
/// apagar un flujo que no es suyo.
#[tokio::test]
async fn only_the_owner_of_the_row_can_toggle_it() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vera", "viewer").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let cat = app.create_category(&owner, "expense", "Vida").await;
    let id = create_flow(&app, &owner, json!({ "category_id": cat, "title": "Coche",
                                               "expected_amount": "9000" })).await;

    for (who, expected) in [(&viewer, http::StatusCode::FORBIDDEN), (&member, http::StatusCode::FORBIDDEN)] {
        let r = app
            .patch_json_with_cookie(
                &format!("/v1/planning/flows/{id}"),
                json!({ "enabled": false }),
                &who.cookie,
            )
            .await;
        assert_eq!(r.status, expected, "{r:?}");
    }
    let listed = get(&app, &viewer.cookie, "/v1/planning/flows?view=household").await;
    assert_eq!(listed[0]["enabled"], true, "el viewer VE el estado: {listed}");
}
