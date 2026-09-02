//! La categoría POR DEFECTO de cada scope (`categories.is_fallback`, 4.15.0).
//!
//! Antes de 4.15.0 «sin categoría» era un estado persistible de un ingreso o un gasto, y el
//! resultado era una atribución que mentía sin que ningún total dejara de cuadrar: los importes
//! sumaban bien y el desglose por categoría se comía la diferencia en un hueco sin nombre. La
//! reforma lo hace **irrepresentable** — CHECK en la base + resolución en toda vía de escritura —
//! y para eso necesita un destino garantizado por instalación y scope.
//!
//! Esta suite fija ese destino: nace con la instalación, es único, no se puede desmarcar ni
//! borrar, se mueve designando otro, y solo existe en `income`/`expense`.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn categories(app: &TestApp, owner: &LoggedInOwner) -> Vec<Value> {
    let r = app.get_with_cookie("/v1/categories", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    r.json().as_array().unwrap().clone()
}

fn fallback_of<'a>(cats: &'a [Value], scope: &str) -> &'a Value {
    let mut found: Vec<&Value> = cats
        .iter()
        .filter(|c| c["scope"] == scope && c["is_fallback"] == json!(true))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "se esperaba exactamente una categoría por defecto en '{scope}': {found:?}"
    );
    found.pop().unwrap()
}

// ---------------------------------------------------------------------------
// El invariante de arranque
// ---------------------------------------------------------------------------

/// Una instalación recién creada tiene **una** categoría por defecto de ingreso y **una** de
/// gasto, y ninguna en los otros dos scopes. Es lo que hace que las escrituras nunca se queden
/// sin destino; sin esto, el primer import sin regla se llevaría un `fallback_category_missing`.
#[tokio::test]
async fn a_fresh_installation_has_exactly_one_fallback_per_ledger_scope() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cats = categories(&app, &owner).await;

    assert_eq!(fallback_of(&cats, "income")["name"], "Otros ingresos");
    assert_eq!(fallback_of(&cats, "expense")["name"], "Otros gastos");

    for scope in ["asset", "liability"] {
        let marked = cats
            .iter()
            .filter(|c| c["scope"] == scope && c["is_fallback"] == json!(true))
            .count();
        assert_eq!(marked, 0, "'{scope}' no puede tener categoría por defecto");
    }

    // El campo viaja en TODAS las filas, no solo en las marcadas: un cliente que no lo vea no
    // puede distinguir «no es la por defecto» de «este servidor no lo publica».
    for c in &cats {
        assert!(c["is_fallback"].is_boolean(), "is_fallback ausente en {c}");
    }
}

/// El filtro por scope publica el mismo campo (misma core, dos ramas de SQL: la que filtra y la
/// que no; una de las dos podría olvidarse la columna).
#[tokio::test]
async fn the_scope_filtered_listing_also_publishes_is_fallback() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .get_with_cookie("/v1/categories?scope=expense", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let cats = r.json().as_array().unwrap().clone();
    assert_eq!(fallback_of(&cats, "expense")["name"], "Otros gastos");
}

/// Crear una categoría nueva NO la designa por defecto (el POST no tiene ese eje).
#[tokio::test]
async fn a_newly_created_category_is_never_the_default_one() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .post_json_with_cookie(
            "/v1/categories",
            json!({"scope": "expense", "name": "Cañas"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(r.json()["is_fallback"], json!(false), "{}", r.json());
}

// ---------------------------------------------------------------------------
// El swap
// ---------------------------------------------------------------------------

/// Designar otra categoría por defecto **mueve** la marca en una sola transacción: la anterior se
/// desmarca y la nueva se marca. El orden importa —el índice único es parcial sobre
/// `(installation_id, scope) WHERE is_fallback`— y el invariante que verifica el test es el que
/// ese índice protege: nunca hay dos a la vez.
#[tokio::test]
async fn designating_another_category_moves_the_mark_atomically() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cats = categories(&app, &owner).await;
    let antigua = fallback_of(&cats, "expense")["id"].as_str().unwrap().to_string();
    let nueva = app.create_category(&owner, "expense", "Cajón nuevo").await;

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{nueva}"),
            json!({"is_fallback": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["is_fallback"], json!(true), "{}", r.json());

    let cats = categories(&app, &owner).await;
    assert_eq!(fallback_of(&cats, "expense")["id"], json!(nueva));
    let vieja = cats.iter().find(|c| c["id"] == json!(antigua)).unwrap();
    assert_eq!(vieja["is_fallback"], json!(false), "{vieja}");

    // Y la que era intocable ahora se puede borrar: el veto lo daba la marca, no la identidad.
    let d = app
        .delete_with_cookie(&format!("/v1/categories/{antigua}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "{d:?}");
}

/// Repetir la designación sobre la que YA es la por defecto es un no-op con 200, no un 23505: el
/// `UPDATE` que desmarca excluye la propia fila.
#[tokio::test]
async fn designating_the_current_default_again_is_a_no_op() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cats = categories(&app, &owner).await;
    let actual = fallback_of(&cats, "income")["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{actual}"),
            json!({"is_fallback": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["is_fallback"], json!(true), "{}", r.json());
    let cats = categories(&app, &owner).await;
    assert_eq!(fallback_of(&cats, "income")["id"], json!(actual));
}

/// Renombrar la categoría por defecto NO la degrada: la designación vive en la columna, no en el
/// nombre con el que se sembró.
#[tokio::test]
async fn renaming_the_default_category_keeps_the_mark() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cats = categories(&app, &owner).await;
    let id = fallback_of(&cats, "expense")["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{id}"),
            json!({"name": "Varios"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["name"], "Varios", "{b}");
    assert_eq!(b["is_fallback"], json!(true), "renombrar no degrada: {b}");
}

// ---------------------------------------------------------------------------
// Los rechazos
// ---------------------------------------------------------------------------

/// `is_fallback: false` no es una operación. Apagarla dejaría a la instalación sin destino y el
/// siguiente import moriría con un error que nadie relacionaría con este PATCH.
#[tokio::test]
async fn the_default_category_cannot_be_unset() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cats = categories(&app, &owner).await;
    let id = fallback_of(&cats, "expense")["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{id}"),
            json!({"is_fallback": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "fallback_cannot_be_unset", "{}", r.json());

    // Nada se ha movido.
    let cats = categories(&app, &owner).await;
    assert_eq!(fallback_of(&cats, "expense")["id"], json!(id));
}

/// Los activos y los pasivos llevan SIEMPRE categoría explícita: no hay cajón que designar, y el
/// CHECK de la base lo prohíbe. El 400 llega antes, con un código propio.
#[tokio::test]
async fn only_income_and_expense_categories_can_be_the_default_one() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    for scope in ["asset", "liability"] {
        let id = app.create_category(&owner, scope, "Cualquiera").await;
        let r = app
            .patch_json_with_cookie(
                &format!("/v1/categories/{id}"),
                json!({"is_fallback": true}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{scope}: {r:?}");
        assert_eq!(r.json()["code"], "fallback_scope_invalid", "{}", r.json());
    }
}

/// Borrar la categoría por defecto es imposible **aunque esté vacía**: la comprobación va ANTES de
/// contar referencias. Con `remap_to` tampoco — el destino del remap no sustituye a la
/// designación, y quedarse sin cajón rompe la siguiente escritura, no ésta.
#[tokio::test]
async fn the_default_category_cannot_be_deleted_even_when_unused() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cats = categories(&app, &owner).await;
    let fb = fallback_of(&cats, "expense")["id"].as_str().unwrap().to_string();
    let otra = app.create_category(&owner, "expense", "Compras").await;

    for uri in [
        format!("/v1/categories/{fb}"),
        format!("/v1/categories/{fb}?remap_to={otra}"),
    ] {
        let r = app.delete_with_cookie(&uri, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{uri}: {r:?}");
        assert_eq!(r.json()["code"], "category_is_fallback", "{}", r.json());
    }

    // Sigue ahí.
    let cats = categories(&app, &owner).await;
    assert_eq!(fallback_of(&cats, "expense")["id"], json!(fb));
}

/// El PATCH vacío sigue siendo 400 (el campo nuevo no lo convierte en «algo que actualizar» por el
/// mero hecho de existir en el body).
#[tokio::test]
async fn an_empty_patch_is_still_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = app.create_category(&owner, "expense", "Compras").await;
    let r = app
        .patch_json_with_cookie(&format!("/v1/categories/{id}"), json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty", "{}", r.json());
}

// ---------------------------------------------------------------------------
// Las guardas de la base
// ---------------------------------------------------------------------------

/// El índice único PARCIAL es el que sostiene «una y solo una». Se prueba por SQL a propósito:
/// la API nunca deja llegar dos marcadas, y justamente por eso el día que un camino nuevo se
/// olvide del swap la base tiene que ser la que diga que no.
#[tokio::test]
async fn the_database_refuses_a_second_default_category_in_the_same_scope() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let otra = app.create_category(&owner, "expense", "Compras").await;

    let err = sqlx::query("UPDATE categories SET is_fallback = true WHERE id = $1::uuid")
        .bind(uuid::Uuid::parse_str(&otra).unwrap())
        .execute(&app.pool)
        .await
        .expect_err("dos categorías por defecto en el mismo scope deben ser imposibles");
    let db = err.as_database_error().expect("error de base");
    assert_eq!(db.code().as_deref(), Some("23505"), "{err}");
}

/// El CHECK de `transactions` es real: un gasto sin categoría no se puede escribir ni saltándose
/// la API. Es la red que hace que «sin categoría» sea IRREPRESENTABLE y no solo «desaconsejado».
#[tokio::test]
async fn the_database_refuses_an_expense_without_a_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let iid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .expect("instalación");

    let err = sqlx::query(
        r#"INSERT INTO transactions
               (installation_id, owner_user_id, source, op_date, concept, amount, kind,
                category_id, fingerprint)
           VALUES ($1, $2, 'manual', DATE '2026-06-10', 'SIN CATEGORIA', -10, 'expense',
                   NULL, 'fp-sin-categoria')"#,
    )
    .bind(iid)
    .bind(owner.user_id)
    .execute(&app.pool)
    .await
    .expect_err("un gasto sin categoría debe violar el CHECK");
    let db = err.as_database_error().expect("error de base");
    assert_eq!(db.code().as_deref(), Some("23514"), "{err}");

    // La misma fila SIN clase sí entra: «sin clasificar» sigue existiendo a propósito.
    let ok = sqlx::query(
        r#"INSERT INTO transactions
               (installation_id, owner_user_id, source, op_date, concept, amount, kind,
                category_id, fingerprint)
           VALUES ($1, $2, 'manual', DATE '2026-06-10', 'SIN CLASIFICAR', -10, NULL,
                   NULL, 'fp-sin-clasificar')"#,
    )
    .bind(iid)
    .bind(owner.user_id)
    .execute(&app.pool)
    .await;
    assert!(ok.is_ok(), "una fila sin kind es legítima: {ok:?}");
}
