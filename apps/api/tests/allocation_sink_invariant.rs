//! I1 — el sumidero de la cascada, ahora como POST-condición única (`commit_with_sink_invariant`).
//!
//! Cubre los dos agujeros que la dispersión anterior dejaba abiertos:
//!
//! 1. **`PATCH` convertía en sumidero sin recolocar.** Con el scope sin sumidero previo, la guardia
//!    de conteo pasaba y la regla se quedaba donde estaba: la cascada acababa con su sumidero en
//!    MEDIO, comiéndose el sobrante antes de que las reglas de debajo lo vieran, en silencio.
//! 2. **La guardia `sink_must_be_last` del `reorder` no miraba nada en `household`.** Resolvía el
//!    scope desde la VISTA, y `household` era `owner_user_id IS NULL` — que no casa ninguna fila
//!    creada por la API (el alta siempre escribe un owner).
//!
//! Y fija lo que ya funcionaba: un solo sumidero por scope, no se puede borrar el último, y la
//! colocación al crear (el sumidero baja un puesto para seguir siendo el último).

mod common;

use common::TestApp;
use uuid::Uuid;

async fn setup(app: &TestApp) -> (common::LoggedInOwner, String, String) {
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Inversión").await;
    let a1 = create_asset(app, &owner, &cat, "Fondo").await;
    let a2 = create_asset(app, &owner, &cat, "Colchón").await;
    (owner, a1, a2)
}

async fn create_asset(
    app: &TestApp,
    owner: &common::LoggedInOwner,
    category_id: &str,
    name: &str,
) -> String {
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": category_id, "name": name, "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create asset: {r:?}");
    r.json()["id"].as_str().expect("asset id").to_string()
}

async fn create_rule(
    app: &TestApp,
    owner: &common::LoggedInOwner,
    body: serde_json::Value,
) -> common::ResponseParts {
    app.post_json_with_cookie("/v1/allocation-rules", body, &owner.cookie)
        .await
}

async fn rules(app: &TestApp, owner: &common::LoggedInOwner) -> Vec<serde_json::Value> {
    let r = app.get_with_cookie("/v1/allocation-rules", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "list rules: {r:?}");
    r.json().as_array().expect("array").clone()
}

// ---------------------------------------------------------------------------
// El agujero del PATCH: convertirse en sumidero implica irse al final
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patching_a_rule_into_the_sink_moves_it_last() {
    let app = TestApp::spawn().await;
    let (owner, a1, a2) = setup(&app).await;

    // Dos reglas normales, sin ningún sumidero en el scope: es el estado en el que la guardia de
    // conteo pasaba (`n == 0`) y nadie recolocaba nada.
    let r1 = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a1, "kind": "fixed", "amount": "100"}),
    )
    .await;
    assert_eq!(r1.status, http::StatusCode::CREATED, "{r1:?}");
    let rule1 = r1.json()["id"].as_str().unwrap().to_string();

    let r2 = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a2, "kind": "fixed", "amount": "50"}),
    )
    .await;
    assert_eq!(r2.status, http::StatusCode::CREATED, "{r2:?}");
    let rule2 = r2.json()["id"].as_str().unwrap().to_string();

    // La PRIMERA (prioridad menor) pasa a ser el sumidero.
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{rule1}"),
            serde_json::json!({"kind": "remainder", "amount": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");

    let list = rules(&app, &owner).await;
    assert_eq!(list.len(), 2);
    let last = list.last().unwrap();
    assert_eq!(
        last["id"].as_str().unwrap(),
        rule1,
        "el sumidero debe quedar el ÚLTIMO de la cascada; la lista fue {list:?}"
    );
    assert_eq!(last["kind"], "remainder");
    // …y la otra regla no se ha movido de sitio relativo.
    assert_eq!(list[0]["id"].as_str().unwrap(), rule2);
}

// ---------------------------------------------------------------------------
// Un solo sumidero por scope, y no se puede quedar sin él
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_second_uncapped_remainder_is_rejected() {
    let app = TestApp::spawn().await;
    let (owner, a1, a2) = setup(&app).await;

    let first = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a1, "kind": "remainder"}),
    )
    .await;
    assert_eq!(first.status, http::StatusCode::CREATED, "{first:?}");

    let second = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a2, "kind": "remainder"}),
    )
    .await;
    assert_eq!(second.status, http::StatusCode::BAD_REQUEST, "{second:?}");
    assert_eq!(second.json()["code"], "uncapped_remainder_exists");

    // Un `remainder` CON tope no es el sumidero: eso sí se puede crear.
    let capped = create_rule(
        &app,
        &owner,
        serde_json::json!({
            "target_asset_id": a2, "kind": "remainder",
            "cap_kind": "amount", "cap_value": "5000"
        }),
    )
    .await;
    assert_eq!(capped.status, http::StatusCode::CREATED, "{capped:?}");
}

#[tokio::test]
async fn deleting_the_only_sink_is_rejected_and_the_rule_survives() {
    let app = TestApp::spawn().await;
    let (owner, a1, _a2) = setup(&app).await;

    let sink = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a1, "kind": "remainder"}),
    )
    .await;
    let sink_id = sink.json()["id"].as_str().unwrap().to_string();

    let del = app
        .delete_with_cookie(&format!("/v1/allocation-rules/{sink_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::BAD_REQUEST, "{del:?}");
    assert_eq!(del.json()["code"], "remainder_required");
    assert_eq!(rules(&app, &owner).await.len(), 1, "la regla sigue viva");
}

// ---------------------------------------------------------------------------
// Colocación al crear: el sumidero baja un puesto y sigue siendo el último
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_new_rule_is_inserted_before_the_sink() {
    let app = TestApp::spawn().await;
    let (owner, a1, a2) = setup(&app).await;

    let sink = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a1, "kind": "remainder"}),
    )
    .await;
    let sink_id = sink.json()["id"].as_str().unwrap().to_string();

    let normal = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a2, "kind": "fixed", "amount": "200"}),
    )
    .await;
    assert_eq!(normal.status, http::StatusCode::CREATED, "{normal:?}");

    let list = rules(&app, &owner).await;
    assert_eq!(list.len(), 2);
    assert_eq!(
        list.last().unwrap()["id"].as_str().unwrap(),
        sink_id,
        "el sumidero sigue el último: {list:?}"
    );
}

// ---------------------------------------------------------------------------
// El reorder en household: la guardia que no miraba nada
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reorder_in_household_view_still_refuses_to_unseat_the_sink() {
    let app = TestApp::spawn().await;
    let (owner, a1, a2) = setup(&app).await;

    let sink_id = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a1, "kind": "remainder"}),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let normal_id = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a2, "kind": "fixed", "amount": "200"}),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Orden ilegal: el sumidero delante. Sin `?view=mine`, o sea la vista household — la que hasta
    // 4.4.0 no comprobaba nada.
    let bad = app
        .post_json_with_cookie(
            "/v1/allocation-rules/reorder",
            serde_json::json!({"ids": [sink_id, normal_id]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST, "{bad:?}");
    assert_eq!(bad.json()["code"], "sink_must_be_last");

    // …y la cascada no se ha movido (la transacción se revirtió entera).
    let list = rules(&app, &owner).await;
    assert_eq!(list.last().unwrap()["id"].as_str().unwrap(), sink_id);

    // El orden legal sí pasa.
    let ok = app
        .post_json_with_cookie(
            "/v1/allocation-rules/reorder",
            serde_json::json!({"ids": [normal_id, sink_id]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::OK, "{ok:?}");
}

// ---------------------------------------------------------------------------
// Cache FULL: crear y borrar reglas invalida la proyección
// ---------------------------------------------------------------------------

#[tokio::test]
async fn creating_and_deleting_a_rule_invalidates_the_projection_cache() {
    let app = TestApp::spawn().await;
    let (owner, a1, a2) = setup(&app).await;
    let iid: Uuid = app.installation_id().await;
    let key = app.household_key(iid, owner.user_id);
    app.settle_login_warmup(iid).await;

    app.warm_household(&owner.cookie, &key).await;
    let created = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a1, "kind": "remainder"}),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    app.assert_invalidated(&key, "POST /v1/allocation-rules").await;

    let extra = create_rule(
        &app,
        &owner,
        serde_json::json!({"target_asset_id": a2, "kind": "fixed", "amount": "10"}),
    )
    .await;
    let extra_id = extra.json()["id"].as_str().unwrap().to_string();

    app.warm_household(&owner.cookie, &key).await;
    let del = app
        .delete_with_cookie(&format!("/v1/allocation-rules/{extra_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT, "{del:?}");
    app.assert_invalidated(&key, "DELETE /v1/allocation-rules/{id}")
        .await;
}
