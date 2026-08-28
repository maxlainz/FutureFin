//! Plusvalía latente por activo (`GET /v1/assets`): euros, porcentaje y **los dos motivos**.
//!
//! Los dos inputs (`current_value`, `purchase_price`) ya viajaban; lo caro de esta cifra no es
//! calcularla, es **etiquetarla**: no es rentabilidad (no anualiza ni descuenta las aportaciones
//! mensuales posteriores) y `purchase_price` es opcional, así que hay que distinguir «0 %» de «sin
//! coste declarado» — y, dentro de eso, «coste cero» de «coste desconocido».

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn asset(app: &TestApp, owner: &LoggedInOwner, cat: &str, name: &str, body: Value) -> String {
    let mut b = json!({"category_id": cat, "name": name});
    let obj = b.as_object_mut().unwrap();
    for (k, v) in body.as_object().unwrap() {
        obj.insert(k.clone(), v.clone());
    }
    let r = app.post_json_with_cookie("/v1/assets", b, &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset {name}: {r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

fn by_name<'a>(list: &'a Value, name: &str) -> &'a Value {
    list.as_array()
        .expect("assets array")
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no asset {name} in {list}"))
}

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un decimal como string, no {v}"))
        .parse()
        .unwrap()
}

#[tokio::test]
async fn unrealized_pnl_is_published_with_its_two_absence_reasons() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;

    asset(
        &app,
        &owner,
        &cat,
        "Ganador",
        json!({"current_value": "1500", "purchase_price": "1000"}),
    )
    .await;
    asset(
        &app,
        &owner,
        &cat,
        "Perdedor",
        json!({"current_value": "800", "purchase_price": "1000"}),
    )
    .await;
    asset(
        &app,
        &owner,
        &cat,
        "Plano",
        json!({"current_value": "1000", "purchase_price": "1000"}),
    )
    .await;
    asset(&app, &owner, &cat, "Sin coste", json!({"current_value": "5000"})).await;
    asset(
        &app,
        &owner,
        &cat,
        "Coste cero",
        json!({"current_value": "300", "purchase_price": "0"}),
    )
    .await;

    let list = app.get_with_cookie("/v1/assets", &owner.cookie).await;
    assert_eq!(list.status, http::StatusCode::OK, "{list:?}");
    let list = list.json();

    let g = by_name(&list, "Ganador");
    assert_eq!(dec(&g["unrealized_pnl"]), 500.0, "{g}");
    assert_eq!(dec(&g["unrealized_pnl_pct"]), 50.0, "{g}");
    assert_eq!(g["unrealized_pnl_absent_reason"], Value::Null, "{g}");
    assert_eq!(g["unrealized_pnl_pct_absent_reason"], Value::Null, "{g}");

    let p = by_name(&list, "Perdedor");
    assert_eq!(dec(&p["unrealized_pnl"]), -200.0, "puede ser negativa: {p}");
    assert_eq!(dec(&p["unrealized_pnl_pct"]), -20.0, "{p}");

    // «0 %» real: la cifra existe y vale cero.
    let f = by_name(&list, "Plano");
    assert_eq!(dec(&f["unrealized_pnl"]), 0.0, "{f}");
    assert_eq!(dec(&f["unrealized_pnl_pct"]), 0.0, "{f}");
    assert_eq!(f["unrealized_pnl_absent_reason"], Value::Null, "{f}");

    // …frente a «no sé lo que te costó»: ausencia con motivo, NUNCA un 0.
    let s = by_name(&list, "Sin coste");
    assert_eq!(s["unrealized_pnl"], Value::Null, "{s}");
    assert_eq!(s["unrealized_pnl_pct"], Value::Null, "{s}");
    assert_eq!(s["unrealized_pnl_absent_reason"], "no_purchase_price", "{s}");
    assert_eq!(
        s["unrealized_pnl_pct_absent_reason"], "no_purchase_price",
        "{s}"
    );

    // Coste declarado CERO: los euros existen, el porcentaje no (división entre 0) — y por eso
    // los dos motivos son campos distintos.
    let z = by_name(&list, "Coste cero");
    assert_eq!(dec(&z["unrealized_pnl"]), 300.0, "{z}");
    assert_eq!(z["unrealized_pnl_absent_reason"], Value::Null, "{z}");
    assert_eq!(z["unrealized_pnl_pct"], Value::Null, "{z}");
    assert_eq!(
        z["unrealized_pnl_pct_absent_reason"], "zero_purchase_price",
        "{z}"
    );
}

/// La plusvalía se recalcula en el acto tras editar la valoración: sale de los dos campos de la
/// fila, no de nada cacheado.
#[tokio::test]
async fn unrealized_pnl_follows_an_edit_of_the_valuation() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;
    let id = asset(
        &app,
        &owner,
        &cat,
        "Indexado",
        json!({"current_value": "1000", "purchase_price": "1000"}),
    )
    .await;

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{id}"),
            json!({"current_value": "1250"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(dec(&b["unrealized_pnl"]), 250.0, "{b}");
    assert_eq!(dec(&b["unrealized_pnl_pct"]), 25.0, "{b}");

    // NOTA para quien venga: **`{"purchase_price": null}` por HTTP no borra el precio**, devuelve
    // 400 `patch_empty`. No es un fallo de esta cifra: `Option<serde_json::Value>` con serde mapea
    // el `null` de JSON a `None` (= campo ausente), así que la rama `is_null()` de
    // `merge_optional_decimal_patch` es inalcanzable por este camino. La vía viva para borrarlo es
    // la tool MCP `update_asset` con `clear_purchase_price: true`, que construye el `Value::Null`
    // en Rust. Se fija aquí para que el comportamiento no se lea como un descuido de la plusvalía.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{id}"),
            json!({"purchase_price": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty");
}
