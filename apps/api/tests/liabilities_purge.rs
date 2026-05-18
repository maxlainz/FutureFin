//! Fase 1.1 — La purga de pasivos vencidos debe salir de los GETs.
//!
//! Contrato esperado tras el cambio:
//! - `GET /v1/liabilities` filtra por `payment_end_date IS NULL OR >= today`, pero no borra filas.
//! - `GET /v1/summary` no incluye principales de pasivos vencidos en `total_liabilities`
//!   ni en los breakdowns de categoría/type_tag.
//! - Las filas vencidas siguen existiendo en BD (no hay mutación en lecturas).

mod common;

use chrono::{Duration, Utc};
use common::TestApp;

#[tokio::test]
async fn expired_liability_is_hidden_from_listing_but_persists_in_db() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_id = app.create_category(&owner, "liability", "Préstamo").await;

    let today = Utc::now().date_naive();
    let past = (today - Duration::days(5)).format("%Y-%m-%d").to_string();
    let future = (today + Duration::days(30)).format("%Y-%m-%d").to_string();

    // Pasivo vencido (con plan de pago acabado).
    let expired = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_id,
                "label": "Tarjeta cerrada",
                "principal": "1000",
                "payment_amount": "100",
                "payment_frequency": "monthly",
                "payment_end_date": past,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        expired.status,
        http::StatusCode::CREATED,
        "expired liability create failed: {expired:?}"
    );

    // Pasivo activo (plan futuro).
    let active = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_id,
                "label": "Hipoteca",
                "principal": "50000",
                "payment_amount": "300",
                "payment_frequency": "monthly",
                "payment_end_date": future,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        active.status,
        http::StatusCode::CREATED,
        "active liability create failed: {active:?}"
    );

    // Antes de listar: ambos existen en BD.
    assert_eq!(app.count_rows("liabilities").await, 2);

    let list = app.get_with_cookie("/v1/liabilities", &owner.cookie).await;
    assert_eq!(list.status, http::StatusCode::OK);
    let body = list.json();
    let arr = body.as_array().expect("liabilities list is an array");
    assert_eq!(arr.len(), 1, "solo el pasivo activo debe aparecer en la lista");
    assert_eq!(arr[0]["label"], "Hipoteca");

    // ⚠️ Tras la Fase 1.1, las filas vencidas deben PERSISTIR. Hoy el handler las purga.
    let remaining = app.count_rows("liabilities").await;
    assert_eq!(
        remaining, 2,
        "GET /v1/liabilities no debe borrar filas vencidas (hoy fallará: el handler purga)"
    );
}

#[tokio::test]
async fn summary_excludes_expired_liability_principal_and_breakdown() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bob").await;
    let cat_id = app.create_category(&owner, "liability", "Préstamo").await;

    let today = Utc::now().date_naive();
    let past = (today - Duration::days(2)).format("%Y-%m-%d").to_string();
    let future = (today + Duration::days(60)).format("%Y-%m-%d").to_string();

    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": cat_id,
            "label": "Vencido",
            "principal": "9999",
            "payment_amount": "100",
            "payment_frequency": "monthly",
            "payment_end_date": past,
        }),
        &owner.cookie,
    )
    .await;
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": cat_id,
            "label": "Activo",
            "principal": "100",
            "payment_amount": "10",
            "payment_frequency": "monthly",
            "payment_end_date": future,
        }),
        &owner.cookie,
    )
    .await;

    let summary = app.get_with_cookie("/v1/summary", &owner.cookie).await;
    assert_eq!(summary.status, http::StatusCode::OK);
    let body = summary.json();

    let total: f64 = body["total_liabilities"]
        .as_str()
        .expect("total_liabilities is string")
        .parse()
        .expect("parse decimal");
    assert!(
        (total - 100.0).abs() < 0.001,
        "total_liabilities solo debe sumar el pasivo activo (esperado ≈ 100, recibido {total})"
    );

    let breakdown = body["liabilities_by_category"]
        .as_array()
        .expect("liabilities_by_category es array");
    let prestamo_total: f64 = breakdown
        .iter()
        .find(|row| row["category_name"] == "Préstamo")
        .and_then(|row| row["total"].as_str())
        .expect("Préstamo en breakdown")
        .parse()
        .expect("parse breakdown total");
    assert!(
        (prestamo_total - 100.0).abs() < 0.001,
        "breakdown por categoría no debe incluir el principal vencido (esperado ≈ 100, recibido {prestamo_total})"
    );

    assert_eq!(
        app.count_rows("liabilities").await,
        2,
        "GET /v1/summary no debe borrar filas vencidas"
    );
}
