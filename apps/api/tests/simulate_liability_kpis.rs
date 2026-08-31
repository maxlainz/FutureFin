//! KPIs de deuda de `simulate_projection` (4.5.0): `liability_total_interest`,
//! `liability_debt_free_month_index` y la identidad de `net_cash_monthly`.
//!
//! Estos tres campos existen para que «¿me compensa amortizar antes?» tenga una respuesta
//! **numérica** y no una lectura de la curva. Se calculan con el MISMO
//! `liability_amortization_schedule` que sirve `GET /v1/liabilities/{id}/schedule`, así que la
//! prueba que de verdad importa es cruzada: el mes de extinción que publica el what-if y el que
//! publica el calendario tienen que ser **el mismo número** para los mismos datos. Dos números
//! distintos para la misma pregunta es exactamente el fallo que este repo llama «cifras
//! plausibles pero mal».
//!
//! La mecánica exacta de los ejes `liability_overrides` vive en `crates/engine` (números
//! predichos a mano); aquí se cubre la superficie MCP end-to-end — incluida la compensación por
//! reembolso anticipado (#151), que es lo que hace que el what-if de amortizar no sea gratis.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::json;

const PROTOCOL: &str = "2026-07-28";

async fn mcp_post(app: &TestApp, bearer: &str, body: serde_json::Value) -> serde_json::Value {
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
    assert_eq!(resp.status, http::StatusCode::OK, "MCP POST failed: {resp:?}");
    let text = String::from_utf8(resp.body.clone()).expect("utf8 body");
    let content_type = resp
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.starts_with("application/json") {
        return serde_json::from_str(&text).expect("json body");
    }
    let mut last = None;
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                last = Some(v);
            }
        }
    }
    last.unwrap_or_else(|| panic!("no JSON data frame in SSE response:\n{text}"))
}

async fn create_token(app: &TestApp, owner: &LoggedInOwner) -> String {
    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            json!({"label": "liability kpis"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

fn tool_call(name: &str, arguments: serde_json::Value) -> serde_json::Value {
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

fn tool_json(envelope: &serde_json::Value) -> serde_json::Value {
    let result = &envelope["result"];
    assert_ne!(result["isError"], true, "tool devolvió error: {envelope}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).expect("json")
}

fn dec(v: &serde_json::Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba un string decimal, llegó {v}"))
        .parse()
        .expect("decimal")
}

/// Hogar con una hipoteca francesa de 100.000 € al 3 % y 500 €/mes — el mismo caso que el test del
/// engine `french_extinction_at_month_278` y el del calendario HTTP. Devuelve el id del pasivo.
async fn seed(app: &TestApp, owner: &LoggedInOwner) -> String {
    let cat_inc = app.create_category(owner, "income", "Nómina").await;
    let cat_exp = app.create_category(owner, "expense", "Vida").await;
    let cat_ast = app.create_category(owner, "asset", "Fondos").await;
    let cat_liab = app.create_category(owner, "liability", "Préstamos").await;
    let cat_cuota = app.create_category(owner, "expense", "Cuotas").await;
    for (cat, amount) in [(&cat_inc, "3000"), (&cat_exp, "1000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({"category_id": cat, "amount": amount}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({
                "category_id": cat_ast,
                "name": "MSCI World",
                "current_value": "50000",
                "is_liquid": true,
                "expected_annual_return_percent": "5",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({ "target_asset_id": asset_id, "kind": "remainder", "priority": 100 }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "sumidero: {r:?}");

    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({
                "category_id": cat_liab,
                "expense_category_id": cat_cuota,
                "label": "Hipoteca",
                "principal": "100000",
                "repayment_model": "french",
                "apr_percent": "3",
                "payment_amount": "500",
                "payment_frequency": "monthly",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");
    liab.json()["id"].as_str().unwrap().to_string()
}

/// **El what-if y el calendario cuentan lo mismo.**
///
/// PREDICCIONES (las mismas que el calendario HTTP y el engine, verificadas a 50 dígitos antes de
/// correr nada):
/// - `liability_debt_free_month_index` = **278** en los dos lados (sin overrides, escenario ==
///   baseline).
/// - `liability_total_interest` ≈ **38.802,80 €** en los dos lados.
/// - todos los deltas de deuda a **0**.
/// - `liability_extra_principal_monthly` = **0** en los dos lados: el baseline no lleva overrides
///   nunca, y sin `liability_overrides` el escenario tampoco.
#[tokio::test]
async fn the_what_if_debt_kpis_agree_with_the_liability_schedule() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_id = seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({ "months": 360 })),
        )
        .await,
    );

    for lado in ["baseline", "scenario"] {
        let k = &sim[lado];
        assert_eq!(
            k["liability_debt_free_month_index"], 278,
            "{lado}: mes libre de deuda"
        );
        assert!(
            k["liability_debt_free_absent_reason"].is_null(),
            "{lado}: {k}"
        );
        let interes = dec(&k["liability_total_interest"]);
        assert!(
            (interes - 38_802.7999).abs() < 0.05,
            "{lado}: interés esperado ≈ 38.802,80 €, obtenido {interes}"
        );
        assert_eq!(dec(&k["liability_extra_principal_monthly"]), 0.0, "{lado}");
    }

    let d = &sim["deltas"];
    assert_eq!(dec(&d["liability_extra_principal_monthly_delta"]), 0.0);
    assert_eq!(dec(&d["liability_total_interest_delta"]), 0.0);
    assert_eq!(d["liability_debt_free_months_delta"], 0);

    // El contraste cruzado: el mismo número por la otra superficie.
    let sch = app
        .get_with_cookie(
            &format!("/v1/liabilities/{liab_id}/schedule"),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(
        sch["payoff_month_index"], sim["baseline"]["liability_debt_free_month_index"],
        "el calendario y el what-if no pueden dar meses de extinción distintos"
    );
    assert!(
        (dec(&sch["total_interest_remaining"]) - dec(&sim["baseline"]["liability_total_interest"]))
            .abs()
            < 0.05,
        "…ni intereses distintos: {} vs {}",
        sch["total_interest_remaining"],
        sim["baseline"]["liability_total_interest"]
    );
}

/// La identidad publicada de `net_cash_monthly`, comprobable con una resta:
/// `net_recurring_monthly + monthly_cash_adjustment − liability_extra_principal_monthly`.
///
/// Existe porque el término de amortización extra **no es** un `monthly_cash_adjustment` (ese eje
/// es constante en todo el horizonte; la amortización se acaba con la deuda), así que se publica
/// aparte en vez de mezclarse — y la resta tiene que seguir cuadrando en los dos lados para que
/// un consumidor pueda verificarla sin conocer el código.
#[tokio::test]
async fn net_cash_monthly_stays_verifiable_by_subtraction() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({ "months": 360, "extra_monthly_savings": "200" }),
            ),
        )
        .await,
    );

    for lado in ["baseline", "scenario"] {
        let k = &sim[lado];
        let esperado = dec(&k["net_recurring_monthly"]) + dec(&k["monthly_cash_adjustment"])
            - dec(&k["liability_extra_principal_monthly"]);
        assert!(
            (dec(&k["net_cash_monthly"]) - esperado).abs() < 0.0001,
            "{lado}: net_cash_monthly debe cuadrar con la resta publicada ({} vs {esperado})",
            k["net_cash_monthly"]
        );
    }
    // El eje de caja mueve `net_cash_monthly` y NO el neto recurrente (contrato previo, intacto).
    assert_eq!(dec(&sim["deltas"]["net_recurring_monthly_delta"]), 0.0);
    assert_eq!(dec(&sim["deltas"]["net_cash_monthly_delta"]), 200.0);
    // …y tampoco toca la deuda.
    assert_eq!(dec(&sim["deltas"]["liability_total_interest_delta"]), 0.0);
}

/// Un hogar sin pasivos está libre de deuda **hoy** (mes 0) y no debe intereses. `Some(0)` y no
/// `null`: «no debo nada» es un hecho, no una ausencia de base para calcularlo — y `null`
/// obligaría al consumidor a leer un `*_absent_reason` que tampoco existiría.
#[tokio::test]
async fn a_household_without_liabilities_is_debt_free_at_month_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": cat_inc, "amount": "3000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({ "months": 120 })),
        )
        .await,
    );
    assert_eq!(sim["baseline"]["liability_debt_free_month_index"], 0);
    assert!(sim["baseline"]["liability_debt_free_absent_reason"].is_null());
    assert_eq!(dec(&sim["baseline"]["liability_total_interest"]), 0.0);
}

/// #151, números a mano sobre la hipoteca del seed (100.000 € / french 3 % / cuota 500):
/// lump de 20.000 € en el mes 12.
/// - Comisión DEFAULT (omitida) = 2 % ⇒ `liability_early_repayment_fee_total` = **400,00 €** en
///   el escenario (0 en el baseline; delta = 400). Es la única línea de la ola que cambia el
///   resultado de un caller de 4.4.0: antes ese what-if salía gratis.
/// - Con `early_repayment_fee_pct: "0"` explícito, la comisión desaparece (opt-out).
/// - `reduce_payment` conserva EXACTAMENTE el mes de extinción del baseline (**278**, el mismo
///   del calendario); `reduce_term` (default) lo adelanta.
#[tokio::test]
async fn the_early_repayment_fee_makes_the_what_if_not_free() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bearer = create_token(&app, &owner).await;
    let liab_id = seed(&app, &owner).await;

    // Default 2 %.
    let envelope = mcp_post(
        &app,
        &bearer,
        tool_call(
            "simulate_projection",
            json!({
                "months": 360,
                "liability_overrides": [
                    { "liability_id": liab_id, "lump_sum_amount": "20000", "lump_sum_month_index": 12 }
                ]
            }),
        ),
    )
    .await;
    let body = tool_json(&envelope);
    assert_eq!(
        body["scenario"]["liability_early_repayment_fee_total"].as_str(),
        Some("400.0000"),
        "20.000 × 2 % default: {body}"
    );
    assert_eq!(
        body["baseline"]["liability_early_repayment_fee_total"].as_str(),
        Some("0.0000"),
        "el baseline nunca comisiona"
    );
    assert_eq!(
        body["deltas"]["liability_early_repayment_fee_total_delta"].as_str(),
        Some("400.0000")
    );
    let base_free = body["baseline"]["liability_debt_free_month_index"].as_u64().unwrap();
    assert_eq!(base_free, 278, "el mismo 278 del calendario");
    let term_free = body["scenario"]["liability_debt_free_month_index"].as_u64().unwrap();
    assert!(term_free < base_free, "reduce_term (default) acorta el plazo");

    // Opt-out explícito.
    let envelope = mcp_post(
        &app,
        &bearer,
        tool_call(
            "simulate_projection",
            json!({
                "months": 360,
                "liability_overrides": [
                    { "liability_id": liab_id, "lump_sum_amount": "20000",
                      "lump_sum_month_index": 12, "early_repayment_fee_pct": "0" }
                ]
            }),
        ),
    )
    .await;
    let body = tool_json(&envelope);
    assert_eq!(
        body["scenario"]["liability_early_repayment_fee_total"].as_str(),
        Some("0.0000"),
        "con \"0\" explícito no hay comisión"
    );

    // «Reducir cuota»: la invariante del plazo.
    let envelope = mcp_post(
        &app,
        &bearer,
        tool_call(
            "simulate_projection",
            json!({
                "months": 360,
                "liability_overrides": [
                    { "liability_id": liab_id, "lump_sum_amount": "20000",
                      "lump_sum_month_index": 12, "early_repayment_effect": "reduce_payment" }
                ]
            }),
        ),
    )
    .await;
    let body = tool_json(&envelope);
    assert_eq!(
        body["scenario"]["liability_debt_free_month_index"].as_u64(),
        Some(278),
        "reduce_payment conserva el mes de extinción del baseline: {body}"
    );
    assert_eq!(body["deltas"]["liability_debt_free_months_delta"].as_i64(), Some(0));

    // Las dos puertas nuevas.
    let envelope = mcp_post(
        &app,
        &bearer,
        tool_call(
            "simulate_projection",
            json!({
                "months": 120,
                "liability_overrides": [
                    { "liability_id": liab_id, "lump_sum_amount": "1000",
                      "lump_sum_month_index": 3, "early_repayment_fee_pct": "3" }
                ]
            }),
        ),
    )
    .await;
    let text = envelope["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        envelope["result"]["isError"].as_bool().unwrap_or(false)
            && text.contains("early_repayment_fee_out_of_range"),
        "3 % supera el techo legal: {envelope}"
    );

    let envelope = mcp_post(
        &app,
        &bearer,
        tool_call(
            "simulate_projection",
            json!({
                "months": 120,
                "liability_overrides": [
                    { "liability_id": liab_id, "early_repayment_effect": "reduce_payment",
                      "repayment_model": "french", "apr_percent": "3" }
                ]
            }),
        ),
    )
    .await;
    let text = envelope["result"]["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        envelope["result"]["isError"].as_bool().unwrap_or(false)
            && text.contains("liability_early_repayment_axis_needs_amortization"),
        "efecto sin amortización es un no-op prohibido: {envelope}"
    );
}
