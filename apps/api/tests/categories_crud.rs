//! `PATCH` / `DELETE /v1/categories/{id}` — las cores que faltaban, y el desglose del preview.
//!
//! `create_category` sin contraparte es un pozo sin fondo: el catálogo es de la instalación entera,
//! así que una categoría creada por error se queda ahí para siempre. Estos tests fijan la
//! contraparte (editar y borrar), las cuatro reglas del `remap_to`, y que el borrado **arrastra**
//! la atribución de gasto de las cuotas de pasivo en vez de degradarla a `NULL`.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};
use uuid::Uuid;

async fn cat(app: &TestApp, owner: &LoggedInOwner, scope: &str, name: &str) -> String {
    app.create_category(owner, scope, name).await
}

async fn list_categories(app: &TestApp, owner: &LoggedInOwner) -> Vec<Value> {
    let r = app.get_with_cookie("/v1/categories", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    r.json().as_array().unwrap().clone()
}

// ---------------------------------------------------------------------------
// PATCH
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patching_a_category_renames_it_and_keeps_its_scope() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = cat(&app, &owner, "expense", "Comida").await;

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{id}"),
            json!({"name": "  Alimentación  ", "sort_index": 7}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["name"], "Alimentación", "el nombre se normaliza: {b}");
    assert_eq!(b["sort_index"], 7);
    assert_eq!(b["scope"], "expense", "el scope es inmutable: {b}");
}

#[tokio::test]
async fn patching_with_an_empty_body_is_rejected_instead_of_a_silent_200() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = cat(&app, &owner, "expense", "Comida").await;

    let r = app
        .patch_json_with_cookie(&format!("/v1/categories/{id}"), json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty");
}

#[tokio::test]
async fn renaming_onto_an_existing_name_in_the_same_scope_conflicts() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = cat(&app, &owner, "expense", "Comida").await;
    let _b = cat(&app, &owner, "expense", "Ocio").await;

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{a}"),
            json!({"name": "Ocio"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CONFLICT, "{r:?}");
}

#[tokio::test]
async fn patching_a_category_of_another_installation_is_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{}", Uuid::new_v4()),
            json!({"name": "X"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NOT_FOUND, "{r:?}");
}

// ---------------------------------------------------------------------------
// DELETE + remap_to
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deleting_an_unused_category_needs_no_remap() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = cat(&app, &owner, "expense", "Sobrante").await;
    let before = list_categories(&app, &owner).await.len();

    let r = app
        .delete_with_cookie(&format!("/v1/categories/{id}"), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::NO_CONTENT, "{r:?}");
    assert_eq!(list_categories(&app, &owner).await.len(), before - 1);
}

#[tokio::test]
async fn deleting_a_used_category_demands_a_valid_remap_target() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let comida = cat(&app, &owner, "expense", "Comida").await;
    let ocio = cat(&app, &owner, "expense", "Ocio").await;
    let fondos = cat(&app, &owner, "asset", "Fondos").await;

    let e = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": comida, "amount": "300"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(e.status, http::StatusCode::CREATED, "{e:?}");

    // 1. Sin remap_to.
    let r = app
        .delete_with_cookie(&format!("/v1/categories/{comida}"), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "category_in_use");

    // 2. remap_to = ella misma.
    let r = app
        .delete_with_cookie(
            &format!("/v1/categories/{comida}?remap_to={comida}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "remap_to_same_category");

    // 3. remap_to inexistente.
    let r = app
        .delete_with_cookie(
            &format!("/v1/categories/{comida}?remap_to={}", Uuid::new_v4()),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "remap_to_not_found");

    // 4. remap_to de otro scope.
    let r = app
        .delete_with_cookie(
            &format!("/v1/categories/{comida}?remap_to={fondos}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "remap_to_scope_mismatch");

    // 5. El destino correcto: la partida sobrevive apuntando a `ocio`.
    let r = app
        .delete_with_cookie(
            &format!("/v1/categories/{comida}?remap_to={ocio}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NO_CONTENT, "{r:?}");

    let budget = app.get_with_cookie("/v1/budget", &owner.cookie).await.json();
    let entries = budget["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "la partida no se borra, se remapea: {budget}");
    assert_eq!(entries[0]["category_id"].as_str().unwrap(), ocio, "{budget}");
}

/// `liabilities.expense_category_id` tiene FK `ON DELETE SET NULL`: no bloquea el borrado y por eso
/// no cuenta en `references_total`. Pero un remap **sí** debe llevárselo, o la atribución de la
/// cuota se degrada a `NULL` en silencio en cuanto alguien reorganiza sus categorías de gasto.
#[tokio::test]
async fn remapping_an_expense_category_follows_the_liability_quota_attribution() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cuotas = cat(&app, &owner, "expense", "Cuotas").await;
    let deudas = cat(&app, &owner, "expense", "Deudas").await;
    let prestamos = cat(&app, &owner, "liability", "Préstamos").await;

    let future = (chrono::Utc::now().date_naive() + chrono::Duration::days(400))
        .format("%Y-%m-%d")
        .to_string();
    let l = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({
                "category_id": prestamos, "expense_category_id": cuotas,
                "label": "Hipoteca", "principal": "10000",
                "payment_amount": "200", "payment_frequency": "monthly",
                "payment_end_date": future,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l.status, http::StatusCode::CREATED, "{l:?}");

    // `Cuotas` no está en `references_total` (SET NULL), así que se borra sin `remap_to`… pero
    // entonces la atribución se perdería. Con `remap_to` la sigue.
    let r = app
        .delete_with_cookie(
            &format!("/v1/categories/{cuotas}?remap_to={deudas}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NO_CONTENT, "{r:?}");

    let liabs = app
        .get_with_cookie("/v1/liabilities", &owner.cookie)
        .await
        .json();
    let first = liabs
        .as_array()
        .and_then(|a| a.first())
        .unwrap_or_else(|| panic!("se esperaba un pasivo: {liabs}"));
    assert_eq!(
        first["expense_category_id"].as_str(),
        Some(deudas.as_str()),
        "la atribución de la cuota debe seguir al remap: {liabs}"
    );
}

/// Contrato histórico del módulo: las categorías **no** invalidan la cache de proyección. Renombrar
/// o remapear no mueve ningún número del motor — el engine agrega por importe, no por categoría.
#[tokio::test]
async fn category_mutations_do_not_touch_the_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = cat(&app, &owner, "expense", "Comida").await;
    let b = cat(&app, &owner, "expense", "Ocio").await;
    let iid: Uuid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    app.settle_login_warmup(iid).await;

    app.warm_default_view(&owner.cookie, &key).await;
    let p = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{a}"),
            json!({"name": "Alimentación"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    assert!(
        app.cache_contains(&key).await,
        "PATCH /v1/categories no debe invalidar la proyección"
    );

    let d = app
        .delete_with_cookie(&format!("/v1/categories/{b}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "{d:?}");
    assert!(
        app.cache_contains(&key).await,
        "DELETE /v1/categories no debe invalidar la proyección"
    );
}

/// La categoría POR DEFECTO (4.15.0) no se puede borrar, pero **sí es un destino válido de
/// `remap_to`** — de hecho es el que la UI ofrecerá por defecto al borrar. Un veto que se
/// extendiera al destino dejaría el borrado sin salida natural.
#[tokio::test]
async fn the_default_category_is_a_valid_remap_destination() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let comida = cat(&app, &owner, "expense", "Comida").await;
    let fb: String = sqlx::query_scalar(
        "SELECT id::text FROM categories \
         WHERE installation_id = (SELECT id FROM installation LIMIT 1) \
           AND scope = 'expense' AND is_fallback",
    )
    .fetch_one(&app.pool)
    .await
    .expect("categoría por defecto de gasto");

    let e = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": comida, "amount": "300"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(e.status, http::StatusCode::CREATED, "{e:?}");

    let r = app
        .delete_with_cookie(
            &format!("/v1/categories/{comida}?remap_to={fb}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NO_CONTENT, "{r:?}");

    let moved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM budget_entries WHERE category_id = $1::uuid")
        .bind(Uuid::parse_str(&fb).unwrap())
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(moved, 1, "la partida se movió al cajón");
    assert!(
        !list_categories(&app, &owner).await.iter().any(|c| c["id"] == json!(comida)),
        "la categoría borrada ya no está"
    );
}
