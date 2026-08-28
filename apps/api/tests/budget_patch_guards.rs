//! Guardias de forma del `PATCH /v1/budget/entries/{id}` (Fase 2, issue #83).
//!
//! El caso: `expense_end_date` **y** `clear_expense_end_date: true` en la MISMA llamada. La
//! guardia existía solo en la tool MCP `update_budget_entry`, mientras `patch_budget_entry_core`
//! dejaba ganar al `clear` en silencio — o sea, la superficie **derivada** era más estricta que la
//! fuente, exactamente lo contrario del contrato D14 (las tools comparten la core; cero deriva).
//! Por HTTP/SPA el resultado era un 200 con la fecha fin a NULL: la partida deja de terminar
//! nunca, el gasto se extiende hasta el horizonte y la proyección se mueve sin que nadie lo pida.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::TestApp;
use serde_json::json;

/// Crea una partida de gasto con fecha fin y devuelve su id.
async fn entry_with_end_date(app: &TestApp, cookie: &str, cat: &str, end: &str) -> String {
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({ "category_id": cat, "amount": "300", "expense_end_date": end }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(r.json()["expense_end_date"], end);
    r.json()["id"].as_str().unwrap().to_string()
}

/// Poner y borrar la fecha fin a la vez es 400 `expense_end_set_and_clear`, y **no escribe nada**.
///
/// Se comprueba por HTTP a propósito: la core la comparten el handler y la tool, así que si la
/// guardia se moviera de vuelta a la capa MCP este test la echaría en falta inmediatamente.
#[tokio::test]
async fn patch_rejects_setting_and_clearing_the_expense_end_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ocio = app.create_category(&owner, "expense", "Ocio").await;
    let id = entry_with_end_date(&app, &owner.cookie, &ocio, "2030-01-01").await;

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{id}"),
            json!({ "expense_end_date": "2031-06-30", "clear_expense_end_date": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "expense_end_set_and_clear", "{}", r.json());

    // Nada se ha escrito: la fecha original sigue ahí. Antes de la guardia, esta misma llamada
    // devolvía 200 y dejaba `expense_end_date` a null.
    let listed = app.get_with_cookie("/v1/budget", &owner.cookie).await;
    let entry = listed.json()["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == json!(id))
        .cloned()
        .expect("la partida sigue en el presupuesto");
    assert_eq!(entry["expense_end_date"], "2030-01-01", "{entry}");
}

/// Los dos caminos sanos siguen intactos: solo `expense_end_date` reescribe la fecha, y solo
/// `clear_expense_end_date` la borra. La guardia rechaza la combinación, no cada flag por su
/// cuenta — que es el error fácil al añadirla.
#[tokio::test]
async fn setting_or_clearing_the_expense_end_date_alone_still_works() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ocio = app.create_category(&owner, "expense", "Ocio").await;
    let id = entry_with_end_date(&app, &owner.cookie, &ocio, "2030-01-01").await;

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{id}"),
            json!({ "expense_end_date": "2031-06-30" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["expense_end_date"], "2031-06-30");

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{id}"),
            json!({ "clear_expense_end_date": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["expense_end_date"], json!(null), "{}", r.json());

    // `clear_expense_end_date: false` no borra ni bloquea: es el no-op explícito, y con una fecha
    // al lado tampoco entra en conflicto (la guardia mira `== Some(true)`, no `is_some()`).
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{id}"),
            json!({ "expense_end_date": "2032-03-31", "clear_expense_end_date": false }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["expense_end_date"], "2032-03-31");
}

/// El `PATCH /v1/assets/{id}` con un cuerpo vacío ya devuelve `patch_empty` desde la core — la
/// prueba de que la guardia gemela de la tool MCP `update_asset_value` («provide current_value
/// and/or expected_annual_return_percent», sin código y con otro texto) es redundante y puede
/// borrarse sin dejar el caso al descubierto.
///
/// Vive en este fichero por cercanía temática (guardias de forma de PATCH compartidas por HTTP y
/// MCP), no por pertenecer al presupuesto.
#[tokio::test]
async fn asset_patch_with_no_fields_is_covered_by_the_core() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "Indexado", "current_value": "10000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(&format!("/v1/assets/{id}"), json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty", "{}", r.json());
}
