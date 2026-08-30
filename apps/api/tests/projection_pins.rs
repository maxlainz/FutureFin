//! PINS de regresión del programa de resolución (Olas 3-7): escenarios canónicos cuyos números
//! van a MOVERSE a propósito, una ola cada vez. Disciplina (plan aprobado 2026-08-30): capturar
//! ANTES de tocar el modelo, predecir a mano el nuevo valor en el cuerpo del PR, implementar, y
//! actualizar el pin con el número predicho citándolo en el CHANGELOG. Si el real no coincide
//! con el predicho, el diagnóstico está mal — no el pin.
//!
//! Valores marcados `a mano:` → derivados con la fórmula; `capturado 4.6.0:` → regresión
//! capturada del código actual (patrón projection_marker.rs), con tolerancia estrecha.

mod common;
use common::TestApp;
use serde_json::{json, Value};

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba string decimal, llegó {v:?}"))
        .parse::<f64>()
        .expect("decimal")
}

fn nw_at(series: &Value, month: u64) -> f64 {
    series["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["month_index"] == month)
        .unwrap_or_else(|| panic!("sin punto para el mes {month}"))["net_worth"]
        .as_f64()
        .unwrap()
}

/// Escenario A — «hipoteca viva en modo A» (lo moverán #144/#142/#124 en las Olas 3-4).
/// Activo 50.000 € líquido al 5 %; ingreso 3.000, gasto 1.200 (persiste en jubilación);
/// hipoteca `french` 150.000 € al TIN 3 %, cuota 800 €/mes, plan de 180 meses; SWR 3,5 %,
/// impuestos ON (escala ES por defecto), modo annual_expense, inflación 0.
#[tokio::test]
async fn pin_escenario_a_hipoteca_viva_modo_a() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("pina").await;
    let cat_a = app.create_category(&owner, "asset", "Fondos").await;
    let cat_i = app.create_category(&owner, "income", "Nomina").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let cat_l = app.create_category(&owner, "liability", "Hipoteca").await;
    let cat_le = app.create_category(&owner, "expense", "Cuota").await;

    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Indexado", "current_value": "50000",
                   "is_liquid": true, "expected_annual_return_percent": "5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let asset_id = r.json()["id"].as_str().unwrap().to_string();
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({"target_asset_id": asset_id, "kind": "remainder"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    for (path, body) in [
        ("/v1/budget/entries", json!({"category_id": cat_i, "amount": "3000"})),
        ("/v1/budget/entries", json!({"category_id": cat_e, "amount": "1200", "ends_at_retirement": false})),
        ("/v1/liabilities", json!({"category_id": cat_l, "expense_category_id": cat_le,
                                   "label": "Casa", "principal": "150000", "apr_percent": "3",
                                   "payment_amount": "800", "payment_frequency": "monthly",
                                   "repayment_model": "french",
                                   "payment_end_date": "2041-08-31"})),
    ] {
        let r = app.post_json_with_cookie(path, body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{path}: {r:?}");
    }
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"fire_number_mode": "annual_expense", "taxes_enabled": true,
                    "swr_pct": "3.5"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();

    // a mano: need = 1.200×12 = 14.400 (SIN la cuota — divergencia D17/#142, decidida: contará
    // los meses restantes del préstamo; este pin SE MOVERÁ en la Ola 4). Gross-up escala ES:
    // tramo 19 %: 14.400/0,81 = 17.777,78 > 6.000 → K = 1.140;
    // tramo 21 %: (14.400 + 1.140 − 0,21×6.000)/0,79 = 14.280/0,79 = 18.075,9494 ≤ 50.000 ✓.
    // target = 18.075,9494 / 0,035 = 516.455,6961.
    let target = dec(&s["jubilacion_target_net_worth"]);
    assert!((target - 516_455.6961).abs() < 0.01, "target: {target}");

    // capturado 4.6.0 (se moverán en las Olas 3-4 — #144 default french ya aplicado aquí a mano,
    // #142 descongelará la base del objetivo, #124 filtrará vencidas):
    let jub = s["jubilacion_month_index"].clone();
    let nw12 = nw_at(&s, 12);
    let nw180 = nw_at(&s, 180);
    let nw360 = nw_at(&s, 360);
    assert_eq!(jub, 235, "jubilacion_month_index capturado: {jub}");
    assert!((nw12 - (-80_006.71)).abs() < 0.01, "NW(12) capturado: {nw12}");
    assert!((nw180 - 316_313.32).abs() < 0.01, "NW(180) capturado: {nw180}");
    assert!((nw360 - 703_274.35).abs() < 0.01, "NW(360) capturado: {nw360}");
}

/// Escenario B — «inflación 2,5 %» (lo moverán #146/#139/#149 en la Ola 5).
/// Activo 20.000 € al 7 %; ingreso 2.500, gasto 1.500; SWR 4 %, sin impuestos; inflación 2,5 %.
#[tokio::test]
async fn pin_escenario_b_inflacion() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("pinb").await;
    let cat_a = app.create_category(&owner, "asset", "Fondos").await;
    let cat_i = app.create_category(&owner, "income", "Nomina").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Indexado", "current_value": "20000",
                   "is_liquid": true, "expected_annual_return_percent": "7"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let asset_id = r.json()["id"].as_str().unwrap().to_string();
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({"target_asset_id": asset_id, "kind": "remainder"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    for (path, body) in [
        ("/v1/budget/entries", json!({"category_id": cat_i, "amount": "2500"})),
        ("/v1/budget/entries", json!({"category_id": cat_e, "amount": "1500", "ends_at_retirement": false})),
    ] {
        let r = app.post_json_with_cookie(path, body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{path}: {r:?}");
    }
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"annual_inflation_assumption_percent": "2.5",
                   "fire_settings": {"fire_number_mode": "annual_expense", "taxes_enabled": false,
                    "swr_pct": "4"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();

    // a mano: base = 1.500×12/0,04 = 450.000. target(120) = 450.000×1,025^10 = 576.018,10
    // (1,025^10 = 1,2800845…). El gasto de la SIMULACIÓN va congelado (divergencia D1/#139,
    // decidida: gastos indexados, ingresos planos — este pin SE MOVERÁ en la Ola 5).
    let base = dec(&s["jubilacion_target_net_worth"]);
    assert!((base - 450_000.0).abs() < 0.01, "base: {base}");
    let ft120 = s["fire_target_series"].as_array().map(|a| a.len());
    assert!(ft120.unwrap_or(0) > 0, "fire_target_series vacío");

    // capturado 4.6.0 (ahorro plano 1.000/mes al 7 %):
    let jub = s["jubilacion_month_index"].clone();
    let nw120 = nw_at(&s, 120);
    let nw360 = nw_at(&s, 360);
    assert_eq!(jub, 285, "jubilacion_month_index capturado: {jub}");
    assert!((nw120 - 211_361.91).abs() < 0.01, "NW(120) capturado: {nw120}");
    assert!((nw360 - 1_094_275.23).abs() < 0.01, "NW(360) capturado: {nw360}");
}
