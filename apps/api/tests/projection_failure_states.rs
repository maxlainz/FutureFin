//! #119 (Ola 2): los estados de FALLO que el motor ya calculaba llegan por fin al wire HTTP —
//! mes de agotamiento, déficit descubierto, amortización negativa por pasivo y la razón de un
//! objetivo FIRE ausente. Norma de la casa: NULL nunca es cero; al lado viaja el porqué.
//! Números predichos A MANO en cada doc-comment antes de ejecutar.

mod common;
use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

const PROTOCOL: &str = "2026-07-28";

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba string decimal, llegó {v:?}"))
        .parse::<f64>()
        .expect("decimal")
}

async fn mcp_post(app: &TestApp, bearer: &str, body: Value) -> Value {
    let mut builder = http::Request::builder()
        .method(http::Method::POST)
        .uri("/mcp")
        .header(http::header::HOST, "futurefin.test")
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL)
        .header(http::header::AUTHORIZATION, format!("Bearer {bearer}"));
    if let Some(method) = body["method"].as_str() {
        builder = builder.header("Mcp-Method", method);
    }
    if let Some(name) = body["params"]["name"].as_str() {
        builder = builder.header("Mcp-Name", name);
    }
    let resp = app
        .request(
            builder
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .expect("build MCP request"),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "MCP POST falló: {resp:?}");
    let text = String::from_utf8(resp.body.clone()).expect("utf8");
    let content_type = resp
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.starts_with("application/json") {
        return serde_json::from_str(&text).expect("json");
    }
    let mut last = None;
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                last = Some(v);
            }
        }
    }
    last.unwrap_or_else(|| panic!("sin frame JSON en la respuesta SSE:\n{text}"))
}

fn tool_call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {},
            }
        }
    })
}

async fn create_token(app: &TestApp, owner: &LoggedInOwner) -> String {
    let created = app
        .post_json_with_cookie("/v1/api-tokens", json!({"label": "issue 119"}), &owner.cookie)
        .await;
    created.json()["token"].as_str().unwrap().to_string()
}

fn tool_json(envelope: &Value) -> Value {
    serde_json::from_str(envelope["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// T1 · A mano: 200.000 € líquidos al 0 %; gasto 2.000 €/mes en AMBAS fases (la retirada no es
/// pegajosa: sin gasto regular el hogar «volvería a trabajar» en el mes 2); target manual
/// 8.000/0,04 = 200.000 ⇒ jubilado desde el mes 0. La cartera se VACÍA en el mes
/// 200.000/2.000 = 100 (caso exacto, predicado >=); el descubierto empieza en el 101 y acumula
/// (360−100)×2.000 = 520.000 ⇒ NW(360) = −520.000. Control: con 96 meses no se agota
/// (200.000 − 192.000 = 8.000 > 0) ⇒ null = «no en el horizonte», no «no calculado».
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
    // 5.0.0 (D13): el modo del objetivo, el importe manual y el SWR son del PERFIL del usuario;
    // los impuestos siguen siendo del hogar. Dos PATCHes, los mismos cuatro números.
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxes_enabled": false}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "manual", "fire_number_manual_amount": "8000", "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let series = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();
    assert_eq!(series["jubilacion_month_index"], 0, "{series}");
    assert_eq!(series["assets_depleted_month_index"], 100, "{series}");
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

/// T3 · `swr_pct = "0"` es escritura válida («jamás») pero anulaba el objetivo SIN explicación
/// en HTTP. Ahora la razón viaja con el mismo literal que simulate_projection publica — paridad
/// por construcción (mismo campo, misma función).
#[tokio::test]
async fn fire_target_absent_reason_reaches_http_with_swr_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("t3x").await;
    let cat_a = app.create_category(&owner, "asset", "Cash").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Cuenta", "current_value": "10000", "is_liquid": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    // Con gasto > 0 la necesidad es positiva: la razón que queda es EXACTAMENTE el SWR
    // (compute_fire_target_nw evalúa la necesidad antes que el SWR).
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": cat_e, "amount": "1500", "ends_at_retirement": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"swr_pct": "0"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let series = app.get_with_cookie("/v1/projection/series", &owner.cookie).await.json();
    assert!(series["jubilacion_target_net_worth"].is_null(), "{series}");
    assert_eq!(series["fire_target_absent_reason"], "swr_not_positive", "{series}");

    let token = create_token(&app, &owner).await;
    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    assert_eq!(
        sim["baseline"]["fire_target_absent_reason"], "swr_not_positive",
        "paridad HTTP↔MCP rota: {sim}"
    );
}

/// T4 · Las otras dos causas + el control de que el campo no es siempre no-nulo.
#[tokio::test]
async fn fire_target_absent_reason_covers_the_other_two_causes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("t4x").await;
    let cat_i = app.create_category(&owner, "income", "Pension").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;

    // (a) `manual_amount_missing` NO tiene camino vivo por la API:
    // `validate_retirement_profile` rechaza «manual sin importe» EN LA ESCRITURA
    // (fire_manual_amount_required) — el literal es la guardia defensiva del lado de cálculo. Se
    // pinea el rechazo en la puerta, que desde 5.0.0 es la del perfil.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "manual"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");

    // (b) annual_expense con ingreso de jubilación ≥ gasto de jubilación (2.000 ≥ 1.500).
    for (cat, body) in [
        (&cat_i, json!({"category_id": cat_i, "amount": "2000", "persists_after_retirement": true})),
        (&cat_e, json!({"category_id": cat_e, "amount": "1500", "ends_at_retirement": false})),
    ] {
        let _ = cat;
        let r = app
            .post_json_with_cookie("/v1/budget/entries", body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "annual_expense"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let series = app.get_with_cookie("/v1/projection/series", &owner.cookie).await.json();
    assert_eq!(series["fire_target_absent_reason"], "net_need_not_positive", "{series}");

    // (c) bien configurado ⇒ razón null Y objetivo presente (el campo no es siempre no-nulo).
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxes_enabled": false}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "manual", "fire_number_manual_amount": "500000", "swr_pct": "3.5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let series = app.get_with_cookie("/v1/projection/series", &owner.cookie).await.json();
    assert!(series["fire_target_absent_reason"].is_null(), "{series}");
    assert!(!series["jubilacion_target_net_worth"].is_null(), "{series}");
}
