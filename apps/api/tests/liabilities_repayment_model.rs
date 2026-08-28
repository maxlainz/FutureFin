//! `repayment_model` de los pasivos (4.2.0): default, dominio del enum y las cuatro validaciones
//! de coherencia modelo ↔ TIN ↔ plan de pago.
//!
//! Lo que estos tests protegen es sobre todo el **silencio**: un `french` sin TIN, sin cuota o
//! con frecuencia semanal se comportaría exactamente como el modelo histórico y el usuario
//! creería estar viendo intereses que nadie está cobrando. Por eso la API los rechaza en vez de
//! guardarlos.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::{LoggedInOwner, ResponseParts, TestApp};
use serde_json::json;

/// Crea las dos categorías que exige un pasivo y devuelve `(liability_cat, expense_cat)`.
async fn categories(app: &TestApp, owner: &LoggedInOwner) -> (String, String) {
    (
        app.create_category(owner, "liability", "Préstamos").await,
        app.create_category(owner, "expense", "Cuotas").await,
    )
}

/// POST `/v1/liabilities` con los campos obligatorios ya puestos; `extra` se mezcla encima.
async fn post_liability(
    app: &TestApp,
    owner: &LoggedInOwner,
    cat: &str,
    exp_cat: &str,
    extra: serde_json::Value,
) -> ResponseParts {
    let mut body = json!({
        "category_id": cat,
        "expense_category_id": exp_cat,
        "label": "Hipoteca",
    });
    for (k, v) in extra.as_object().expect("extra must be an object") {
        body[k] = v.clone();
    }
    app.post_json_with_cookie("/v1/liabilities", body, &owner.cookie).await
}

/// Asserta un 400 cuyo `message` empieza por el código estable esperado.
fn assert_bad_request_code(r: &ResponseParts, code: &str) {
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "esperaba 400 {code}: {r:?}");
    let body = r.json();
    let msg = body["message"].as_str().unwrap_or_default();
    assert!(msg.starts_with(code), "esperaba el código {code}, llegó: {msg}");
}

/// Un pasivo creado sin mencionar el campo es `fixed_payments`: el modelo histórico. Es la
/// garantía de que 4.2.0 no mueve un solo número de una instalación existente.
#[tokio::test]
async fn create_without_the_field_defaults_to_fixed_payments() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(&app, &owner, &cat, &exp_cat, json!({ "principal": "100000" })).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(r.json()["repayment_model"], "fixed_payments");

    // Y viaja igual en el listado (no solo en la respuesta del create).
    let listed = app.get_with_cookie("/v1/liabilities", &owner.cookie).await;
    assert_eq!(listed.status, http::StatusCode::OK);
    assert_eq!(listed.json()[0]["repayment_model"], "fixed_payments");
}

/// El dominio del enum lo cierra **serde**, no una validación nuestra: un literal desconocido en
/// el body no llega ni al handler. 422, igual que `payment_frequency` o `fire_number_mode`.
#[tokio::test]
async fn unknown_repayment_model_literal_is_422() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "principal": "100000", "repayment_model": "aleman" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::UNPROCESSABLE_ENTITY, "{r:?}");
}

/// Sin plan de pago no hay ni interés ni amortización: el engine exige plan activo para devengar,
/// así que un `french` sin cuota sería un `fixed_payments` disfrazado.
#[tokio::test]
async fn model_without_payment_plan_is_400_payment_plan_required() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "principal": "100000", "repayment_model": "french", "apr_percent": "3" }),
    )
    .await;
    assert_bad_request_code(&r, "payment_plan_required_for_model");
}

/// `french` exige TIN > 0. Un TIN **explícitamente 0** es el caso peligroso: el engine degeneraría
/// en `fixed_payments` y el usuario tendría un «francés» que no cobra un céntimo de interés.
#[tokio::test]
async fn french_without_apr_or_with_zero_apr_is_400_apr_required() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let plan = json!({
        "principal": "100000",
        "repayment_model": "french",
        "payment_amount": "500",
        "payment_frequency": "monthly",
    });

    let r = post_liability(&app, &owner, &cat, &exp_cat, plan.clone()).await;
    assert_bad_request_code(&r, "apr_required_for_model");

    let mut with_zero = plan;
    with_zero["apr_percent"] = json!("0");
    let r = post_liability(&app, &owner, &cat, &exp_cat, with_zero).await;
    assert_bad_request_code(&r, "apr_required_for_model");
}

/// La recurrencia del engine es MENSUAL. Con `weekly` el handler convierte la cuota a su
/// equivalente mensual (×52/12), lo que en un modelo sin intereses es exacto pero en uno que
/// devenga cambiaría el devengo.
#[tokio::test]
async fn weekly_frequency_with_a_non_fixed_model_is_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "apr_percent": "3",
            "payment_amount": "120",
            "payment_frequency": "weekly",
        }),
    )
    .await;
    assert_bad_request_code(&r, "weekly_not_supported_for_model");
}

/// Derivar el principal del plan solo tiene inversa cerrada en `fixed_payments` (Σ cuotas) y
/// `french` (valor actual). En `interest_only` la cuota no amortiza y en `revolving` el plan no
/// describe un calendario cerrado.
#[tokio::test]
async fn derive_with_interest_only_or_revolving_is_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    for model in ["interest_only", "revolving"] {
        let r = post_liability(
            &app,
            &owner,
            &cat,
            &exp_cat,
            json!({
                "repayment_model": model,
                "apr_percent": "3",
                "derive_principal_from_plan": true,
                "payment_amount": "500",
                "payment_frequency": "monthly",
                "payment_end_date": "2040-01-01",
            }),
        )
        .await;
        assert_bad_request_code(&r, "derive_not_supported_for_model");
    }
}

/// La validación del PATCH mira el estado **resultante**, no el body. Aquí el body solo lleva el
/// modelo; la cuota viene de la fila y el TIN falta en ambos → 400. Con el criterio ingenuo
/// («solo valido lo que llega») esto pasaría y guardaría un francés sin TIN.
#[tokio::test]
async fn patch_validates_the_merged_state_not_just_the_body() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let created = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "payment_amount": "500",
            "payment_frequency": "monthly",
        }),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            json!({ "repayment_model": "french" }),
            &owner.cookie,
        )
        .await;
    assert_bad_request_code(&r, "apr_required_for_model");

    // El mismo PATCH, ahora aportando el TIN que faltaba, sí pasa.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            json!({ "repayment_model": "french", "apr_percent": "3" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["repayment_model"], "french");
}

/// Volver al modelo histórico es un PATCH normal: la columna es NOT NULL, así que «deshacer» es
/// mandar `fixed_payments` explícito.
#[tokio::test]
async fn patch_can_switch_the_model_back_to_fixed_payments() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let created = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "apr_percent": "3",
            "payment_amount": "500",
            "payment_frequency": "monthly",
        }),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    assert_eq!(created.json()["repayment_model"], "french");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            json!({ "repayment_model": "fixed_payments" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["repayment_model"], "fixed_payments");
}

/// `fixed_payments` NO impone nada: un TIN configurado es informativo (el engine lo ignora en
/// ese modelo) y `weekly` sigue siendo válido. Es el contrato de compatibilidad: ningún pasivo
/// que se pudiera crear antes de 4.2.0 empieza a dar 400.
#[tokio::test]
async fn fixed_payments_accepts_apr_and_weekly_without_complaining() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "fixed_payments",
            "apr_percent": "3",
            "payment_amount": "120",
            "payment_frequency": "weekly",
        }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(r.json()["repayment_model"], "fixed_payments");
}

/// Última línea de defensa: el CHECK de la columna. Aunque alguien escriba en la tabla saltándose
/// la API, un literal fuera del dominio no entra.
#[tokio::test]
async fn the_database_check_rejects_a_bogus_model_written_outside_the_api() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let created =
        post_liability(&app, &owner, &cat, &exp_cat, json!({ "principal": "100000" })).await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = uuid::Uuid::parse_str(created.json()["id"].as_str().unwrap()).unwrap();

    let err = sqlx::query("UPDATE liabilities SET repayment_model = 'aleman' WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .expect_err("el CHECK debe rechazar un modelo fuera del dominio");
    assert!(
        format!("{err}").contains("liabilities_repayment_model_chk"),
        "esperaba la violación del CHECK, llegó: {err}"
    );
}

// ---------------------------------------------------------------------------
// Validación agrupada: todo lo que falta, en un solo viaje (Fase 2, issue #83)
// ---------------------------------------------------------------------------

/// El caso del issue: `french` sin plan **y** sin TIN. Hasta 4.3.1 el servidor devolvía
/// `payment_plan_required_for_model`, el cliente añadía la cuota, y en el SEGUNDO viaje aparecía
/// `apr_required_for_model`. Tres turnos para un alta cuyas tres condiciones el servidor conocía
/// desde la primera llamada — y cada rebote es una oportunidad de que un agente se invente un TIN
/// plausible para desatascarse, con la amortización entera colgando de ese número.
///
/// Ahora sale un único 400 `repayment_model_state_invalid` que **enumera las dos** exigencias.
#[tokio::test]
async fn french_missing_plan_and_apr_reports_both_at_once() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "principal": "100000", "repayment_model": "french" }),
    )
    .await;
    assert_bad_request_code(&r, "repayment_model_state_invalid");

    let msg = r.json()["message"].as_str().unwrap().to_string();
    assert!(
        msg.contains("payment_amount and payment_frequency are required"),
        "el mensaje debe nombrar el plan que falta: {msg}"
    );
    assert!(
        msg.contains("apr_percent > 0 is required"),
        "el mensaje debe nombrar el TIN que falta: {msg}"
    );

    // Y con las DOS cosas aportadas de una vez, el alta pasa: el mensaje era la receta completa,
    // no un primer paso. Esto es lo que convierte tres viajes en dos.
    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "payment_amount": "500",
            "payment_frequency": "monthly",
            "apr_percent": "3",
        }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(r.json()["repayment_model"], "french");
}

/// El peor caso previo: `revolving` con `derive_principal_from_plan` y sin nada más. Cuatro
/// condiciones, cuatro viajes. Aquí el plan falta (1) y el TIN falta (2) — `derive` no llega a
/// contar porque `revolving` lo prohíbe siempre (3). Las TRES en la misma respuesta.
#[tokio::test]
async fn revolving_with_derive_reports_every_problem_at_once() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "repayment_model": "revolving", "derive_principal_from_plan": true }),
    )
    .await;
    assert_bad_request_code(&r, "repayment_model_state_invalid");

    let msg = r.json()["message"].as_str().unwrap().to_string();
    for esperado in [
        "payment_amount and payment_frequency are required",
        "apr_percent > 0 is required",
        "derive_principal_from_plan must be false",
    ] {
        assert!(msg.contains(esperado), "falta «{esperado}» en: {msg}");
    }
}

/// El PATCH agrupa igual que el POST, y sobre el estado **resultante**: la fila de partida es un
/// `fixed_payments` pelado (sin plan ni TIN) y el body solo cambia el modelo.
#[tokio::test]
async fn patch_reports_every_problem_of_the_merged_state_at_once() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    let created = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "principal": "100000" }),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            json!({ "repayment_model": "french" }),
            &owner.cookie,
        )
        .await;
    assert_bad_request_code(&r, "repayment_model_state_invalid");
    let msg = r.json()["message"].as_str().unwrap().to_string();
    assert!(msg.contains("payment_amount and payment_frequency are required"), "{msg}");
    assert!(msg.contains("apr_percent > 0 is required"), "{msg}");
}

/// La contrapartida del agrupado: con UN solo problema sigue saliendo el código específico de
/// siempre. Es lo que hace que el cambio no rompa el fixture `error-codes.json`, las frases de
/// `errorMessages.ts` ni los tests de arriba — y es también el código más accionable cuando de
/// verdad solo falta una cosa. Aquí se fija el contrato explícitamente, para que nadie lo
/// «simplifique» a un único código agregado sin darse cuenta de lo que cuesta.
#[tokio::test]
async fn a_single_problem_still_yields_its_specific_code() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    // Solo falta el TIN.
    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "payment_amount": "500",
            "payment_frequency": "monthly",
        }),
    )
    .await;
    assert_bad_request_code(&r, "apr_required_for_model");

    // Solo sobra la frecuencia semanal.
    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "apr_percent": "3",
            "payment_amount": "120",
            "payment_frequency": "weekly",
        }),
    )
    .await;
    assert_bad_request_code(&r, "weekly_not_supported_for_model");
}
