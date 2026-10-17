//! 6.0.0 F2-1 — CRUD de «Planes»: planes, bloques y cambios anidados, la forma de cada tipo de
//! cambio, «como mucho un plan activo por persona» (también con activaciones simultáneas), los
//! borrados en cascada y la subida de `revision`.
//!
//! Y la promesa de la fase: **ninguna cifra se mueve**. La proyección todavía no lee los planes
//! (F2-4), así que crear y activar un plan lleno de cambios deja la serie idéntica — pero toda
//! escritura invalida ya la cache, como hará falta cuando sí la lea.
//!
//! Propiedad y vistas: `plans_ownership.rs`.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn create_plan(app: &TestApp, who: &LoggedInOwner, name: &str) -> Value {
    let r = app
        .post_json_with_cookie("/v1/plans", json!({ "name": name }), &who.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    r.json()
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().expect("id").to_string()
}

async fn get_plan(app: &TestApp, who: &LoggedInOwner, plan_id: &str) -> Value {
    let r = app
        .get_with_cookie(&format!("/v1/plans/{plan_id}"), &who.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    r.json()
}

async fn add_change(app: &TestApp, who: &LoggedInOwner, plan_id: &str, body: Value) -> Value {
    let r = app
        .post_json_with_cookie(&format!("/v1/plans/{plan_id}/changes"), body, &who.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    r.json()
}

async fn expect_code(r: &common::ResponseParts, status: http::StatusCode, code: &str) {
    assert_eq!(r.status, status, "{r:?}");
    assert_eq!(r.json()["code"], code, "{r:?}");
}

#[tokio::test]
async fn create_list_get_patch_delete_round_trip() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;

    let plan = create_plan(&app, &owner, "  Comprar casa  ").await;
    let plan_id = id_of(&plan);
    assert_eq!(plan["name"], "Comprar casa");
    assert_eq!(plan["is_active"], false, "un plan nace inactivo");
    assert_eq!(plan["revision"], 1);
    assert_eq!(plan["owner_user_id"], owner.user_id.to_string());
    assert_eq!(
        plan["assumptions"],
        json!({
            "inflation_override_pct": null,
            "income_follows_inflation": true,
            "expense_follows_inflation": true,
            "horizon_age": 90,
            "market_return_adjustment_pp": "0"
        }),
        "omitir los supuestos = todos por defecto, publicados enteros"
    );
    let blocks = plan["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 1, "nace con un bloque");
    assert_eq!(blocks[0]["name"], "General");
    assert_eq!(blocks[0]["changes"], json!([]));

    let list = app.get_with_cookie("/v1/plans", &owner.cookie).await;
    assert_eq!(list.status, http::StatusCode::OK);
    assert_eq!(
        list.json()["view"],
        "mine",
        "la lista ecoa la vista (default mine)"
    );
    assert_eq!(list.json()["plans"].as_array().unwrap().len(), 1);
    assert_eq!(get_plan(&app, &owner, &plan_id).await["id"], plan["id"]);

    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}"),
            json!({
                "name": "Casa en 2029",
                "assumptions": { "inflation_override_pct": "3", "horizon_age": 95 },
                "sort_index": 4
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    let p = patched.json();
    assert_eq!(p["name"], "Casa en 2029");
    assert_eq!(p["revision"], 2);
    assert_eq!(p["sort_index"], 4);
    assert_eq!(p["assumptions"]["inflation_override_pct"], "3");
    assert_eq!(p["assumptions"]["horizon_age"], 95);
    assert_eq!(
        p["assumptions"]["income_follows_inflation"], true,
        "lo no mandado vuelve a su default"
    );

    // Los supuestos REEMPLAZAN: mandar `{}` devuelve el override a null.
    let reset = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}"),
            json!({ "assumptions": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        reset.json()["assumptions"]["inflation_override_pct"],
        Value::Null
    );
    assert_eq!(reset.json()["revision"], 3);

    let empty = app
        .patch_json_with_cookie(&format!("/v1/plans/{plan_id}"), json!({}), &owner.cookie)
        .await;
    expect_code(&empty, http::StatusCode::BAD_REQUEST, "patch_empty").await;

    let del = app
        .delete_with_cookie(&format!("/v1/plans/{plan_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT, "{del:?}");
    let gone = app
        .get_with_cookie(&format!("/v1/plans/{plan_id}"), &owner.cookie)
        .await;
    expect_code(&gone, http::StatusCode::NOT_FOUND, "plan_not_found").await;
    assert_eq!(app.count_rows("plans").await, 0);
    assert_eq!(
        app.count_rows("plan_blocks").await,
        0,
        "el bloque cae con el plan"
    );
}

#[tokio::test]
async fn assumptions_are_strict_on_write() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;

    for (body, why) in [
        (json!({ "inflation_pct": "3" }), "clave desconocida"),
        (json!({ "horizon_age": 84 }), "horizonte por debajo de 85"),
        (json!({ "horizon_age": 106 }), "horizonte por encima de 105"),
        (
            json!({ "inflation_override_pct": "50.1" }),
            "inflación > 50",
        ),
        (
            json!({ "inflation_override_pct": "-2.5" }),
            "inflación < -2",
        ),
        (
            json!({ "market_return_adjustment_pp": "-6" }),
            "ajuste < -5",
        ),
        (
            json!({ "market_return_adjustment_pp": 1 }),
            "número en vez de string",
        ),
        (json!([1, 2]), "no es un objeto"),
    ] {
        let r = app
            .post_json_with_cookie(
                "/v1/plans",
                json!({ "name": "X", "assumptions": body }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{why}: {r:?}");
        assert_eq!(r.json()["code"], "plan_assumptions_invalid", "{why}");
    }
    assert_eq!(app.count_rows("plans").await, 0, "ningún alta a medias");

    let ok = app
        .post_json_with_cookie(
            "/v1/plans",
            json!({ "name": "X", "assumptions": {
                "inflation_override_pct": "-2", "income_follows_inflation": false,
                "expense_follows_inflation": true, "horizon_age": 105,
                "market_return_adjustment_pp": "-1.5"
            }}),
            &owner.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::CREATED, "{ok:?}");
    assert_eq!(ok.json()["assumptions"]["income_follows_inflation"], false);
    assert_eq!(
        ok.json()["assumptions"]["market_return_adjustment_pp"],
        "-1.5"
    );

    let blank = app
        .post_json_with_cookie("/v1/plans", json!({ "name": "   " }), &owner.cookie)
        .await;
    expect_code(&blank, http::StatusCode::BAD_REQUEST, "name_empty").await;
}

#[tokio::test]
async fn blocks_and_changes_nest_and_every_write_bumps_the_revision() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let plan = create_plan(&app, &owner, "Vida").await;
    let plan_id = id_of(&plan);
    let general = plan["blocks"][0]["id"].as_str().unwrap().to_string();
    let mut revision = 1;

    let casa = app
        .post_json_with_cookie(
            &format!("/v1/plans/{plan_id}/blocks"),
            json!({ "name": "Casa", "sort_index": 1 }),
            &owner.cookie,
        )
        .await;
    assert_eq!(casa.status, http::StatusCode::CREATED, "{casa:?}");
    let casa_id = id_of(&casa.json());
    revision += 1;

    // Sin `block_id`: va al primer bloque (General). Ingreso mensual por defecto, en € de hoy.
    let raise = add_change(
        &app,
        &owner,
        &plan_id,
        json!({ "kind": "income", "start_date": "2031-01-01", "amount": "200" }),
    )
    .await;
    revision += 1;
    assert_eq!(raise["block_id"], general.as_str());
    assert_eq!(raise["cadence"], "monthly");
    assert_eq!(raise["amount_unit"], "eur");
    assert_eq!(
        raise["amount"], "200.0000",
        "Decimal como string, a la escala de la columna"
    );
    assert_eq!(raise["keeps_purchasing_power"], true);
    assert_eq!(raise["end_date"], Value::Null, "null = sin fin, y viaja");
    assert_eq!(raise["params"], json!({}));

    let house = add_change(
        &app,
        &owner,
        &plan_id,
        json!({
            "block_id": casa_id, "kind": "asset_in", "start_date": "2029-03-01",
            "amount": "250000",
            "params": { "name": "Piso", "expected_annual_return_percent": "2", "is_liquid": false }
        }),
    )
    .await;
    revision += 1;
    assert_eq!(
        house["cadence"], "one_off",
        "asset_in es puntual por defecto"
    );
    assert_eq!(house["params"]["name"], "Piso");
    assert_eq!(house["params"]["is_liquid"], false);
    assert_eq!(house["params"]["annual_volatility_percent"], Value::Null);

    let mortgage = add_change(
        &app,
        &owner,
        &plan_id,
        json!({
            "block_id": casa_id, "kind": "loan", "start_date": "2029-03-01",
            "amount": "200000",
            "params": { "apr_percent": "3", "term_months": 300 }
        }),
    )
    .await;
    revision += 1;
    assert_eq!(
        mortgage["keeps_purchasing_power"], false,
        "un préstamo es nominal"
    );
    assert_eq!(mortgage["params"]["repayment_model"], "french");

    let sale = add_change(
        &app,
        &owner,
        &plan_id,
        json!({
            "kind": "asset_sale", "start_date": "2045-01-01",
            "target_change_id": house["id"]
        }),
    )
    .await;
    revision += 1;
    assert_eq!(sale["amount"], Value::Null, "sin importe = vender entero");
    assert_eq!(sale["target_change_id"], house["id"]);

    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&raise)),
            json!({ "end_date": "2040-12-01", "amount": "-150", "block_id": casa_id }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    revision += 1;
    assert_eq!(patched.json()["amount"], "-150.0000", "con signo: baja");
    assert_eq!(patched.json()["end_date"], "2040-12-01");
    assert_eq!(patched.json()["block_id"], casa_id.as_str());

    let cleared = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&raise)),
            json!({ "end_date": null }),
            &owner.cookie,
        )
        .await;
    assert_eq!(cleared.json()["end_date"], Value::Null, "null borra");
    revision += 1;

    let renamed = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}/blocks/{general}"),
            json!({ "enabled": false }),
            &owner.cookie,
        )
        .await;
    assert_eq!(renamed.status, http::StatusCode::OK, "{renamed:?}");
    assert_eq!(renamed.json()["enabled"], false);
    revision += 1;

    let full = get_plan(&app, &owner, &plan_id).await;
    assert_eq!(
        full["revision"], revision,
        "cada escritura sube la revisión"
    );
    let blocks = full["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["name"], "General");
    assert_eq!(
        blocks[0]["changes"].as_array().unwrap().len(),
        1,
        "la venta"
    );
    let casa_changes: Vec<&str> = blocks[1]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    // Cronológico: 2029-03 (entrada y préstamo, por alta) y 2031-01 (el ingreso movido aquí).
    assert_eq!(casa_changes, vec!["asset_in", "loan", "income"]);

    let del = app
        .delete_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&mortgage)),
            &owner.cookie,
        )
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    assert_eq!(
        get_plan(&app, &owner, &plan_id).await["revision"],
        revision + 1
    );
    let again = app
        .delete_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&mortgage)),
            &owner.cookie,
        )
        .await;
    expect_code(&again, http::StatusCode::NOT_FOUND, "plan_change_not_found").await;
}

#[tokio::test]
async fn every_kind_has_its_shape() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let plan_id = id_of(&create_plan(&app, &owner, "Formas").await);
    let income = add_change(
        &app,
        &owner,
        &plan_id,
        json!({ "kind": "income", "start_date": "2030-01-01", "amount": "10", "amount_unit": "pct" }),
    )
    .await;
    assert_eq!(income["amount_unit"], "pct");

    let cases = [
        (
            json!({ "kind": "bonus", "start_date": "2030-01-01", "amount": "1" }),
            "tipo desconocido",
        ),
        (
            json!({ "kind": "income", "start_date": "2030-01-01" }),
            "ingreso sin importe",
        ),
        (
            json!({ "kind": "expense", "start_date": "2030-01-01", "amount": "0" }),
            "importe cero",
        ),
        (
            json!({ "kind": "expense", "start_date": "2030-01-01", "amount": "5",
                    "amount_unit": "pct", "cadence": "one_off" }),
            "pct solo mensual",
        ),
        (
            json!({ "kind": "expense", "start_date": "2030-01-01", "amount": "5",
                    "end_date": "2029-01-01" }),
            "fin antes de inicio",
        ),
        (
            json!({ "kind": "expense", "start_date": "2030-01-01", "amount": "5",
                    "cadence": "one_off", "end_date": "2031-01-01" }),
            "un puntual no tiene fin",
        ),
        (
            json!({ "kind": "income", "start_date": "2030-01-01", "amount": "5",
                    "params": { "x": 1 } }),
            "params desconocidos",
        ),
        (
            json!({ "kind": "income", "start_date": "2030-01-01", "amount": "5",
                    "target_change_id": income["id"] }),
            "sujeto en un ingreso",
        ),
        (
            json!({ "kind": "asset_in", "start_date": "2030-01-01", "amount": "5" }),
            "asset_in sin params",
        ),
        (
            json!({ "kind": "asset_in", "start_date": "2030-01-01", "amount": "5",
                    "params": { "name": "X", "annual_volatility_percent": "101" } }),
            "volatilidad fuera de rango",
        ),
        (
            json!({ "kind": "asset_sale", "start_date": "2030-01-01" }),
            "venta sin sujeto",
        ),
        (
            json!({ "kind": "asset_sale", "start_date": "2030-01-01",
                    "target_change_id": income["id"] }),
            "vender algo que no es un asset_in",
        ),
        (
            json!({ "kind": "asset_sale", "start_date": "2030-01-01", "cadence": "monthly",
                    "asset_id": income["id"] }),
            "vender entero cada mes",
        ),
        (
            json!({ "kind": "loan", "start_date": "2030-01-01", "amount": "1000",
                    "keeps_purchasing_power": true,
                    "params": { "apr_percent": "3", "term_months": 120 } }),
            "préstamo indexado",
        ),
        (
            json!({ "kind": "loan", "start_date": "2030-01-01", "amount": "1000",
                    "params": { "apr_percent": "3", "term_months": 601 } }),
            "plazo > 600",
        ),
        (
            json!({ "kind": "income", "start_date": "2200-01-01", "amount": "5" }),
            "más de 100 años",
        ),
    ];
    for (body, why) in cases {
        let r = app
            .post_json_with_cookie(&format!("/v1/plans/{plan_id}/changes"), body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{why}: {r:?}");
        assert_eq!(
            r.json()["code"],
            "plan_change_invalid_shape",
            "{why}: {r:?}"
        );
    }

    // Un asset_id inexistente no es «forma»: es un activo fuera de tu alcance.
    let ghost = app
        .post_json_with_cookie(
            &format!("/v1/plans/{plan_id}/changes"),
            json!({ "kind": "asset_sale", "start_date": "2030-01-01",
                    "asset_id": "00000000-0000-0000-0000-000000000001" }),
            &owner.cookie,
        )
        .await;
    expect_code(&ghost, http::StatusCode::BAD_REQUEST, "asset_not_in_scope").await;

    // El PATCH valida el estado RESULTANTE, y el tipo es inmutable.
    let to_one_off = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&income)),
            json!({ "cadence": "one_off" }),
            &owner.cookie,
        )
        .await;
    expect_code(
        &to_one_off,
        http::StatusCode::BAD_REQUEST,
        "plan_change_invalid_shape",
    )
    .await;
    let to_expense = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&income)),
            json!({ "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    expect_code(
        &to_expense,
        http::StatusCode::BAD_REQUEST,
        "plan_change_invalid_shape",
    )
    .await;

    let full = get_plan(&app, &owner, &plan_id).await;
    assert_eq!(
        full["blocks"][0]["changes"].as_array().unwrap().len(),
        1,
        "ningún rechazo deja una fila"
    );
    assert_eq!(full["revision"], 2, "ni sube la revisión");
}

#[tokio::test]
async fn exactly_one_active_plan_per_person() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let a = id_of(&create_plan(&app, &owner, "A").await);
    let b = id_of(&create_plan(&app, &owner, "B").await);

    let act_a = app
        .post_json_with_cookie(&format!("/v1/plans/{a}/activate"), json!({}), &owner.cookie)
        .await;
    assert_eq!(act_a.status, http::StatusCode::OK, "{act_a:?}");
    assert_eq!(act_a.json()["active_plan_id"], a.as_str());
    assert_eq!(act_a.json()["deactivated_plan_ids"], json!([]));
    assert_eq!(act_a.json()["changed"], true);

    let act_b = app
        .post_json_with_cookie(&format!("/v1/plans/{b}/activate"), json!({}), &owner.cookie)
        .await;
    assert_eq!(
        act_b.json()["deactivated_plan_ids"],
        json!([a]),
        "activar B apaga A"
    );
    let list = app.get_with_cookie("/v1/plans", &owner.cookie).await.json();
    let active: Vec<&str> = list["plans"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["is_active"] == true)
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(active, vec![b.as_str()]);
    assert_eq!(list["plans"][0]["id"], b.as_str(), "el activo, el primero");

    let rev_b = get_plan(&app, &owner, &b).await["revision"].clone();
    let again = app
        .post_json_with_cookie(&format!("/v1/plans/{b}/activate"), json!({}), &owner.cookie)
        .await;
    assert_eq!(
        again.json()["changed"],
        false,
        "activar el activo no hace nada"
    );
    assert_eq!(get_plan(&app, &owner, &b).await["revision"], rev_b);

    let off = app
        .post_json_with_cookie("/v1/plans/deactivate", json!({}), &owner.cookie)
        .await;
    assert_eq!(off.status, http::StatusCode::OK, "{off:?}");
    assert_eq!(
        off.json()["active_plan_id"],
        Value::Null,
        "solo la realidad"
    );
    assert_eq!(off.json()["deactivated_plan_ids"], json!([b]));
    let off_again = app
        .post_json_with_cookie("/v1/plans/deactivate", json!({}), &owner.cookie)
        .await;
    assert_eq!(off_again.json()["changed"], false);
    let any_active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans WHERE is_active")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(any_active, 0);

    // La base lo garantiza también por debajo de la API.
    sqlx::query("UPDATE plans SET is_active = true")
        .execute(&app.pool)
        .await
        .expect_err("el índice único parcial impide dos activos de la misma persona");
}

#[tokio::test]
async fn concurrent_activations_leave_exactly_one_active() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let mut ids = Vec::new();
    for n in 0..4 {
        ids.push(id_of(&create_plan(&app, &owner, &format!("P{n}")).await));
    }
    let urls: Vec<String> = (0..8)
        .map(|i| format!("/v1/plans/{}/activate", ids[i % 4]))
        .collect();
    let c = &owner.cookie;
    let (r0, r1, r2, r3, r4, r5, r6, r7) = tokio::join!(
        app.post_json_with_cookie(&urls[0], json!({}), c),
        app.post_json_with_cookie(&urls[1], json!({}), c),
        app.post_json_with_cookie(&urls[2], json!({}), c),
        app.post_json_with_cookie(&urls[3], json!({}), c),
        app.post_json_with_cookie(&urls[4], json!({}), c),
        app.post_json_with_cookie(&urls[5], json!({}), c),
        app.post_json_with_cookie(&urls[6], json!({}), c),
        app.post_json_with_cookie(&urls[7], json!({}), c),
    );
    for r in [r0, r1, r2, r3, r4, r5, r6, r7] {
        // Se serializan (200); el 409 `plan_activation_conflict` es la red si alguna se colara.
        // Lo que no puede salir nunca es un 500 (interbloqueo) ni dos activos.
        assert!(
            r.status == http::StatusCode::OK
                || (r.status == http::StatusCode::CONFLICT
                    && r.json()["code"] == "plan_activation_conflict"),
            "{r:?}"
        );
    }
    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plans WHERE is_active")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(active, 1, "exactamente uno activo tras la carrera");
}

#[tokio::test]
async fn deletes_cascade_and_orphaned_sales_keep_no_subject() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let asset_cat = app.create_category(&owner, "asset", "Inversión").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": asset_cat, "name": "Fondo", "current_value": "10000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    let asset_id = id_of(&asset.json());

    let plan = create_plan(&app, &owner, "Ventas").await;
    let plan_id = id_of(&plan);
    let extra = app
        .post_json_with_cookie(
            &format!("/v1/plans/{plan_id}/blocks"),
            json!({ "name": "Extra" }),
            &owner.cookie,
        )
        .await;
    let extra_id = id_of(&extra.json());
    let house = add_change(
        &app,
        &owner,
        &plan_id,
        json!({ "block_id": extra_id, "kind": "asset_in", "start_date": "2029-01-01",
                "amount": "1000", "params": { "name": "Casa" } }),
    )
    .await;
    let sell_house = add_change(
        &app,
        &owner,
        &plan_id,
        json!({ "kind": "asset_sale", "start_date": "2040-01-01", "amount": "500",
                "target_change_id": house["id"] }),
    )
    .await;
    let sell_fund = add_change(
        &app,
        &owner,
        &plan_id,
        json!({ "kind": "asset_sale", "start_date": "2035-01-01", "amount": "2",
                "amount_unit": "pct", "cadence": "monthly", "asset_id": asset_id }),
    )
    .await;

    // Borrar el bloque se lleva sus cambios; la venta de OTRO bloque se queda sin sujeto.
    let del_block = app
        .delete_with_cookie(
            &format!("/v1/plans/{plan_id}/blocks/{extra_id}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        del_block.status,
        http::StatusCode::NO_CONTENT,
        "{del_block:?}"
    );
    // Borrar el activo REAL (no es una escritura del plan) deja la otra venta sin sujeto.
    let del_asset = app
        .delete_with_cookie(&format!("/v1/assets/{asset_id}"), &owner.cookie)
        .await;
    assert_eq!(
        del_asset.status,
        http::StatusCode::NO_CONTENT,
        "{del_asset:?}"
    );

    let full = get_plan(&app, &owner, &plan_id).await;
    let changes = full["blocks"][0]["changes"].as_array().unwrap();
    assert_eq!(full["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(changes.len(), 2, "las dos ventas sobreviven");
    for c in changes {
        assert_eq!(c["asset_id"], Value::Null, "{c}");
        assert_eq!(c["target_change_id"], Value::Null, "{c}");
    }
    assert!(changes.iter().any(|c| c["id"] == sell_house["id"]));
    assert!(changes.iter().any(|c| c["id"] == sell_fund["id"]));

    // Una venta sin sujeto no se puede editar sin dársele uno: el PATCH valida el resultado.
    let touch = app
        .patch_json_with_cookie(
            &format!("/v1/plans/{plan_id}/changes/{}", id_of(&sell_house)),
            json!({ "enabled": false }),
            &owner.cookie,
        )
        .await;
    expect_code(
        &touch,
        http::StatusCode::BAD_REQUEST,
        "plan_change_invalid_shape",
    )
    .await;

    let del_plan = app
        .delete_with_cookie(&format!("/v1/plans/{plan_id}"), &owner.cookie)
        .await;
    assert_eq!(del_plan.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("plans").await, 0);
    assert_eq!(app.count_rows("plan_blocks").await, 0);
    assert_eq!(app.count_rows("plan_changes").await, 0);
}

/// La promesa de F2-1: los planes no mueven la proyección todavía. Pero sus escrituras ya
/// invalidan la cache (barato, y correcto el día que F2-4 los cablee).
#[tokio::test]
async fn plans_do_not_move_the_projection_yet_but_invalidate_its_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let asset_cat = app.create_category(&owner, "asset", "Inversión").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": asset_cat, "name": "Fondo", "current_value": "10000",
                    "expected_annual_return_percent": "4" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED);
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);

    let before = app
        .get_with_cookie("/v1/projection/series?months=120", &owner.cookie)
        .await;
    assert_eq!(before.status, http::StatusCode::OK);

    app.warm_default_view(&owner.cookie, &key).await;
    let plan_id = id_of(&create_plan(&app, &owner, "Todo").await);
    app.assert_invalidated(&key, "crear un plan").await;

    app.warm_default_view(&owner.cookie, &key).await;
    add_change(
        &app,
        &owner,
        &plan_id,
        json!({ "kind": "expense", "start_date": "2027-01-01", "amount": "900" }),
    )
    .await;
    app.assert_invalidated(&key, "añadir un cambio").await;

    app.warm_default_view(&owner.cookie, &key).await;
    let act = app
        .post_json_with_cookie(
            &format!("/v1/plans/{plan_id}/activate"),
            json!({}),
            &owner.cookie,
        )
        .await;
    assert_eq!(act.status, http::StatusCode::OK);
    app.assert_invalidated(&key, "activar un plan").await;

    let after = app
        .get_with_cookie("/v1/projection/series?months=120", &owner.cookie)
        .await;
    assert!(!before.json()["points"].as_array().unwrap().is_empty());
    assert_eq!(
        before.json(),
        after.json(),
        "F2-1 no cablea los planes: la proyección no puede moverse"
    );
}

#[tokio::test]
async fn the_recent_changes_feed_covers_plans() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let plan_id = id_of(&create_plan(&app, &owner, "Feed").await);
    let r = app
        .get_with_cookie("/v1/changes?since=2000-01-01", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert!(
        b["tables_covered"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "plans"),
        "{b}"
    );
    let row = b["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["entity"] == "plan")
        .unwrap_or_else(|| panic!("el plan no aparece en el feed: {b}"))
        .clone();
    assert_eq!(row["id"], plan_id.as_str());
    assert_eq!(row["label"], "Feed");
}
