//! **D21 (5.0.0)** — toda mutación del ledger exige que la fila sea del usuario de la sesión.
//!
//! El contrato, tabla por tabla (activos, pasivos, presupuesto, próximos, reglas de asignación):
//!
//! * fila de OTRO miembro → **403 `not_row_owner`**, y el rol `owner` **tampoco** salta la regla:
//!   ser dueño de la instalación no es ser dueño de la fila;
//! * fila que no existe → **404**, exactamente como antes (una mutación sobre un id inventado no
//!   puede empezar a devolver 403: el cliente perdería la señal de «esto ya no está»);
//! * la LECTURA no cambia: `?view=household` sigue enseñando el hogar entero. `view` nunca fue
//!   una frontera de autorización (D2) y sigue sin serlo; lo que cambia es la escritura.
//!
//! Por qué importa: desde 5.0.0 cada fila del ledger pertenece a la simulación de UNA persona
//! (D9, proyecciones independientes por miembro). Editar la fila de otro no es «colaborar»: es
//! mover su plan de jubilación sin que se entere, y sin ninguna huella que lo diga.
//!
//! El caso del `reorder` de reglas es distinto y está aparte: reordenar en `household` renumeraba
//! de golpe las cascadas de TODOS los miembros — la única mutación que tocaba filas ajenas por
//! diseño. Pasa a ser 400 `household_read_only`.

mod common;

use common::{LoggedInOwner, TestApp};

/// Crea una fila de cada una de las cinco tablas a nombre de `who` y devuelve sus ids
/// `(asset, liability, budget_entry, planning_flow, allocation_rule)`.
async fn seed_ledger(app: &TestApp, who: &LoggedInOwner, tag: &str) -> [String; 5] {
    let asset_cat = app.create_category(who, "asset", &format!("Inversión {tag}")).await;
    let liab_cat = app.create_category(who, "liability", &format!("Préstamo {tag}")).await;
    let expense_cat = app.create_category(who, "expense", &format!("Cuota {tag}")).await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": format!("Fondo {tag}"),
                "current_value": "10000"
            }),
            &who.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    let asset_id = asset.json()["id"].as_str().expect("asset id").to_string();
    // El PRIMER activo del scope siembra su regla `remainder` (#150): esa es la regla que se usa
    // como fila de reglas del propietario.
    let rule_id = asset.json()["seeded_allocation_rule_id"]
        .as_str()
        .expect("el primer activo del scope siembra su sumidero")
        .to_string();

    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": liab_cat,
                "expense_category_id": expense_cat,
                "label": format!("Préstamo {tag}"),
                "principal": "5000"
            }),
            &who.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");
    let liab_id = liab.json()["id"].as_str().expect("liability id").to_string();

    let budget_cat = app
        .create_category(who, "expense", &format!("Supermercado {tag}"))
        .await;
    let entry = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            serde_json::json!({"category_id": budget_cat, "amount": "300"}),
            &who.cookie,
        )
        .await;
    assert_eq!(entry.status, http::StatusCode::CREATED, "{entry:?}");
    let entry_id = entry.json()["id"].as_str().expect("entry id").to_string();

    let flow = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            serde_json::json!({
                "category_id": budget_cat,
                "title": format!("Viaje {tag}"),
                "expected_amount": "1200"
            }),
            &who.cookie,
        )
        .await;
    assert_eq!(flow.status, http::StatusCode::CREATED, "{flow:?}");
    let flow_id = flow.json()["id"].as_str().expect("flow id").to_string();

    [asset_id, liab_id, entry_id, flow_id, rule_id]
}

/// Las cinco tablas, con su PATCH y su DELETE. Un `member` no puede tocar las filas del owner…
#[tokio::test]
async fn a_member_cannot_touch_another_members_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;

    let [asset, liab, entry, flow, rule] = seed_ledger(&app, &owner, "alice").await;

    let patches: Vec<(String, serde_json::Value)> = vec![
        (format!("/v1/assets/{asset}"), serde_json::json!({"name": "secuestrado"})),
        (format!("/v1/liabilities/{liab}"), serde_json::json!({"label": "secuestrado"})),
        (format!("/v1/budget/entries/{entry}"), serde_json::json!({"amount": "1"})),
        (format!("/v1/planning/flows/{flow}"), serde_json::json!({"title": "secuestrado"})),
        (format!("/v1/allocation-rules/{rule}"), serde_json::json!({"enabled": false})),
    ];
    for (path, body) in &patches {
        let r = app.patch_json_with_cookie(path, body.clone(), &member.cookie).await;
        assert_eq!(
            r.status,
            http::StatusCode::FORBIDDEN,
            "PATCH {path} de otro miembro debía dar 403: {r:?}"
        );
        assert_eq!(r.json()["code"], "not_row_owner", "PATCH {path}: {r:?}");
    }

    for path in [
        format!("/v1/assets/{asset}"),
        format!("/v1/liabilities/{liab}"),
        format!("/v1/budget/entries/{entry}"),
        format!("/v1/planning/flows/{flow}"),
        format!("/v1/allocation-rules/{rule}"),
    ] {
        let r = app.delete_with_cookie(&path, &member.cookie).await;
        assert_eq!(
            r.status,
            http::StatusCode::FORBIDDEN,
            "DELETE {path} de otro miembro debía dar 403: {r:?}"
        );
        assert_eq!(r.json()["code"], "not_row_owner", "DELETE {path}: {r:?}");
    }

    // Y nada se ha movido: el 403 es un rechazo, no un rechazo a medias.
    let listed = app.get_with_cookie("/v1/assets", &owner.cookie).await;
    let rows = listed.json();
    let row = rows
        .as_array()
        .expect("assets array")
        .iter()
        .find(|a| a["id"] == asset.as_str())
        .expect("el activo sigue ahí");
    assert_eq!(row["name"], "Fondo alice", "{row}");
}

/// …y el `owner` de la instalación TAMPOCO. Es la mitad del contrato que se olvida: ser dueño
/// del hogar no es ser dueño de la fila, y el owner es justo quien tiene el botón a mano.
#[tokio::test]
async fn the_installation_owner_does_not_bypass_the_rule() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;

    let [asset, liab, entry, flow, rule] = seed_ledger(&app, &member, "bob").await;

    for (path, body) in [
        (format!("/v1/assets/{asset}"), serde_json::json!({"name": "x"})),
        (format!("/v1/liabilities/{liab}"), serde_json::json!({"label": "x"})),
        (format!("/v1/budget/entries/{entry}"), serde_json::json!({"amount": "1"})),
        (format!("/v1/planning/flows/{flow}"), serde_json::json!({"title": "x"})),
        (format!("/v1/allocation-rules/{rule}"), serde_json::json!({"enabled": false})),
    ] {
        let r = app.patch_json_with_cookie(&path, body, &owner.cookie).await;
        assert_eq!(
            r.status,
            http::StatusCode::FORBIDDEN,
            "el owner tampoco puede editar {path}: {r:?}"
        );
        assert_eq!(r.json()["code"], "not_row_owner", "{path}: {r:?}");
    }

    // La LECTURA del hogar sigue siendo del hogar: D21 no toca `?view`.
    let listed = app.get_with_cookie("/v1/assets", &owner.cookie).await;
    assert_eq!(listed.status, http::StatusCode::OK, "{listed:?}");
    assert!(
        listed
            .json()
            .as_array()
            .expect("assets array")
            .iter()
            .any(|a| a["id"] == asset.as_str()),
        "el owner sigue VIENDO el activo del miembro: {listed:?}"
    );
}

/// Un id que no existe sigue siendo 404. Sin este test, la forma más fácil de implementar D21
/// («devuelve 403 si no casa la fila») convertiría todo 404 en 403 y el cliente perdería la señal
/// de «esto ya no está».
#[tokio::test]
async fn an_unknown_id_is_still_a_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ghost = "00000000-0000-4000-8000-000000000001";

    for (path, body) in [
        (format!("/v1/assets/{ghost}"), serde_json::json!({"name": "x"})),
        (format!("/v1/liabilities/{ghost}"), serde_json::json!({"label": "x"})),
        (format!("/v1/planning/flows/{ghost}"), serde_json::json!({"title": "x"})),
        (
            format!("/v1/allocation-rules/{ghost}"),
            serde_json::json!({"enabled": false}),
        ),
    ] {
        let r = app.patch_json_with_cookie(&path, body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::NOT_FOUND, "PATCH {path}: {r:?}");
        let d = app.delete_with_cookie(&path, &owner.cookie).await;
        assert_eq!(d.status, http::StatusCode::NOT_FOUND, "DELETE {path}: {d:?}");
    }

    // El presupuesto tiene su propio desambiguador (un id de pasivo → 422); un id inventado sigue
    // cayendo a su 404 de siempre.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{ghost}"),
            serde_json::json!({"amount": "1"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NOT_FOUND, "{r:?}");
}

/// Cada uno SÍ puede con lo suyo: la regla acota, no bloquea.
#[tokio::test]
async fn everyone_can_still_edit_their_own_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;

    let [asset, _liab, _entry, _flow, _rule] = seed_ledger(&app, &member, "bob").await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{asset}"),
            serde_json::json!({"current_value": "12345"}),
            &member.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["current_value"], "12345.0000");

    let d = app
        .delete_with_cookie(&format!("/v1/assets/{asset}"), &member.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "{d:?}");
}

/// Reordenar la cascada es POR MIEMBRO desde 5.0.0: `household` deja de ser reordenable.
///
/// No es un permiso que falte (por eso 400 y no 403): es que la vista agregada no admite esta
/// operación. Hasta 4.15.x un reorder en `household` renumeraba de golpe las reglas de todos los
/// miembros, y con proyecciones independientes eso mueve la cascada de otra persona.
#[tokio::test]
async fn reordering_allocation_rules_is_per_member_only() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let [_asset, _liab, _entry, _flow, rule] = seed_ledger(&app, &owner, "alice").await;

    // Sin `?view` (= household) → 400 con el código que dice qué pedir en su lugar.
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules/reorder",
            serde_json::json!({"ids": [rule]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "household_read_only", "{r:?}");

    // Explícito tampoco.
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules/reorder?view=household",
            serde_json::json!({"ids": [rule]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "household_read_only", "{r:?}");

    // Con `?view=mine` funciona como siempre.
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules/reorder?view=mine",
            serde_json::json!({"ids": [rule]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json().as_array().expect("rules array").len(), 1, "{r:?}");
}
