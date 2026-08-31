//! Principal derivado del plan de pago, por modelo de amortización (4.2.0).
//!
//! Dos contratos distintos conviviendo:
//!
//! - `fixed_payments` → `Σ cuotas` = `cuota × n`, **bit a bit igual** al comportamiento
//!   pre-4.2.0. Cualquier deriva aquí cambiaría el principal de pasivos ya existentes.
//! - `french` → **valor actual** de esas cuotas al TIN, que es el capital pendiente de verdad.
//!   200 cuotas de 500 € son 100.000 € de caja futura pero solo 78.618,15 € de deuda hoy al 3 %.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use chrono::{Months, NaiveDate};
use common::{LoggedInOwner, ResponseParts, TestApp};
use rust_decimal::Decimal;
use serde_json::json;

/// «Hoy» según el servidor (fecha civil de la instalación, `calendar_tz`), leída del ancla que ya
/// publica la serie histórica. Derivar el principal cuenta los pagos DESDE hoy, así que el test
/// no puede inventarse la fecha.
async fn server_today(app: &TestApp, owner: &LoggedInOwner) -> NaiveDate {
    let r = app.get_with_cookie("/v1/history/series", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    NaiveDate::parse_from_str(r.json()["anchor_date_ymd"].as_str().expect("anchor"), "%Y-%m-%d")
        .expect("anchor date")
}

/// Fecha fin que produce EXACTAMENTE `n` pagos mensuales contando desde `today` inclusive.
///
/// Se construye con la misma iteración («suma un mes al valor anterior») que
/// `payment_interval_count`, en vez de con un `today + (n-1) meses` de un tirón: con un `today`
/// que caiga en día 29/30/31 las dos formas NO coinciden (chrono clampa a fin de mes y la
/// iterativa arrastra el clamp), y el test daría 200 o 201 pagos según el día en que se ejecute.
fn end_date_for_n_monthly_payments(today: NaiveDate, n: u32) -> NaiveDate {
    // Anclado (#123): `hoy + (n−1) meses` en un solo paso. La versión encadenada producía,
    // con hoy en día 29-31, una fecha degradada que bajo el conteo anclado vale n−1 cuotas —
    // este helper habría hecho flaky-por-calendario todos los tests de derivación.
    today
        .checked_add_months(Months::new(n - 1))
        .expect("no overflow")
}

async fn categories(app: &TestApp, owner: &LoggedInOwner) -> (String, String) {
    (
        app.create_category(owner, "liability", "Préstamos").await,
        app.create_category(owner, "expense", "Cuotas").await,
    )
}

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
        "derive_principal_from_plan": true,
        "payment_amount": "500",
        "payment_frequency": "monthly",
    });
    for (k, v) in extra.as_object().expect("extra must be an object") {
        body[k] = v.clone();
    }
    app.post_json_with_cookie("/v1/liabilities", body, &owner.cookie).await
}

fn principal_of(r: &ResponseParts) -> Decimal {
    r.json()["principal"].as_str().expect("principal string").parse().expect("decimal")
}

/// El contrato histórico, intacto: en `fixed_payments` el principal derivado es la SUMA de las
/// cuotas, exacta, con TIN ausente o con TIN 0 — el modelo no descuenta nada. 500 × 200 = 100.000.
#[tokio::test]
async fn fixed_payments_derives_the_plain_sum_of_the_payments() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;
    let end = end_date_for_n_monthly_payments(server_today(&app, &owner).await, 200);

    let expected: Decimal = "100000".parse().unwrap();

    // Sin TIN.
    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "payment_end_date": end.to_string() }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(principal_of(&r), expected, "500 × 200 pagos");
    assert_eq!(r.json()["principal_derived_from_plan"], true);

    // Con TIN 0 explícito: mismo número, sin pasar por la exponencial.
    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "payment_end_date": end.to_string(), "apr_percent": "0" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(principal_of(&r), expected, "un TIN 0 no descuenta en fixed_payments");

    // INVERTIDO en 4.7.0 (#144): un TIN > 0 en el modelo sin intereses ya no es «informativo»
    // — es un alta inválida. El caso «fixed_payments ignora el TIN al derivar» murió con él:
    // la derivación es una sola rama (valor actual al TIN) y en este modelo el TIN no existe.
    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({ "payment_end_date": end.to_string(), "apr_percent": "3" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    let msg = r.json()["message"].as_str().unwrap_or_default().to_string();
    assert!(msg.starts_with("apr_forbidden_for_model"), "{msg}");
}

/// `french` descuenta: `P = M · (1 − (1 + i)^−n) / i` con `i = 3/1200 = 0,0025` y `n = 200`.
///
/// El valor exacto que devuelve el engine es `78618.15423035613958458543406`; redondeado a la
/// escala de money (4 decimales, `MidpointAwayFromZero`) queda **78618.1542**. Al 6 % la misma
/// renta vale **63120.2771**: el TIN participa de verdad, no es decorativo.
#[tokio::test]
async fn french_derives_the_present_value_of_the_payments() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;
    let end = end_date_for_n_monthly_payments(server_today(&app, &owner).await, 200);

    let r = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "payment_end_date": end.to_string(),
            "repayment_model": "french",
            "apr_percent": "3",
        }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(principal_of(&r), "78618.1542".parse::<Decimal>().unwrap());
    assert!(
        principal_of(&r) < "100000".parse::<Decimal>().unwrap(),
        "el capital pendiente descontado tiene que ser MENOR que la suma de las cuotas"
    );
}

/// Con el flag activo, cambiar SOLO el TIN re-deriva el principal. Es el caso de uso literal de
/// la tool MCP («el TIN de mi hipoteca ha subido»): si el principal no se recalculase, la deuda
/// declarada seguiría siendo la del tipo viejo.
#[tokio::test]
async fn patching_only_the_apr_rederives_the_principal() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;
    let end = end_date_for_n_monthly_payments(server_today(&app, &owner).await, 200);

    let created = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "payment_end_date": end.to_string(),
            "repayment_model": "french",
            "apr_percent": "3",
        }),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    assert_eq!(principal_of(&created), "78618.1542".parse::<Decimal>().unwrap());
    let id = created.json()["id"].as_str().unwrap().to_string();

    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            json!({ "apr_percent": "6" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    assert_eq!(principal_of(&patched), "63120.2771".parse::<Decimal>().unwrap());
}

/// Cambiar el MODELO con el flag activo también re-deriva: `french` → `fixed_payments` devuelve
/// el principal a la suma pelada de las cuotas. Esto obliga a que `new_model` se resuelva antes
/// del bloque de derivación, no después.
#[tokio::test]
async fn patching_the_model_rederives_the_principal() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;
    let end = end_date_for_n_monthly_payments(server_today(&app, &owner).await, 200);

    let created = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "payment_end_date": end.to_string(),
            "repayment_model": "french",
            "apr_percent": "3",
        }),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            // Volver al modelo histórico exige soltar el TIN en el mismo PATCH (#144).
            json!({ "repayment_model": "fixed_payments", "apr_percent": null }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    assert_eq!(patched.json()["repayment_model"], "fixed_payments");
    assert_eq!(
        principal_of(&patched),
        "100000".parse::<Decimal>().unwrap(),
        "al volver al modelo histórico el principal vuelve a ser Σ cuotas"
    );
}
