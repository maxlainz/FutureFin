//! Fase 3.2 — Tras unificar la detección en `impl From<sqlx::Error> for ApiError`, todas las
//! violaciones de UNIQUE (`23505`) deben convertirse a 409 Conflict sin que cada handler tenga
//! su propio mapeador ad-hoc.
//!
//! 4.4.0 (Fase 1) — el mismo `impl` gana el brazo `22003` (numeric_value_out_of_range): las
//! columnas de dinero son NUMERIC(18,4), así que un importe absurdo las desbordaba en el INSERT
//! y salía como 500 «internal error» pelado. Era el único error de toda la superficie que un
//! cliente no podía clasificar — y justo el que dispara las políticas de retry-on-5xx contra
//! una entrada que jamás va a ser válida, metiendo a un agente desatendido en un bucle.

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

/// Un importe que desborda NUMERIC(18,4) es culpa de la ENTRADA, no del servidor: 400 con código
/// tipado, no 500. La regresión que protege es la del bucle de reintentos (ver cabecera).
#[tokio::test]
async fn absurd_amount_returns_400_amount_out_of_range_not_500() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("carol").await;

    let cat = app
        .post_json_with_cookie(
            "/v1/categories",
            serde_json::json!({"scope": "asset", "name": "Cuentas"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(cat.status, http::StatusCode::CREATED, "{cat:?}");
    let category_id = cat.json()["id"].as_str().expect("category id").to_string();

    // 27 dígitos: cabe en un rust_decimal::Decimal (por eso el parseo lo acepta) pero no en la
    // columna NUMERIC(18,4), que admite 14 enteros. El fallo ocurría en el INSERT, ya en la BD.
    let res = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "name": "desbordado",
                "category_id": category_id,
                "current_value": "999999999999999999999999999",
            }),
            &owner.cookie,
        )
        .await;

    assert_eq!(
        res.status,
        http::StatusCode::BAD_REQUEST,
        "un importe fuera de rango debe ser 400, no 500: {res:?}"
    );
    assert_eq!(
        res.json()["code"], "amount_out_of_range",
        "el código debe ser estable y accionable, no `bad_request` genérico: {res:?}"
    );
}

/// Una fecha absurda entraba tal cual en `upcoming_outflows_total` de `/v1/summary`: se validaba
/// el FORMATO, nunca el rango. Un flujo a ocho mil años vista movía una cifra de portada sin
/// aviso, y los modelos fallan con fechas relativas mucho más que con importes.
#[tokio::test]
async fn a_due_date_millennia_away_is_rejected_instead_of_polluting_the_summary() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("dave").await;

    let cat = app
        .post_json_with_cookie(
            "/v1/categories",
            serde_json::json!({"scope": "expense", "name": "Viajes"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(cat.status, http::StatusCode::CREATED, "{cat:?}");
    let category_id = cat.json()["id"].as_str().expect("category id").to_string();

    let res = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            serde_json::json!({
                "category_id": category_id,
                "title": "IRPF del año 9999",
                "expected_amount": "1000",
                "due_date": "9999-12-31",
            }),
            &owner.cookie,
        )
        .await;

    assert_eq!(res.status, http::StatusCode::BAD_REQUEST, "{res:?}");
    assert_eq!(res.json()["code"], "due_date_out_of_range", "{res:?}");

    // Y una fecha lejana pero razonable sigue entrando: la cota es generosa a propósito.
    let ok = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            serde_json::json!({
                "category_id": category_id,
                "title": "Herencia",
                "expected_amount": "1000",
                "due_date": "2070-01-01",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::CREATED, "{ok:?}");
}
