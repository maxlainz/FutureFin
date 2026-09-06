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
/// 8.000/0,04 = 200.000 ⇒ jubilado desde el mes 0. La cartera se VACÍA en el mes 100 del BUCLE
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
    // #210 — **99, no 100**: el motor agota la cartera en su mes 100 (1-based del bucle) y desde
    // 5.0.0 el handler publica ese hecho en la MISMA rejilla 0-based que `points[].month_index` y
    // que `jubilacion_month_index`, con `engine_month_to_grid` (k − 1). El mes civil no se ha
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

/// T3 · `swr_pct = "0"` es escritura válida («jamás») pero anulaba el número FIRE SIN explicación
/// en HTTP. Ahora la razón viaja con el mismo literal que simulate_projection publica — paridad
/// por construcción (mismo campo, misma función).
///
/// **5.0.0**: el campo se llama `fire_number_classic_absent_reason` y lo que falta es un ESCALAR
/// informativo, no un objetivo que dispare nada (el motor recibe `fire_target: None`). Los tres
/// literales y la causa son los mismos; el nombre cambió para que su ausencia no se lea como
/// «esta simulación no se jubila», que es lo que significaba en 4.15.x.
#[tokio::test]
async fn fire_number_classic_absent_reason_reaches_http_with_swr_zero() {
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
    assert!(series["fire_number_classic_today"].is_null(), "{series}");
    assert_eq!(
        series["fire_number_classic_absent_reason"], "swr_not_positive",
        "{series}"
    );

    let token = create_token(&app, &owner).await;
    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    assert_eq!(
        sim["baseline"]["fire_target_absent_reason"], "swr_not_positive",
        "paridad HTTP↔MCP rota: {sim}"
    );
}

/// T4 · Las otras dos causas + el control de que el campo no es siempre no-nulo.
#[tokio::test]
async fn fire_number_classic_absent_reason_covers_the_other_two_causes() {
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
    assert_eq!(
        series["fire_number_classic_absent_reason"], "net_need_not_positive",
        "{series}"
    );

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
    assert!(series["fire_number_classic_absent_reason"].is_null(), "{series}");
    assert!(!series["fire_number_classic_today"].is_null(), "{series}");
}

// =================================================================================================
// 5.0.0 — los estados de fallo del modelo v2
// =================================================================================================

/// **Una pensión declarada que no puede entrar al plan lo DICE** (B4).
///
/// Sin fecha de nacimiento no hay edad que convertir en mes, así que la pensión no tiene
/// calendario y el bucle no la cobra nunca. Publicar la serie sin más dejaría al usuario mirando
/// una proyección sin pensión con la pensión escrita en su perfil: el mismo hueco silencioso que
/// esta casa persigue. La razón viaja en su propio campo, no solo como aviso.
#[tokio::test]
async fn a_pension_without_birth_date_publishes_its_absent_reason() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("penx").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
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
            json!({"pension": {"monthly_amount_today": "1200", "starts_at_age": 67},
                   "birth_date": Value::Null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_eq!(s["pension_absent_reason"], "birth_date_missing", "{s}");
    assert!(
        s["pension_start_month_index"].is_null(),
        "sin calendario no hay mes de inicio: {s}"
    );
    // Y el plan entero está ausente por la misma causa (C5), con la serie publicándose igual.
    assert_eq!(s["plan_absent_reason"], "birth_date_missing", "{s}");
    assert!(!s["points"].as_array().expect("points").is_empty(), "{s}");
}

/// **La pensión que empieza DENTRO de la media jornada y no se cobra se avisa** (B9).
///
/// `fraction_while_partial` vale 0 por defecto —el supuesto conservador: contar una pensión que
/// no cobras adelanta la fecha con dinero que no existe—, así que un hogar que baja a media
/// jornada a los 60 y cumple la edad de pensión a los 63 pasa tres años sin sueldo completo y sin
/// pensión. No es un error: es el supuesto. Pero tiene que decirse, porque la tarjeta de la
/// pensión enseña un importe que en esos meses no entra.
#[tokio::test]
async fn a_pension_inside_the_partial_phase_warns_that_it_is_unpaid() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("b9x").await;
    let cat_i = app.create_category(&owner, "income", "Nómina").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let cat_a = app.create_category(&owner, "asset", "Fondos").await;
    for body in [
        json!({"category_id": cat_i, "amount": "3000", "ends_at_retirement": true}),
        json!({"category_id": cat_e, "amount": "2000", "ends_at_retirement": false}),
    ] {
        let r = app
            .post_json_with_cookie("/v1/budget/entries", body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Indexado", "current_value": "100000",
                   "is_liquid": true, "expected_annual_return_percent": "5",
                   "annual_volatility_percent": "15"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    // Media jornada a los 55 (edad fija) y pensión a los 67: **doce años** de fase con la pensión
    // entrando por el medio. La fase es larga a propósito — con una ventana corta el sorteo
    // podría colocar la jubilación TOTAL antes de la pensión, y entonces no habría ningún mes en
    // que la pensión coincida con la media jornada: el aviso no tendría nada que advertir.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "partial", "swr_pct": "4",
                   "partial_retirement": {"mode": "at_age", "starts_at_age": 55,
                                          "income_monthly_today": "1000"},
                   "pension": {"monthly_amount_today": "1400", "starts_at_age": 67,
                               "fraction_while_partial": "0"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    assert!(
        warnings.contains(&"pension_unpaid_during_partial"),
        "la pensión empieza dentro de la fase y no se cobra: {warnings:?} en {s}"
    );

    // Control negativo: cobrándola entera durante la fase, el aviso desaparece — el aviso mide el
    // SUPUESTO, no la mera coincidencia de fechas.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"pension": {"monthly_amount_today": "1400", "starts_at_age": 67,
                               "fraction_while_partial": "1"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let s2 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    let warnings2: Vec<&str> = s2["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    assert!(
        !warnings2.contains(&"pension_unpaid_during_partial"),
        "cobrándola no hay nada que advertir: {warnings2:?} en {s2}"
    );
}

/// **Un plan que no alcanza el umbral en ningún mes dice `not_reachable`, no «mes 0»**.
///
/// «No llegas» y «llegas ya» son respuestas opuestas, y la segunda es la que sale sola si alguien
/// rellena la ausencia con el valor que más se le parece. Lo que se publica es la base
/// `not_reachable`, la fecha a `null` y el éxito del **mejor intento observado** al lado — porque
/// «lo más cerca que llegas es el 40 %» es información, y un hueco no.
#[tokio::test]
async fn a_plan_that_cannot_reach_the_threshold_says_not_reachable_not_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("nrx").await;
    let cat_i = app.create_category(&owner, "income", "Nómina").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    // Gasto por encima del ingreso y sin cartera: no hay ningún mes del horizonte en que este
    // hogar pueda jubilarse.
    for body in [
        json!({"category_id": cat_i, "amount": "1200", "ends_at_retirement": true}),
        json!({"category_id": cat_e, "amount": "1800", "ends_at_retirement": false}),
    ] {
        let r = app
            .post_json_with_cookie("/v1/budget/entries", body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    let s = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_eq!(s["retirement_date_basis"], "not_reachable", "{s}");
    assert!(
        s["jubilacion_month_index"].is_null(),
        "sin fecha se publica null, JAMÁS un 0 — un 0 se lee como «ya puedes»: {s}"
    );
    assert!(
        s["plan_absent_reason"].is_null(),
        "«no llegas» es un RESULTADO, no un plan ausente: {s}"
    );
    assert!(
        !s["success_of_plan"].is_null(),
        "sin fecha sigue habiendo una medición: la del mejor intento: {s}"
    );
    assert!(
        s["needed_capital_today"].is_null()
            && s["needed_capital_absent_reason"].is_string(),
        "un hogar sin líquido no necesita 0 €; la ausencia se nombra: {s}"
    );
}
