//! Fase 2.1 + 2.2 — `GET /v1/projection/series` debe seguir devolviendo los mismos números
//! después de envolver el motor en `spawn_blocking` y paralelizar las dos simulaciones con
//! `tokio::join!`. Sirve también como red de seguridad para Fase 1.5 (la serie FIRE).

mod common;

use common::TestApp;

/// Setup deterministico: 1 activo de 100k al 15% TAE, sin pasivos, sin planning, presupuesto
/// 100 € de ahorro neto al mes. El componente de mercado supera el ahorro desde el primer mes,
/// así que el marker «compound supera ahorro» debe activarse en `month_index = 1`.
#[tokio::test]
async fn projection_series_returns_stable_compound_marker_and_starting_nw() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Bolsa").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Alquiler").await;

    let asset_resp = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": "MSCI World",
                "current_value": "100000",
                "is_liquid": true,
                "expected_annual_return_percent": "15",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset_resp.status, http::StatusCode::CREATED, "create asset: {asset_resp:?}");
    let asset_id = asset_resp.json()["id"].as_str().unwrap().to_string();

    let income_resp = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            serde_json::json!({"category_id": income_cat, "amount": "1100"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(income_resp.status, http::StatusCode::CREATED, "create income entry: {income_resp:?}");

    let expense_resp = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            serde_json::json!({"category_id": expense_cat, "amount": "1000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(expense_resp.status, http::StatusCode::CREATED, "create expense entry: {expense_resp:?}");

    let rule_resp = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            serde_json::json!({"target_asset_id": asset_id, "kind": "remainder"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(rule_resp.status, http::StatusCode::CREATED, "create allocation rule: {rule_resp:?}");

    let series = app.get_with_cookie("/v1/projection/series?months=24", &owner.cookie).await;
    assert_eq!(series.status, http::StatusCode::OK, "GET series: {series:?}");
    let body = series.json();

    let starting_nw: f64 = body["starting_net_worth"].as_str().unwrap().parse().unwrap();
    assert!(
        (starting_nw - 100_000.0).abs() < 0.01,
        "starting_net_worth esperado 100000, recibido {starting_nw}"
    );

    let monthly_delta: f64 = body["monthly_delta_assumption"].as_str().unwrap().parse().unwrap();
    assert!(
        (monthly_delta - 100.0).abs() < 0.01,
        "monthly_delta_assumption esperado 100 (1100 - 1000), recibido {monthly_delta}"
    );

    let marker = body["compound_outpaces_true_savings_month_index"].as_u64();
    // Con 100k al 15% anual ≈ 1170 €/mes de mercado vs 100 €/mes de ahorro → 3 meses
    // consecutivos cumplidos desde el inicio, devuelve 1 (primer mes de la racha).
    assert_eq!(
        marker,
        Some(1),
        "compound_outpaces_true_savings_month_index esperado 1, recibido {marker:?}"
    );

    let points = body["points"].as_array().unwrap();
    assert_eq!(points.len(), 25, "horizon 24 meses → 25 puntos (índices 0..=24)");

    let nw_at_12: f64 = points[12]["net_worth"].as_f64().unwrap();
    // NW al mes 12 ≈ 100_000 * 1.15 + 100 * 12 ≈ 116_700 (sin compound de los aportes mensuales).
    // El motor sí compone, así que el valor exacto es ligeramente superior; tolerancia ancha.
    assert!(
        (115_000.0..120_000.0).contains(&nw_at_12),
        "NW al mes 12 fuera del rango razonable: {nw_at_12}"
    );
}
