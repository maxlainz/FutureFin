//! `GET /v1/changes` — la mitad honesta de una auditoría.
//!
//! Cubre altas y ediciones sobre los `updated_at` que ocho tablas ya mantienen. **No cubre
//! borrados** (no hay tombstones) y no ve `categories` ni `allocation_rules` (no tienen
//! `updated_at`). Los tests fijan las dos cosas: que lo que promete lo cumple, y que **lo que no
//! cubre lo dice en la respuesta** en vez de dejar una lista vacía ambigua.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

fn entities(b: &Value) -> Vec<String> {
    b["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|c| c["entity"].as_str().unwrap().to_string())
        .collect()
}

fn find<'a>(b: &'a Value, entity: &str) -> &'a Value {
    b["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .find(|c| c["entity"] == entity)
        .unwrap_or_else(|| panic!("no hay cambio de tipo {entity} en {b}"))
}

async fn setup(app: &TestApp) -> LoggedInOwner {
    app.register_and_login_owner("alice").await
}

#[tokio::test]
async fn changes_lists_creations_and_edits_and_declares_what_it_cannot_see() {
    let app = TestApp::spawn().await;
    let owner = setup(&app).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Indexado", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();

    let entry = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": cat_exp, "amount": "500"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(entry.status, http::StatusCode::CREATED, "{entry:?}");

    let b = app
        .get_with_cookie("/v1/changes?since=1970-01-01", &owner.cookie)
        .await;
    assert_eq!(b.status, http::StatusCode::OK, "{b:?}");
    let b = b.json();

    // El aviso: es lo que separa «no ha pasado nada» de «no lo puedo ver».
    assert_eq!(b["covers_deletions"], false, "{b}");
    assert_eq!(b["deletions_absent_reason"], "no_tombstones", "{b}");
    assert_eq!(
        b["tables_missing_updated_at"],
        json!(["categories", "allocation_rules"]),
        "las dos tablas invisibles se nombran: {b}"
    );
    assert_eq!(b["tables_covered"].as_array().unwrap().len(), 8, "{b}");
    // 5.0.0 (R2): sin `?view` la respuesta es `mine`. Este test tiene un solo usuario, así que
    // el contenido no cambia — lo que cambia es lo que la respuesta DECLARA haber aplicado.
    assert_eq!(b["view"], "mine", "{b}");
    assert!(b["since"].as_str().unwrap().starts_with("1970-01-01"), "{b}");

    let ents = entities(&b);
    assert!(ents.contains(&"asset".to_string()), "{b}");
    assert!(ents.contains(&"budget_entry".to_string()), "{b}");
    assert!(
        !ents.iter().any(|e| e == "category"),
        "las categorías no tienen updated_at y no pueden aparecer: {b}"
    );

    let a = find(&b, "asset");
    assert_eq!(a["id"].as_str().unwrap(), asset_id, "{b}");
    assert_eq!(a["label"], "Indexado", "{b}");
    // La partida de presupuesto no tiene nombre propio: se identifica por su categoría.
    assert_eq!(find(&b, "budget_entry")["label"], "Vida", "{b}");
    assert_eq!(a["change"], "created", "nació dentro de la ventana: {b}");

    assert_eq!(
        b["item_count"], b["items_included"],
        "sin truncar, los dos contadores coinciden: {b}"
    );
    assert_eq!(b["truncated"], false, "{b}");
}

/// Una fila que ya existía antes de `since` y se edita después sale como `updated`, no `created`.
/// Y una que se **borra** no sale de ninguna manera: eso es lo que el endpoint no puede hacer.
#[tokio::test]
async fn an_edit_reads_as_updated_and_a_deletion_leaves_no_trace() {
    let app = TestApp::spawn().await;
    let owner = setup(&app).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;

    let keep = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Indexado", "current_value": "1000"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let doomed = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Efímero", "current_value": "1"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Corte: todo lo anterior queda fuera de la ventana.
    let cut = chrono::Utc::now().to_rfc3339();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let p = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{keep}"),
            json!({"current_value": "1200"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    let d = app
        .delete_with_cookie(&format!("/v1/assets/{doomed}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "{d:?}");

    let b = app
        .get_with_cookie(
            &format!("/v1/changes?since={}", urlencoding_lite(&cut)),
            &owner.cookie,
        )
        .await;
    assert_eq!(b.status, http::StatusCode::OK, "{b:?}");
    let b = b.json();

    let assets: Vec<&Value> = b["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["entity"] == "asset")
        .collect();
    assert_eq!(assets.len(), 1, "solo la editada: {b}");
    assert_eq!(assets[0]["id"].as_str().unwrap(), keep, "{b}");
    assert_eq!(
        assets[0]["change"], "updated",
        "existía antes del corte, así que es una edición: {b}"
    );
    assert!(
        !b["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"].as_str() == Some(doomed.as_str())),
        "un borrado no deja rastro — por eso esto no es una auditoría: {b}"
    );
}

#[tokio::test]
async fn since_is_required_and_limit_is_bounded() {
    let app = TestApp::spawn().await;
    let owner = setup(&app).await;

    let r = app.get_with_cookie("/v1/changes", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "date_required");

    let r = app
        .get_with_cookie("/v1/changes?since=ayer", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "date_invalid");

    let r = app
        .get_with_cookie("/v1/changes?since=1970-01-01&limit=0", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "limit_out_of_range");

    let r = app
        .get_with_cookie("/v1/changes?since=1970-01-01&limit=501", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "limit_out_of_range");
}

/// Con `limit` por debajo del total, la respuesta declara el recorte en vez de dejar una lista
/// corta indistinguible de «no hay más» (D22/I18).
#[tokio::test]
async fn a_truncated_page_says_so() {
    let app = TestApp::spawn().await;
    let owner = setup(&app).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    for i in 0..3 {
        app.post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": format!("A{i}"), "current_value": "1"}),
            &owner.cookie,
        )
        .await;
    }

    let b = app
        .get_with_cookie("/v1/changes?since=1970-01-01&limit=2", &owner.cookie)
        .await
        .json();
    assert_eq!(b["items_included"], 2, "{b}");
    assert!(b["item_count"].as_i64().unwrap() >= 3, "{b}");
    assert_eq!(b["truncated"], true, "{b}");
}

/// `+` en un timestamp RFC 3339 con offset se leería como espacio en una query string.
fn urlencoding_lite(s: &str) -> String {
    s.replace('+', "%2B")
}
