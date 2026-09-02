//! El `null` presente en un PATCH borra de verdad (issues #95 y #113, Ola 1 de la resolución).
//!
//! Serde colapsa `"campo": null` con «clave ausente» en `Option<T>`, así que las ramas
//! `Value::Null` de estos seis campos eran código muerto: el contrato publicado prometía
//! «`null` borra» y el binario devolvía 200 sin efecto (en `birth_date`, además, sobre un
//! input del engine — el horizonte). Con `deserialize_double_option` el trío por campo es:
//! `null` → borra · ausente → intacto · valor → aplica.

mod common;
use common::TestApp;
use serde_json::{json, Value};

#[tokio::test]
async fn birth_date_null_clears_absent_keeps_value_sets() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;

    // valor → aplica
    let r = app
        .patch_json_with_cookie("/v1/auth/me", json!({"birth_date": "1990-05-01"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let me = app.get_with_cookie("/v1/auth/me", &owner.cookie).await.json();
    assert_eq!(me["birth_date"], "1990-05-01");

    // ausente → intacto
    let r = app
        .patch_json_with_cookie("/v1/auth/me", json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let me = app.get_with_cookie("/v1/auth/me", &owner.cookie).await.json();
    assert_eq!(me["birth_date"], "1990-05-01", "un cuerpo vacío no puede tocar la fecha");

    // null → borra (antes: 200 sin UPDATE — el bug de #113)
    let r = app
        .patch_json_with_cookie("/v1/auth/me", json!({"birth_date": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let me = app.get_with_cookie("/v1/auth/me", &owner.cookie).await.json();
    assert!(me["birth_date"].is_null(), "null presente debe borrar: {me}");
}

#[tokio::test]
async fn purchase_price_null_clears_absent_keeps_value_sets() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bea").await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat, "name": "Indexado", "current_value": "10000",
                   "is_liquid": true, "purchase_price": "8000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let id = r.json()["id"].as_str().unwrap().to_string();

    // ausente → intacto
    let r = app
        .patch_json_with_cookie(&format!("/v1/assets/{id}"), json!({"name": "Indexado MSCI"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["purchase_price"].as_str().map(|s| s.starts_with("8000")), Some(true));

    // valor → aplica
    let r = app
        .patch_json_with_cookie(&format!("/v1/assets/{id}"), json!({"purchase_price": "9000"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["purchase_price"].as_str().map(|s| s.starts_with("9000")), Some(true));

    // null → borra (la promesa de OpenAPI que era inalcanzable — #95)
    let r = app
        .patch_json_with_cookie(&format!("/v1/assets/{id}"), json!({"purchase_price": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(r.json()["purchase_price"].is_null(), "{:?}", r.json());
}

#[tokio::test]
async fn allocation_cap_null_clears_and_amount_null_hits_the_live_validation() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("carla").await;
    let cat = app.create_category(&owner, "asset", "Cartera").await;
    async fn mk_asset(app: &TestApp, cookie: &str, cat: &str, name: &str) -> String {
        let r = app
            .post_json_with_cookie(
                "/v1/assets",
                json!({"category_id": cat, "name": name, "current_value": "1000", "is_liquid": true}),
                cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
        r.json()["id"].as_str().unwrap().to_string()
    }
    let a1 = mk_asset(&app, &owner.cookie, &cat, "Fondo A").await;
    let a2 = mk_asset(&app, &owner.cookie, &cat, "Sumidero").await;

    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({"target_asset_id": a1, "kind": "fixed", "amount": "150",
                   "cap_kind": "amount", "cap_value": "5000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let fixed_id = r.json()["id"].as_str().unwrap().to_string();
    // #150: "Fondo A" (a1) fue el primer activo del owner → ya sembró el sumidero apuntándole.
    // Retargeteamos al activo pensado como sumidero ("Sumidero"/a2) en vez de crear uno segundo.
    let seeded = app.sink_rule_id(&owner.cookie).await;
    let retarget = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{seeded}"),
            json!({"target_asset_id": a2}),
            &owner.cookie,
        )
        .await;
    assert_eq!(retarget.status, http::StatusCode::OK, "{retarget:?}");

    // ausente → cap intacto
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{fixed_id}"),
            json!({"amount": "175"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let body: Value = r.json();
    assert_eq!(body["cap_kind"], "amount", "{body}");

    // cap: null → borra el par (el doc-comment lo prometía; la rama era inalcanzable)
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{fixed_id}"),
            json!({"cap": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let body: Value = r.json();
    assert!(body["cap_kind"].is_null(), "{body}");
    assert!(body["cap_value"].is_null(), "{body}");

    // amount: null sobre una regla `fixed` → ahora la rama VIVE y valida: 400, no 200 mudo.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{fixed_id}"),
            json!({"amount": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
}

#[tokio::test]
async fn planning_due_date_null_clears_absent_keeps_value_sets() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("dora").await;
    let cat = app.create_category(&owner, "income", "Extra").await;
    let r = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            json!({"category_id": cat, "title": "Devolución renta",
                   "expected_amount": "900", "due_date": "2027-06-30"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let id = r.json()["id"].as_str().unwrap().to_string();

    // ausente → intacta
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/planning/flows/{id}"),
            json!({"expected_amount": "950"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["due_date"], "2027-06-30");

    // null → borra
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/planning/flows/{id}"),
            json!({"due_date": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(r.json()["due_date"].is_null(), "{:?}", r.json());

    // valor → vuelve a aplicar
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/planning/flows/{id}"),
            json!({"due_date": "2028-01-15"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["due_date"], "2028-01-15");
}

#[tokio::test]
async fn fire_settings_null_resets_to_defaults() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("eva").await;

    // Los defaults se capturan ANTES de tocar nada (autocontenido: no congelamos literales).
    let defaults = app.get_with_cookie("/v1/installation", &owner.cookie).await.json()
        ["installation"]["fire_settings"]
        .clone();

    // 5.0.0: el SWR salió de `fire_settings` (D13), así que el eje que aleja el objeto de sus
    // defaults es otro — `taxable_gain_ratio`, que sigue siendo del hogar. Lo que se prueba no
    // cambia: que `"fire_settings": null` BORRA el JSONB y la lectura vuelve a los defaults.
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxable_gain_ratio": "0.4"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let fs = app.get_with_cookie("/v1/installation", &owner.cookie).await.json()
        ["installation"]["fire_settings"]
        .clone();
    assert_eq!(fs["taxable_gain_ratio"], "0.4", "{fs}");
    assert_ne!(fs, defaults, "el patch debe alejarlo de los defaults para que el reset pruebe algo");

    // null → borra el JSONB guardado; la lectura vuelve a los defaults (rama `Some(None)` viva)
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({"fire_settings": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let fs = app.get_with_cookie("/v1/installation", &owner.cookie).await.json()
        ["installation"]["fire_settings"]
        .clone();
    assert_eq!(fs, defaults, "null presente debe resetear a defaults");
}

/// S3/#135 (Ola 1): cota superior del TIN. El desliz de coma es-ES (350 por 3,50) entraba, hacía
/// crecer el saldo ×1,29/mes y acababa en el overflow tipado del engine. Ahora: 400 con nombre.
#[tokio::test]
async fn apr_percent_above_100_is_rejected_on_create_and_patch() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("fmt").await;
    let cat = app.create_category(&owner, "liability", "Deudas").await;
    let exp = app.create_category(&owner, "expense", "Cuotas").await;

    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({"category_id": cat, "expense_category_id": exp, "label": "Coma perdida",
                   "principal": "200000", "apr_percent": "350",
                   "payment_amount": "1000", "payment_frequency": "monthly",
                   "repayment_model": "french"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    let msg = String::from_utf8_lossy(&r.body).to_string();
    assert!(msg.contains("apr_out_of_range"), "{msg}");

    // Un TIN de mercado (27) entra; y el PATCH aplica la misma puerta.
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({"category_id": cat, "expense_category_id": exp, "label": "Revolving",
                   "principal": "5000", "apr_percent": "27",
                   "payment_amount": "150", "payment_frequency": "monthly",
                   "repayment_model": "french"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let id = r.json()["id"].as_str().unwrap().to_string();
    let r = app
        .patch_json_with_cookie(&format!("/v1/liabilities/{id}"), json!({"apr_percent": "101"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
}
