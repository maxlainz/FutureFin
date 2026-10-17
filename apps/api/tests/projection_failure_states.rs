//! #119 (Ola 2): los estados de FALLO que el motor ya calculaba llegan por fin al wire HTTP —
//! mes de agotamiento, déficit descubierto y amortización negativa por pasivo. Norma de la casa:
//! NULL nunca es cero; al lado viaja el porqué. (Los estados de fallo del PLAN —número FIRE
//! ausente, pensión sin fecha, umbral inalcanzable— se retiraron con él en 6.0.0, D-7.)
//! Números predichos A MANO en cada doc-comment antes de ejecutar.

mod common;
use common::TestApp;
use serde_json::{json, Value};

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba string decimal, llegó {v:?}"))
        .parse::<f64>()
        .expect("decimal")
}

/// T1 · A mano: 200.000 € líquidos al 0 %; gasto 2.000 €/mes sin ingresos. La cartera se VACÍA
/// en el mes 100 del BUCLE
/// (200.000/2.000, caso exacto, predicado >=) = **mes 99 de la rejilla publicada** desde 5.0.0
/// (#210); el descubierto empieza al siguiente y acumula (360−100)×2.000 = 520.000 ⇒
/// NW(360) = −520.000 — la aritmética del descubierto es la del BUCLE y no se mueve. Control:
/// con 96 meses no se agota (200.000 − 192.000 = 8.000 > 0) ⇒ null = «no en el horizonte», no
/// «no calculado».
#[tokio::test]
async fn portfolio_depletion_month_is_published_and_exact() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("t1x").await;
    let cat_a = app.create_category(&owner, "asset", "Cash").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Cuenta", "current_value": "200000", "is_liquid": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": cat_e, "amount": "2000", "ends_at_retirement": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxes_enabled": false}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let series = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();
    // 6.0.0 (D-7): la serie ya no publica plan de jubilación; la cartera se vacía por el GASTO
    // REGULAR —2.000 €/mes—, y el mes de agotamiento no se mueve ni un día.
    assert!(series.get("jubilacion_month_index").is_none(), "{series}");
    // #210 — **99, no 100**: el motor agota la cartera en su mes 100 (1-based del bucle) y desde
    // 5.0.0 el handler publica ese hecho en la MISMA rejilla 0-based que `points[].month_index` y
    // `engine_month_to_grid` (k − 1). El mes civil no se ha
    // movido ni un día: es el mismo mes, nombrado con la convención del resto de la respuesta.
    // Hasta 4.15.x este pin era 100 y era el único índice de la respuesta desplazado.
    assert_eq!(series["assets_depleted_month_index"], 99, "{series}");
    assert_eq!(dec(&series["uncovered_deficit_total"]), 520_000.0, "{series}");
    let last_nw = series["points"].as_array().unwrap().last().unwrap()["net_worth"]
        .as_f64()
        .unwrap();
    assert!((last_nw + 520_000.0).abs() < 0.01, "NW(360) = {last_nw}");

    // Control del null: en 96 meses no llega a agotarse.
    let series = app
        .get_with_cookie("/v1/projection/series?months=96", &owner.cookie)
        .await
        .json();
    assert!(series["assets_depleted_month_index"].is_null(), "{series}");
    assert_eq!(dec(&series["uncovered_deficit_total"]), 0.0, "{series}");
}

/// T2 · A mano: francés 200.000 € al TIN 6 % (i = 0,005), cuota 800 < interés mes 1 (1.000) ⇒
/// P₁ = 200.000×1,005 − 800 = 200.200,00 exacto y la deuda CRECE sin tope. El campo nuevo lo
/// declara con opening/final; el control es un `interest_only` (principal CONGELADO, no crece)
/// que NO debe aparecer — esa distinción con `payment_does_not_reduce_principal` es el valor.
#[tokio::test]
async fn negative_amortization_is_published_per_liability() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("t2x").await;
    let cat_l = app.create_category(&owner, "liability", "Prestamos").await;
    let cat_e = app.create_category(&owner, "expense", "Cuotas").await;
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({"category_id": cat_l, "expense_category_id": cat_e, "label": "Crece",
                   "principal": "200000", "apr_percent": "6", "payment_amount": "800",
                   "payment_frequency": "monthly", "repayment_model": "french"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let growing_id = r.json()["id"].as_str().unwrap().to_string();
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({"category_id": cat_l, "expense_category_id": cat_e, "label": "Congelado",
                   "principal": "80000", "apr_percent": "6", "payment_amount": "400",
                   "payment_frequency": "monthly", "repayment_model": "interest_only"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let series = app
        .get_with_cookie("/v1/projection/series?months=120", &owner.cookie)
        .await
        .json();
    let neg = series["liabilities_negative_amortization"].as_array().unwrap();
    assert_eq!(neg.len(), 1, "solo la que CRECE: {series}");
    assert_eq!(neg[0]["liability_id"], growing_id.as_str(), "{neg:?}");
    assert_eq!(dec(&neg[0]["opening_principal"]), 200_000.0);
    assert!(
        dec(&neg[0]["final_principal"]) > 200_000.0,
        "la deuda debe crecer: {neg:?}"
    );

    // Y el cuadro publica el mes 1 exacto: cierre 200.200,00, amortización −200,00.
    let sched = app
        .get_with_cookie(&format!("/v1/liabilities/{growing_id}/schedule"), &owner.cookie)
        .await
        .json();
    let m1 = &sched["months"].as_array().unwrap()[0];
    assert_eq!(dec(&m1["closing_principal"]), 200_200.0, "{m1}");
    assert_eq!(dec(&m1["principal_repaid"]), -200.0, "{m1}");
}
