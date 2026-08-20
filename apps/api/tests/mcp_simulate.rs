//! Tool MCP `simulate_projection` (what-if puro): paridad del baseline con `get_projection`,
//! el par discriminador gasto-real vs ajuste-neutro, equivalencia date↔month_index del one-off,
//! rentabilidades negativas (post-fix del engine), cotas de validación y neutralidad de cache.

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
            json!({"label": "simulate tests"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

fn request_meta() -> serde_json::Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": PROTOCOL,
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

fn tool_call(name: &str, arguments: serde_json::Value) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments, "_meta": request_meta()}
    })
}

fn tool_json(envelope: &serde_json::Value) -> serde_json::Value {
    let result = &envelope["result"];
    assert_ne!(result["isError"], true, "tool devolvió error: {envelope}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).expect("json")
}

fn dec(v: &serde_json::Value) -> f64 {
    v.as_str().unwrap().parse().unwrap()
}

/// Household mínimo con proyección no trivial: ingreso 3000, gasto 1000 (→ ahorro 2000/mes),
/// un activo líquido al 5 % y target FIRE en modo annual_expense (defaults).
async fn seed(app: &TestApp, owner: &LoggedInOwner) -> String {
    let cat_inc = app.create_category(owner, "income", "Nómina").await;
    let cat_exp = app.create_category(owner, "expense", "Vida").await;
    let cat_ast = app.create_category(owner, "asset", "Fondos").await;
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
    let r = app
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
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn baseline_without_overrides_matches_get_projection_and_scenario() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    let proj = tool_json(&mcp_post(&app, &token, tool_call("get_projection", json!({}))).await);

    // Sin overrides el escenario ES el baseline: deltas cero.
    assert_eq!(sim["baseline"], sim["scenario"], "{sim}");
    assert_eq!(dec(&sim["deltas"]["final_net_worth_delta"]), 0.0);
    assert_eq!(sim["deltas"]["jubilacion_months_delta"], 0);

    // Y el baseline coincide con la proyección servida (mismo contexto y engine).
    assert_eq!(
        sim["baseline"]["jubilacion_month_index"], proj["jubilacion_month_index"],
        "sim: {sim} proj jubilación: {:?}",
        proj["jubilacion_month_index"]
    );
    assert_eq!(sim["horizon_months"], proj["months"]);
    // Los puntos del chart van como números f64 (decisión histórica del wire); el KPI del
    // simulate como Decimal-string → comparar con tolerancia relativa.
    let last_point = proj["points"].as_array().unwrap().last().unwrap().clone();
    let sim_final = dec(&sim["baseline"]["final_net_worth"]);
    let proj_final = last_point["net_worth"].as_f64().unwrap();
    assert!(
        (sim_final - proj_final).abs() <= proj_final.abs().max(1.0) * 1e-9,
        "patrimonio final baseline {sim_final} vs último punto {proj_final}"
    );

    // Sin include_series no hay series.
    assert!(sim.get("series").is_none(), "{sim}");
}

#[tokio::test]
async fn real_expense_moves_fire_target_but_neutral_cash_adjustment_does_not() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    // Gasto REAL: mueve el target FIRE (modo annual_expense) y baja el patrimonio final.
    let real = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "200"})),
        )
        .await,
    );
    assert!(
        dec(&real["deltas"]["fire_target_base_delta"]) > 0.0,
        "gastar más de verdad sube el objetivo: {real}"
    );
    // Objetivo más alto + menos ahorro → la jubilación se retrasa. (El patrimonio FINAL no es
    // monótono: jubilarse más tarde también retrasa el drenaje post-FIRE.)
    assert!(
        real["deltas"]["jubilacion_months_delta"].as_i64().unwrap() > 0,
        "{real}"
    );

    // Ajuste NEUTRO: misma caja menos, pero el target no se mueve.
    let neutral = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"extra_monthly_cash_adjustment": "200"}),
            ),
        )
        .await,
    );
    assert_eq!(
        dec(&neutral["deltas"]["fire_target_base_delta"]),
        0.0,
        "el ajuste neutro no toca el objetivo: {neutral}"
    );
    assert!(
        neutral["deltas"]["jubilacion_months_delta"].as_i64().unwrap() >= 0,
        "menos caja nunca adelanta la jubilación: {neutral}"
    );
    assert!(
        real["deltas"]["jubilacion_months_delta"].as_i64().unwrap()
            > neutral["deltas"]["jubilacion_months_delta"].as_i64().unwrap(),
        "el gasto real retrasa MÁS que el ajuste neutro (además mueve el objetivo)"
    );

    // Y el ahorro extra adelanta la jubilación sin mover el target.
    let savings = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_savings": "300"})),
        )
        .await,
    );
    assert_eq!(dec(&savings["deltas"]["fire_target_base_delta"]), 0.0);
    assert!(
        savings["deltas"]["jubilacion_months_delta"].as_i64().unwrap() < 0,
        "{savings}"
    );
}

#[tokio::test]
async fn one_off_by_date_equals_month_index_and_series_are_opt_in() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    // El servidor está en el mes 0; una fecha dentro de 3 meses = month_index 4 (mes civil
    // actual + 3). Derivamos la fecha del anchor devuelto por la propia proyección.
    let proj = tool_json(&mcp_post(&app, &token, tool_call("get_projection", json!({}))).await);
    let anchor = proj["anchor_date_ymd"].as_str().unwrap(); // YYYY-MM-DD (hoy)
    let (y, m) = (
        anchor[0..4].parse::<i32>().unwrap(),
        anchor[5..7].parse::<u32>().unwrap(),
    );
    let (ty, tm) = if m + 3 > 12 { (y + 1, m + 3 - 12) } else { (y, m + 3) };
    let date = format!("{ty:04}-{tm:02}-15");

    let by_date = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"one_off_expense": {"amount": "5000", "date": date}, "include_series": true}),
            ),
        )
        .await,
    );
    let by_index = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"one_off_expense": {"amount": "5000", "month_index": 4}}),
            ),
        )
        .await,
    );
    assert_eq!(
        by_date["deltas"], by_index["deltas"],
        "date y month_index equivalentes deben producir el mismo escenario"
    );
    assert_eq!(by_date["scenario"], by_index["scenario"]);
    assert_ne!(
        by_date["scenario"], by_date["baseline"],
        "un one-off de 5000 debe mover el escenario"
    );

    // include_series: series decimadas paralelas presentes solo si se piden.
    let series = &by_date["series"];
    assert!(series.is_object(), "{by_date}");
    let n = series["month_indices"].as_array().unwrap().len();
    assert!(n > 10);
    assert_eq!(series["baseline_net_worth"].as_array().unwrap().len(), n);
    assert_eq!(series["scenario_net_worth"].as_array().unwrap().len(), n);
    assert!(by_index.get("series").is_none());
}

#[tokio::test]
async fn negative_return_override_lowers_final_net_worth() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let asset_id = seed(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"asset_return_overrides": [
                    {"asset_id": asset_id, "expected_annual_return_percent": "-30"}
                ]}),
            ),
        )
        .await,
    );
    assert!(
        dec(&sim["deltas"]["final_net_worth_delta"]) < 0.0,
        "una rentabilidad -30 % debe hundir el patrimonio final (solo posible tras el fix del engine): {sim}"
    );
}

#[tokio::test]
async fn validation_bounds_are_enforced() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    for (args, needle) in [
        (json!({"swr_pct": "5"}), "swr_pct"),
        (json!({"months": 6}), "months"),
        (
            json!({"asset_return_overrides": [
                {"asset_id": "00000000-0000-0000-0000-000000000001", "expected_annual_return_percent": "1"}
            ]}),
            "unknown asset_id",
        ),
        (
            json!({"asset_return_overrides": [
                {"asset_id": "00000000-0000-0000-0000-000000000001", "expected_annual_return_percent": "-100"}
            ]}),
            "-100",
        ),
        (json!({"extra_monthly_expense": "-5"}), ">= 0"),
        (
            json!({"one_off_expense": {"amount": "100"}}),
            "exactly one",
        ),
        (
            json!({"one_off_expense": {"amount": "100", "month_index": 1, "date": "2030-01-01"}}),
            "exactly one",
        ),
        (json!({"annual_inflation_percent": "60"}), "inflation"),
        (json!({"retirement_annual_expense": "0"}), "retirement_annual_expense"),
    ] {
        let envelope = mcp_post(&app, &token, tool_call("simulate_projection", args.clone())).await;
        assert_eq!(
            envelope["result"]["isError"], true,
            "esperado error para {args}: {envelope}"
        );
        let body: serde_json::Value =
            serde_json::from_str(envelope["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["error"], "bad_request", "{args}");
        assert!(
            body["message"].as_str().unwrap().contains(needle),
            "mensaje para {args}: {body}"
        );
    }
}

#[tokio::test]
async fn simulate_never_touches_the_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    // 1. Simular con la cache vacía no crea ninguna entrada.
    let _ = tool_json(
        &mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await,
    );
    assert!(
        app.state.projection_cache.read().await.is_empty(),
        "simulate_projection no debe poblar la cache"
    );

    // 2. Calentar la cache con la proyección real y simular un escenario: la entrada sobrevive
    //    intacta (mismo nº de entradas).
    let _ = tool_json(&mcp_post(&app, &token, tool_call("get_projection", json!({}))).await);
    let entries_before = app.state.projection_cache.read().await.len();
    assert!(entries_before > 0);
    let _ = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "100"})),
        )
        .await,
    );
    assert_eq!(app.state.projection_cache.read().await.len(), entries_before);
}

/// Identidad entre superficies: los KPIs de salud del `simulate_projection` sin overrides deben
/// coincidir **exactamente** con el `financial_health` de `GET /v1/summary`, en los tres modos de
/// `savings_source`.
///
/// El caso discriminante es el **modo A con un pasivo activo**: ahí `input.expense_regular_monthly`
/// excluye deliberadamente la cuota (`budget.rs`: fundirla contaría el pasivo dos veces en toda la
/// proyección, en silencio) y la cuota entra por `debt_service_monthly`. Si alguien implementara
/// `expense_total_monthly` como `input.expense_regular_monthly` a secas, este test cae por
/// exactamente el importe de la cuota — 400 €/mes — y no por un epsilon.
///
/// PREDICCIÓN modo A: income 3000; gasto de presupuesto 1000 en «Vida» + 400 de cuota (partida
/// derivada del pasivo desde 3.7.0) = 1400 de gasto total; net = 1600; savings_rate = 1600/3000 =
/// 0,533333 (6 dp). En B y C el gasto sale del promedio real y `debt_service_monthly` es 0 por
/// contrato.
#[tokio::test]
async fn sim_kpis_match_summary_financial_health_in_all_three_modes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    // Pasivo activo: su cuota es gasto en los tres modos, pero por caminos distintos.
    let liab_cat = app.create_category(&owner, "liability", "Deuda").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;
    let created = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat,
                    "label": "Hipoteca", "principal": "100000", "payment_amount": "400",
                    "payment_frequency": "monthly", "payment_end_date": "2040-01-01" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");

    // Movimientos reales en un mes cerrado, para que B y C tengan «meses reales» y no caigan al
    // presupuesto (el fallback también cumpliría la identidad, pero no ejercitaría el modo).
    let exp_cat = app.create_category(&owner, "expense", "Súper").await;
    let inc_cat = app.create_category(&owner, "income", "Sueldo").await;
    for (concept, amount, kind, cat) in [
        ("COMPRA SUPER", "-800", "expense", &exp_cat),
        ("NOMINA", "2500", "income", &inc_cat),
    ] {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({ "op_date": "2026-06-10", "concept": concept, "amount": amount,
                        "kind": kind, "category_id": cat }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    for mode in ["budget", "transactions_avg", "budget_income_real_expense"] {
        let r = app
            .patch_json_with_cookie(
                "/v1/installation",
                json!({ "fire_settings": { "savings_source": mode } }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "set mode {mode}: {r:?}");

        let sim =
            tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
        let k = &sim["baseline"];
        let summary = app.get_with_cookie("/v1/summary", &owner.cookie).await;
        let h = &summary.json()["financial_health"];

        assert_eq!(
            dec(&k["income_monthly"]),
            dec(&h["income_monthly_equivalent"]),
            "modo {mode}: income"
        );
        assert_eq!(
            dec(&k["expense_total_monthly"]),
            dec(&h["expense_total_monthly_equivalent"]),
            "modo {mode}: gasto total (regular + servicio de deuda) — si falla por el importe \
             exacto de la cuota, `expense_total_monthly` está leyendo `expense_regular_monthly`"
        );
        assert_eq!(
            dec(&k["net_monthly"]),
            dec(&h["net_monthly_equivalent"]),
            "modo {mode}: neto"
        );
        assert_eq!(
            dec(&k["savings_rate"]),
            dec(&h["savings_rate"]),
            "modo {mode}: tasa de ahorro (misma precisión en las dos superficies)"
        );

        // Identidad interna del propio KPI, en los tres modos.
        assert_eq!(
            dec(&k["net_monthly"]),
            dec(&k["income_monthly"]) - dec(&k["expense_total_monthly"]),
            "modo {mode}: net = income − expense_total"
        );

        // El servicio de deuda solo es no nulo en modo A; en B/C la cuota ya es un movimiento real.
        let debt = dec(&k["debt_service_monthly"]);
        if mode == "budget" {
            assert_eq!(debt, 400.0, "modo A: la cuota viaja en debt_service_monthly");
        } else {
            assert_eq!(debt, 0.0, "modo {mode}: debt_service es 0 por contrato");
        }

        // Sin overrides, todos los deltas de salud son cero.
        let d = &sim["deltas"];
        for field in [
            "income_monthly_delta",
            "expense_total_monthly_delta",
            "net_monthly_delta",
            "savings_rate_delta",
        ] {
            assert_eq!(dec(&d[field]), 0.0, "modo {mode}: {field} sin overrides");
        }
    }
}
