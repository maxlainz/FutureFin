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

    // Retirar el coste declarado devuelve el activo a «sin coste declarado», y la plusvalía deja
    // de existir con su motivo — no se queda en 0, que sería la afirmación distinta «no has ganado
    // ni perdido».
    //
    // HISTORIA (issue #95, arreglado en 4.4.2): este bloque fijaba lo contrario — un 400
    // `patch_empty` —, porque `Option<serde_json::Value>` con el `Deserialize` por defecto de
    // serde mapea el `null` de JSON a `None` (= clave ausente) y la rama `is_null()` de
    // `merge_optional_decimal_patch` era inalcanzable por HTTP. El campo es ahora un tri-estado de
    // verdad (`Option<Option<…>>` + `deserialize_double_option`).
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{id}"),
            json!({"purchase_price": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["purchase_price"], Value::Null, "{b}");
    assert_eq!(b["unrealized_pnl"], Value::Null, "{b}");
    assert_eq!(b["unrealized_pnl_absent_reason"], "no_purchase_price", "{b}");
    assert_eq!(b["unrealized_pnl_pct"], Value::Null, "{b}");
    assert_eq!(
        b["unrealized_pnl_pct_absent_reason"], "no_purchase_price",
        "{b}"
    );
}

/// El trío del tri-estado de `PATCH /v1/assets/{id}` sobre `purchase_price`: **`null` borra**,
/// **clave ausente conserva**, **valor sustituye**. Las tres en la misma fila y en ese orden,
/// porque el bug de #95 era exactamente que las dos primeras eran indistinguibles.
///
/// El caso que lo hacía caro no es teórico: la SPA manda `purchase_price: null` en CADA edición de
/// activo cuyo campo de compra se deja vacío (`App.tsx`, «PATCH: siempre enviar precio de compra»),
/// así que vaciar el campo y guardar devolvía 200 y **no borraba nada**. Un 400 se ve; un 200 que
/// no hace lo que dice, no.
#[tokio::test]
async fn patch_purchase_price_is_a_real_tristate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;
    let id = asset(
        &app,
        &owner,
        &cat,
        "Indexado",
        json!({"current_value": "1000", "purchase_price": "800"}),
    )
    .await;

    // 1. `null` presente → borra. Y es el ÚNICO campo del cuerpo: ya no es `patch_empty`.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{id}"),
            json!({"purchase_price": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        r.status,
        http::StatusCode::OK,
        "un PATCH cuyo único contenido es `purchase_price: null` es válido y borra: {r:?}"
    );
    assert_eq!(r.json()["purchase_price"], Value::Null, "{:?}", r.json());

    // Persiste: no es solo la respuesta del PATCH.
    let list = app.get_with_cookie("/v1/assets", &owner.cookie).await.json();
    assert_eq!(by_name(&list, "Indexado")["purchase_price"], Value::Null);

    // 2. Valor → aplica.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{id}"),
            json!({"purchase_price": "950"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(dec(&r.json()["purchase_price"]), 950.0);

    // 3. Clave ausente → intacto. El PATCH toca otro campo para no chocar con `patch_empty`.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{id}"),
            json!({"current_value": "1100"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(dec(&b["purchase_price"]), 950.0, "omitir conserva: {b}");
    assert_eq!(dec(&b["unrealized_pnl"]), 150.0, "{b}");

    // El cuerpo VACÍO sigue siendo 400: el tri-estado abre `null`, no la puerta de atrás.
    let r = app
        .patch_json_with_cookie(&format!("/v1/assets/{id}"), json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty");
}
