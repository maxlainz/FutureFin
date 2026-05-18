//! Fase 3.2 — Tras unificar la detección en `impl From<sqlx::Error> for ApiError`, todas las
//! violaciones de UNIQUE (`23505`) deben convertirse a 409 Conflict sin que cada handler tenga
//! su propio mapeador ad-hoc.

mod common;

use common::TestApp;

#[tokio::test]
async fn duplicate_username_registration_returns_409() {
    let app = TestApp::spawn().await;
    let body = serde_json::json!({
        "username": "alice",
        "password": "correct horse battery staple",
        "birth_date": "1990-01-01",
    });

    let first = app.post_json("/v1/auth/register", body.clone()).await;
    assert_eq!(first.status, http::StatusCode::CREATED);

    let second = app.post_json("/v1/auth/register", body).await;
    assert_eq!(second.status, http::StatusCode::CONFLICT, "{second:?}");
}

#[tokio::test]
async fn duplicate_category_name_in_same_scope_returns_409() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bob").await;

    let first = app
        .post_json_with_cookie(
            "/v1/categories",
            serde_json::json!({"scope": "asset", "name": "Bolsa"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(first.status, http::StatusCode::CREATED);

    let dup = app
        .post_json_with_cookie(
            "/v1/categories",
            serde_json::json!({"scope": "asset", "name": "Bolsa"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        dup.status,
        http::StatusCode::CONFLICT,
        "duplicar nombre dentro del mismo scope debe ser 409, recibido {}",
        dup.status
    );

    // El mismo nombre en otro scope SÍ es válido (la constraint es por (installation, scope, name)).
    let other_scope = app
        .post_json_with_cookie(
            "/v1/categories",
            serde_json::json!({"scope": "liability", "name": "Bolsa"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(other_scope.status, http::StatusCode::CREATED);
}
