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
async fn final_net_worth_is_nominal_and_its_real_twin_deflates_by_month_index() {
    // Issue #27 §7. `final_net_worth` es nominal por contrato del motor, y con el horizonte por
    // defecto está a décadas vista: la cifra impresiona y no dice nada. El hermano real la lleva a
    // euros de hoy con la inflación EFECTIVA DEL LADO.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    // Sin inflación el deflactor es exactamente 1: el mismo STRING, no un valor parecido.
    let sin = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    assert_eq!(
        sin["baseline"]["final_net_worth"], sin["baseline"]["final_net_worth_real"],
        "con inflación 0 el par debe ser idéntico carácter a carácter: {}",
        sin["baseline"]
    );
    assert_eq!(dec(&sin["deltas"]["final_net_worth_real_delta"]), 0.0);

    // Con inflación, la razón entre ambos es (1+i)^(meses/12) — deflactado por ÍNDICE DE MES, que
    // es lo que hay que fijar: deflactar por la posición en un array decimado daría otra cosa.
    let con = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"annual_inflation_percent": "3"})),
        )
        .await,
    );
    let esc = &con["scenario"];
    let meses = con["horizon_months"].as_u64().unwrap() as f64;
    let nominal = dec(&esc["final_net_worth"]);
    let real = dec(&esc["final_net_worth_real"]);
    assert!(real < nominal, "la inflación tiene que morder: {esc}");
    let esperado = 1.03_f64.powf(meses / 12.0);
    let observado = nominal / real;
    assert!(
        (observado - esperado).abs() / esperado < 1e-9,
        "razón nominal/real = {observado}, esperada {esperado} para {meses} meses al 3 %: {esc}"
    );

    // El baseline conserva la inflación de la instalación (0), así que su par sigue idéntico: es
    // la prueba de que el deflactor es por lado y no global.
    assert_eq!(
        con["baseline"]["final_net_worth"], con["baseline"]["final_net_worth_real"],
        "{}", con["baseline"]
    );
}

/// Household con presupuesto, un pasivo con cuota y movimientos reales en un mes cerrado — el
/// mínimo para que los modos B y C tengan «meses reales» y no caigan al presupuesto.
async fn seed_with_real_movements(app: &TestApp, owner: &LoggedInOwner) {
    seed(app, owner).await;
    let liab_cat = app.create_category(owner, "liability", "Deuda").await;
    let liab_exp_cat = app.create_category(owner, "expense", "Cuotas").await;
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

    let exp_cat = app.create_category(owner, "expense", "Súper").await;
    let inc_cat = app.create_category(owner, "income", "Sueldo").await;
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
}

#[tokio::test]
async fn savings_source_override_predicts_exactly_what_persisting_it_would_do() {
    // Issue #27 §3. El sentido de poder simular «¿y si cambio de fuente de ahorro?» es que prediga
    // lo que pasaría al cambiarla de verdad. Por eso el override usa el MISMO
    // `FireSettingsPatch::apply_to` que el PATCH real: dos copias del aplicador se separan sin que
    // ningún test lo note. Este es ese test.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_with_real_movements(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    for modo in ["transactions_avg", "budget_income_real_expense"] {
        // Instalación en modo A; el escenario pide el modo por override, sin persistir nada.
        let r = app
            .patch_json_with_cookie(
                "/v1/installation",
                json!({ "fire_settings": { "savings_source": "budget" } }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
        let simulado = tool_json(
            &mcp_post(
                &app,
                &token,
                tool_call(
                    "simulate_projection",
                    json!({"fire_settings_overrides": {"savings_source": modo}}),
                ),
            )
            .await,
        );

        // Ahora el cambio de verdad, y el baseline de una simulación sin overrides.
        let r = app
            .patch_json_with_cookie(
                "/v1/installation",
                json!({ "fire_settings": { "savings_source": modo } }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
        let persistido =
            tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);

        assert_eq!(
            simulado["scenario"], persistido["baseline"],
            "modo {modo}: simular el cambio debe dar lo MISMO que hacerlo. Si difieren, el              aplicador del patchset se ha bifurcado entre el PATCH real y la simulación."
        );
        // Y el escenario tiene que diferir del baseline, o el test pasaría por no hacer nada.
        assert_ne!(
            simulado["scenario"]["expense_base_monthly"],
            simulado["baseline"]["expense_base_monthly"],
            "modo {modo}: el override no ha movido la base de gasto"
        );
        // Los tres efectos en cascada del modo real, visibles en el eco:
        assert_eq!(simulado["scenario"]["savings_source"], modo);
        assert_eq!(
            dec(&simulado["baseline"]["debt_service_monthly"]),
            400.0,
            "modo A: la cuota cuenta aparte"
        );
        assert!(
            simulado["baseline"]["debt_service_absent_reason"].is_null(),
            "modo A: la cuota es medible, no hay razón de ausencia"
        );
        // INVERTIDO en 4.8.0 (#142, opción 3): la cuota SALE del promedio y vuelve como
        // servicio de deuda REAL también en B/C — una sola cuenta, en los tres modos. El
        // literal `included_in_real_expense` se retiró con su modo.
        assert_eq!(
            dec(&simulado["scenario"]["debt_service_monthly"]),
            400.0,
            "modo {modo}: la cuota es servicio de deuda real: {}",
            simulado["scenario"]
        );
        assert!(
            simulado["scenario"]["debt_service_absent_reason"].is_null(),
            "modo {modo}: ya no hay razón de ausencia"
        );
    }
}

#[tokio::test]
async fn savings_source_override_without_real_months_says_it_fell_back() {
    // El fallback por falta de meses reales es silencioso por diseño. Sin el eco de la fuente
    // efectiva, pedir modo B sobre una instalación sin movimientos devuelve un escenario idéntico
    // al baseline y NADA lo explica — que es la razón de que el §8 y el §3 vayan juntos.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await; // presupuesto, cero transacciones
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"fire_settings_overrides": {"savings_source": "transactions_avg"}}),
            ),
        )
        .await,
    );

    assert_eq!(
        sim["scenario"], sim["baseline"],
        "sin meses reales el modo B cae al presupuesto: el escenario ES el baseline"
    );
    assert_eq!(
        sim["scenario"]["savings_source"], "budget",
        "y el eco tiene que decir que se usó presupuesto, no lo que se pidió: {}",
        sim["scenario"]
    );
    assert_eq!(sim["scenario"]["savings_expense_basis"]["basis"], "budget");
}

#[tokio::test]
async fn fire_settings_overrides_reuse_the_validation_of_the_real_patch() {
    // Los overrides pasan por `validate_fire_settings`, la misma del PATCH: no hay una segunda
    // lista de cotas que pueda divergir. Y los enums se parsean con su `Deserialize` de dominio,
    // así que un valor inventado se rechaza con el mismo código que por HTTP.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    for (overrides, needle) in [
        // 5.0.0: `fire_number_mode` y `fire_number_manual_amount` salieron de este override —
        // son del perfil de jubilación por usuario (D13) y volverán como `profile_overrides` en
        // WP5. Los tres casos que quedan siguen probando lo mismo: la validación es la del PATCH
        // real, no una segunda lista de cotas.
        (json!({"savings_source": "no_existe"}), "savings_source"),
        (json!({"income_avg_window_mode": "semanal"}), "income_avg_window_mode"),
        (json!({"expense_avg_window_months": 0}), "expense_avg_window_months"),
    ] {
        let envelope = mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({ "fire_settings_overrides": overrides }),
            ),
        )
        .await;
        let result = &envelope["result"];
        assert_eq!(result["isError"], true, "debía fallar con {overrides}: {envelope}");
        let body = result["content"][0]["text"].as_str().unwrap();
        assert!(
            body.contains(needle),
            "el error de {overrides} debe nombrar «{needle}» y dice: {body}"
        );
    }
}

#[tokio::test]
async fn average_window_override_moves_the_basis_without_persisting_it() {
    // «¿Y si el promedio fuese de otra ventana?» era imposible sin persistirlo. El §3 del issue lo
    // señala aparte porque con una ventana corta un mes atípico mueve la proyección entera.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_with_real_movements(&app, &owner).await;
    let token = create_token(&app, &owner).await;
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "transactions_avg" } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"fire_settings_overrides": {
                    "expense_avg_window_months": 3,
                    "expense_avg_window_mode": "calendar"
                }}),
            ),
        )
        .await,
    );

    assert_eq!(sim["scenario"]["savings_expense_basis"]["window_months"], 3);
    assert_eq!(sim["scenario"]["savings_expense_basis"]["window_mode"], "calendar");
    // El baseline conserva la ventana configurada: el override es del lado, no global.
    assert_eq!(sim["baseline"]["savings_expense_basis"]["window_months"], 12);
    // Y nada se ha persistido: una simulación posterior sin overrides sigue viendo la ventana
    // configurada. Se comprueba por la MISMA superficie, que es donde se notaría la fuga.
    let despues =
        tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    assert_eq!(
        despues["baseline"]["savings_expense_basis"]["window_months"], 12,
        "la simulación no persiste NADA: {}",
        despues["baseline"]
    );
}

#[tokio::test]
async fn a_negative_expense_override_actually_cuts_and_moves_every_dependent_kpi() {
    // Issue #27 §1, el problema del título: la herramienta solo sabía empeorar el escenario.
    // Un recorte tiene que mover lo que un aumento mueve, con el signo contrario — y a diferencia
    // de los ejes de caja, tiene que mover TAMBIÉN gasto total, neto, tasa de ahorro y runway.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await; // gasto 1000, ingreso 3000
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "-200"})),
        )
        .await,
    );
    let d = &sim["deltas"];
    assert_eq!(dec(&sim["scenario"]["expense_base_monthly"]), 800.0, "{}", sim["scenario"]);
    assert_eq!(dec(&d["expense_total_monthly_delta"]), -200.0, "{d}");
    assert_eq!(dec(&d["net_recurring_monthly_delta"]), 200.0, "{d}");
    // Un recorte de gasto SÍ mueve las dos: cambia el neto recurrente y, con él, la caja.
    assert_eq!(dec(&d["net_cash_monthly_delta"]), 200.0, "{d}");
    assert!(dec(&d["savings_rate_delta"]) > 0.0, "recortar sube la tasa de ahorro: {d}");
    assert!(dec(&d["runway_months_delta"]) > 0.0, "y alarga el runway: {d}");
    // Y el objetivo baja: menos gasto, menos patrimonio necesario (modo annual_expense).
    assert!(dec(&d["fire_target_base_delta"]) < 0.0, "{d}");
    assert!(
        d["jubilacion_months_delta"].as_i64().unwrap() < 0,
        "recortar adelanta la jubilación: {d}"
    );

    // Esos cuatro deltas son EXACTAMENTE los que salen 0 con los ejes de caja. Es la diferencia
    // que la descripción de la tool promete, comprobada aquí en la misma llamada.
    let caja = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_savings": "200"})),
        )
        .await,
    );
    for campo in [
        "expense_total_monthly_delta",
        "net_recurring_monthly_delta",
        "savings_rate_delta",
        "runway_months_delta",
    ] {
        assert_eq!(
            dec(&caja["deltas"][campo]),
            0.0,
            "el eje de caja no mueve {campo}: {}",
            caja["deltas"]
        );
    }
    // …pero SÍ mueve el campo que existe justo para eso (4.4.0). Antes de este par de campos, el
    // eje de caja devolvía delta 0 en TODO lo que se parecía a «ahorro», que es exactamente lo que
    // el usuario acababa de preguntar: `net_monthly` era el neto recurrente con nombre ambiguo.
    assert_eq!(
        dec(&caja["deltas"]["net_cash_monthly_delta"]),
        200.0,
        "el eje de caja mueve la caja mensual: {}",
        caja["deltas"]
    );
    assert_eq!(
        dec(&caja["scenario"]["monthly_cash_adjustment"]),
        200.0,
        "el escenario declara su ajuste constante: {}",
        caja["scenario"]
    );
    assert_eq!(
        dec(&caja["baseline"]["monthly_cash_adjustment"]),
        0.0,
        "el baseline nunca lleva ajuste: {}",
        caja["baseline"]
    );
    assert_eq!(
        dec(&caja["scenario"]["net_cash_monthly"]),
        dec(&caja["scenario"]["net_recurring_monthly"])
            + dec(&caja["scenario"]["monthly_cash_adjustment"]),
        "identidad net_cash = net_recurring + ajuste: {}",
        caja["scenario"]
    );
    // Y la nota de modelo viaja siempre: es la única tool que deja mover los supuestos.
    assert!(
        caja["model_note"].as_str().is_some_and(|n| n.len() > 60),
        "simulate_projection debe publicar model_note: {caja}"
    );
}

#[tokio::test]
async fn an_oversized_cut_floors_the_base_at_zero_and_says_so() {
    // El suelo se eligió sobre rechazar porque el error tendría que nombrar un número que el
    // llamante no puede conocer de antemano: la base efectiva es justo lo que la herramienta
    // existe para revelar. A cambio, el recorte aplicado tiene que ser LEGIBLE en la respuesta.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await; // gasto 1000
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "-5000"})),
        )
        .await,
    );
    let esc = &sim["scenario"];
    assert_eq!(
        dec(&esc["expense_base_monthly"]),
        0.0,
        "la base se queda en 0, nunca negativa: {esc}"
    );
    assert_eq!(dec(&esc["expense_retirement_base_monthly"]), 0.0, "{esc}");
    assert_eq!(dec(&esc["expense_total_monthly"]), 0.0, "{esc}");
    // Sin gasto no hay número FIRE en modo `annual_expense`. No es un fallo, y la razón lo dice.
    assert!(esc["fire_target_base"].is_null(), "{esc}");
    assert_eq!(esc["fire_target_absent_reason"], "net_need_not_positive", "{esc}");
    assert!(esc["jubilacion_month_index"].is_null(), "{esc}");
    // Sin base de gasto tampoco hay runway que contar (y no es «infinito»).
    assert!(esc["runway_months"].is_null(), "{esc}");
    assert_eq!(esc["runway_is_indefinite"], false, "{esc}");
}

#[tokio::test]
async fn the_expense_floor_never_leaks_into_the_read_path() {
    // El riesgo real del suelo: un `.max(0)` incondicional tocaría también
    // `GET /v1/projection/series` y `GET /v1/summary`, porque el clamp vive DENTRO del ensamblado
    // que comparten. Está gateado a que el override recorte; esto lo fija.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    // Sin override, los dos lados son idénticos entre sí y al GET (el mismo camino de código).
    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    assert_eq!(sim["baseline"], sim["scenario"]);

    // Con un override POSITIVO tampoco se clampa nada: el gate mira el signo, no la presencia.
    let sube = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "250"})),
        )
        .await,
    );
    assert_eq!(dec(&sube["scenario"]["expense_base_monthly"]), 1250.0);
    assert_eq!(
        dec(&sube["baseline"]["expense_base_monthly"]),
        1000.0,
        "el baseline nunca ve el override"
    );

    let proj = app.get_with_cookie("/v1/summary", &owner.cookie).await;
    assert_eq!(
        dec(&proj.json()["financial_health"]["expense_total_monthly_equivalent"]),
        1000.0,
        "la ruta de lectura no se entera de ningún override ni de ningún suelo"
    );
}

#[tokio::test]
async fn every_side_echoes_the_context_that_produced_it() {
    // Issue #27 §8. Seis de estos valores ya se calculaban dentro de la simulación y se tiraban.
    // Sin ellos, respuestas correctas se leen como fallos: un `fire_target_base_delta: 0` es
    // exacto en modo `manual` e inexplicable sin saber el modo.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);

    // El horizonte deja de ser un número a ciegas: dice de dónde sale.
    assert_eq!(
        sim["horizon_basis"], "lifespan_age",
        "el owner tiene fecha de nacimiento: horizonte hasta la edad límite configurada: {sim}"
    );
    assert_eq!(
        sim["horizon_lifespan_age"], 90,
        "la edad límite por defecto viaja al lado del basis (#149): {sim}"
    );

    for lado in ["baseline", "scenario"] {
        let k = &sim[lado];
        // Cuadra con la MISMA superficie que ya publicaba estos hechos.
        assert_eq!(k["savings_source"], "budget", "{lado}: {k}");
        assert_eq!(k["fire_number_mode"], "annual_expense", "{lado}: {k}");
        assert_eq!(k["savings_income_basis"]["basis"], "budget", "{lado}: {k}");
        assert_eq!(k["savings_expense_basis"]["basis"], "budget", "{lado}: {k}");
        assert_eq!(k["savings_income_basis"]["avg_months"], 0, "{lado}: {k}");
        // Hay target, así que la razón de ausencia va explícitamente a null (no desaparece:
        // es la regla que dejó escrita el auditoría MCP §8).
        assert!(
            k.get("fire_target_absent_reason").is_some_and(|v| v.is_null()),
            "{lado} debe llevar fire_target_absent_reason: null, y lleva {k}"
        );
        // Las tres bases: el presupuesto sembrado, sin la cuota de pasivo (que aquí no hay).
        assert_eq!(dec(&k["expense_base_monthly"]), 1000.0, "{lado}: {k}");
        assert_eq!(dec(&k["income_base_monthly"]), 3000.0, "{lado}: {k}");
        assert_eq!(dec(&k["expense_retirement_base_monthly"]), 1000.0, "{lado}: {k}");
        // `expense_base_monthly` NO es `expense_total_monthly`: aquella excluye el servicio de
        // deuda. Sin pasivos coinciden, y esa coincidencia es la que hay que poder comprobar.
        assert_eq!(dec(&k["expense_total_monthly"]), 1000.0, "{lado}: {k}");
        assert_eq!(dec(&k["income_base_monthly"]), dec(&k["income_monthly"]));
        assert_eq!(dec(&k["swr_pct"]), 3.5, "{lado}: {k}");
        assert_eq!(dec(&k["annual_inflation_percent"]), 0.0, "{lado}: {k}");
    }

    // El eco es POR LADO, y eso solo se demuestra haciéndolos diferir: con un override, el
    // baseline conserva el valor de la instalación y el escenario publica el efectivo. Sin esto,
    // el consumidor no puede saber con qué inflación se calculó lo que está leyendo.
    let con_infl = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"annual_inflation_percent": "3"})),
        )
        .await,
    );
    assert_eq!(dec(&con_infl["baseline"]["annual_inflation_percent"]), 0.0);
    assert_eq!(dec(&con_infl["scenario"]["annual_inflation_percent"]), 3.0);
}

#[tokio::test]
async fn absent_fire_target_says_why_instead_of_going_quiet() {
    // `swr_pct: "0"` es un estado modelado —«jamás», no «conservador»—, pero hasta 4.0.0 producía
    // tres `null` sin causa. Con la razón, el consumidor distingue «no configuraste importe
    // manual» de «tu gasto ya lo cubre la pensión» de «pediste SWR 0».
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(&app, &token, tool_call("simulate_projection", json!({"swr_pct": "0"}))).await,
    );

    assert!(sim["baseline"]["fire_target_absent_reason"].is_null());
    assert!(!sim["baseline"]["fire_target_base"].is_null());

    let esc = &sim["scenario"];
    assert_eq!(
        esc["fire_target_absent_reason"], "swr_not_positive",
        "el escenario debe decir por qué no hay objetivo: {esc}"
    );
    assert!(esc["fire_target_base"].is_null(), "{esc}");
    assert!(esc["jubilacion_month_index"].is_null(), "{esc}");
    assert_eq!(dec(&esc["swr_pct"]), 0.0, "el SWR efectivo se echa de vuelta: {esc}");
    // Los deltas que dependen de los dos lados salen null, y ahora eso se puede explicar.
    assert!(sim["deltas"]["fire_target_base_delta"].is_null());
    assert!(sim["deltas"]["jubilacion_months_delta"].is_null());
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
    // La jubilación se sirve ya legible: índice de mes + fecha civil + edad, y las tres coinciden
    // entre ambas superficies (issue #6 — el consumidor no debe hacer aritmética de calendario).
    assert_eq!(
        sim["baseline"]["jubilacion_date_ymd"], proj["jubilacion_date_ymd"],
        "fecha de jubilación: sim {sim}"
    );
    assert_eq!(
        sim["baseline"]["jubilacion_age"], proj["jubilacion_age"],
        "edad de jubilación: sim {sim}"
    );
    // La respuesta de simulate es AUTOCONTENIDA: trae el ancla sin encadenar get_projection.
    assert_eq!(sim["anchor_date_ymd"], proj["anchor_date_ymd"], "{sim}");
    assert_eq!(sim["show_age_mode"], proj["show_age_mode"], "{sim}");
    assert_eq!(sim["viewer_birth_date"], proj["viewer_birth_date"], "{sim}");
    // Y la fecha es coherente con el índice: ancla + N meses, conservando el día del ancla.
    if let Some(mi) = sim["baseline"]["jubilacion_month_index"].as_u64() {
        let anchor = sim["anchor_date_ymd"].as_str().unwrap();
        let date = sim["baseline"]["jubilacion_date_ymd"].as_str().unwrap();
        let (ay, am): (i32, u32) = (anchor[0..4].parse().unwrap(), anchor[5..7].parse().unwrap());
        let zero = ay * 12 + am as i32 - 1 + mi as i32;
        let expected_ym = format!("{:04}-{:02}", zero.div_euclid(12), zero.rem_euclid(12) + 1);
        assert_eq!(&date[0..7], expected_ym, "ancla {anchor} + {mi} meses");
        // El día se conserva salvo recorte a fin de mes; con ancla ≤ 28 el recorte nunca aplica,
        // así que el assert no depende del día en que corra el test. El clamp está pinneado en
        // los tests unitarios de `jubilacion_civil`.
        if anchor[8..10].parse::<u32>().unwrap() <= 28 {
            assert_eq!(&date[8..10], &anchor[8..10], "conserva el día del ancla");
        }
    }
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
        // `extra_monthly_expense` NO está aquí: desde 4.0.0 admite signo (auditoría de simulate_projection §1). Los dos
        // ejes de caja siguen exigiendo `>= 0`, y esas dos filas son las que prueban que la
        // relajación fue POR EJE y no una apertura del helper compartido.
        (json!({"extra_monthly_cash_adjustment": "-5"}), ">= 0"),
        (json!({"extra_monthly_savings": "-5"}), ">= 0"),
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
    // El warm-up post-login puebla la cache en background: hay que esperarlo y limpiar, o la
    // aserción de «cache vacía» culpa a `simulate_projection` de lo que hizo el login.
    app.settle_login_warmup(app.installation_id().await).await;

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
            dec(&k["net_recurring_monthly"]),
            dec(&h["net_monthly_equivalent"]),
            "modo {mode}: neto"
        );
        assert_eq!(
            dec(&k["savings_rate"]),
            dec(&h["savings_rate"]),
            "modo {mode}: tasa de ahorro (misma precisión en las dos superficies)"
        );
        // El eco del contexto (auditoría de simulate_projection §8) tiene que decir lo MISMO que la superficie que ya
        // publicaba estos hechos: si divergieran, tendríamos dos verdades sobre qué modo se usó.
        assert_eq!(
            k["savings_source"], h["savings_source"],
            "modo {mode}: la fuente efectiva echada debe ser la de /v1/summary"
        );
        assert_eq!(
            k["savings_income_basis"], h["savings_income_basis"],
            "modo {mode}: base de ingreso"
        );
        assert_eq!(
            k["savings_expense_basis"], h["savings_expense_basis"],
            "modo {mode}: base de gasto"
        );

        // Identidad interna del propio KPI, en los tres modos.
        assert_eq!(
            dec(&k["net_recurring_monthly"]),
            dec(&k["income_monthly"]) - dec(&k["expense_total_monthly"]),
            "modo {mode}: net_recurring = income − expense_total"
        );
        // Sin overrides de caja el ajuste es 0 en los DOS lados, así que las dos cifras coinciden.
        assert_eq!(dec(&k["monthly_cash_adjustment"]), 0.0, "modo {mode}: sin ajuste");
        assert_eq!(
            dec(&k["net_cash_monthly"]),
            dec(&k["net_recurring_monthly"]),
            "modo {mode}: sin ajuste, caja = recurrente"
        );

        // INVERTIDO en 4.8.0 (#142, opción 3): la cuota es servicio de deuda REAL en los TRES
        // modos (en B/C el gasto efectivo ya la restó del promedio — contarla aquí es contarla
        // UNA vez). El literal `included_in_real_expense` se retiró con su modo.
        assert_eq!(
            dec(&k["debt_service_monthly"]),
            400.0,
            "modo {mode}: la cuota viaja en debt_service_monthly"
        );
        assert!(k["debt_service_absent_reason"].is_null(), "modo {mode}: {k}");

        // Sin overrides, todos los deltas de salud son cero.
        let d = &sim["deltas"];
        for field in [
            "income_monthly_delta",
            "expense_total_monthly_delta",
            "net_recurring_monthly_delta",
            "net_cash_monthly_delta",
            "savings_rate_delta",
        ] {
            assert_eq!(dec(&d[field]), 0.0, "modo {mode}: {field} sin overrides");
        }
    }
}

/// REGRESIÓN (auditoría MCP §7) — ningún importe sale con más de 4 decimales, ni como `-0`.
///
/// El engine capitaliza con `annual_factor.powd(1/12)` (raíz duodécima irracional) y el target FIRE
/// sale de `gross / (swr/100)`; ninguna de las dos se redondeaba, así que la escala saturaba en los
/// ~28 dígitos de `rust_decimal` y al wire salían `"69946992.976753373554690255548"` y
/// `"1148456.9620253164556962025316"`. Ruido, tokens, y un LLM presentando precisión falsa.
///
/// El barrido es estructural a propósito: comprueba **todos** los strings decimales del payload,
/// no una lista de campos. Un campo nuevo que se olvide de redondear cae aquí solo.
#[tokio::test]
async fn no_money_string_carries_more_than_four_decimals_or_negative_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    /// ¿Es este string un decimal? (Los ymd y los enums no lo son.)
    fn decimal_str(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            && s.chars().any(|c| c.is_ascii_digit())
            && !s.contains("--")
    }

    /// Escalas distintas de 4 que son POLÍTICA, no descuido (`.claude/api-routes.md`, 3.8.0): los
    /// ratios van a 6 dp porque no son dinero, y el runway a 1. Se nombran una a una: si mañana
    /// aparece un campo con 6 decimales que sí es un importe, el barrido lo caza.
    fn ratio_field(key: &str) -> bool {
        key.ends_with("savings_rate")
            || key.ends_with("savings_rate_delta")
            || key.ends_with("debt_to_assets_ratio")
    }

    fn walk(v: &serde_json::Value, path: &str, bad: &mut Vec<String>) {
        if ratio_field(path) {
            return;
        }
        match v {
            serde_json::Value::String(s) if decimal_str(s) => {
                if let Some((_, frac)) = s.split_once('.') {
                    if frac.len() > 4 {
                        bad.push(format!("{path} = {s} ({} decimales)", frac.len()));
                    }
                }
                // `impl Neg for Decimal` voltea el bit de signo también sobre el cero.
                if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') {
                    bad.push(format!("{path} = {s} (cero negativo)"));
                }
            }
            serde_json::Value::Array(a) => {
                for (i, item) in a.iter().enumerate() {
                    walk(item, &format!("{path}[{i}]"), bad);
                }
            }
            serde_json::Value::Object(o) => {
                for (k, item) in o {
                    walk(item, &format!("{path}.{k}"), bad);
                }
            }
            _ => {}
        }
    }

    let mut bad = Vec::new();
    for (tool, args) in [
        ("simulate_projection", json!({})),
        ("simulate_projection", json!({"extra_monthly_expense": "300"})),
        ("get_projection", json!({})),
        ("get_summary", json!({})),
        ("get_transactions_summary", json!({})),
    ] {
        let out = tool_json(&mcp_post(&app, &token, tool_call(tool, args.clone())).await);
        walk(&out, tool, &mut bad);
    }
    assert!(bad.is_empty(), "importes fuera de escala:\n  {}", bad.join("\n  "));
}

/// Los hitos son umbrales redondos y se publican con una sola forma.
///
/// `2.5 × 10⁴` heredaba la escala 1 del literal, así que el mismo array mezclaba `"25000.0"`,
/// `"50000"` y `"100000"` (auditoría MCP §7).
#[tokio::test]
async fn projection_milestones_share_one_format() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    let out = tool_json(&mcp_post(&app, &token, tool_call("get_projection", json!({}))).await);
    let ms = out["milestones"].as_array().expect("milestones");
    assert!(!ms.is_empty(), "sin hitos que comprobar: {out}");
    for m in ms {
        let target = m["target"].as_str().expect("target string");
        assert!(
            !target.contains('.'),
            "un hito es un umbral redondo, no debería llevar decimales: {target}"
        );
    }
}

/// REGRESIÓN (auditoría MCP §8) — la jubilación se publica como `null`, no desapareciendo; y
/// `fire_target_series` es paralela a `points` en las dos densidades.
///
/// Con `skip_serializing_if` el campo se esfumaba cuando el horizonte no alcanzaba el objetivo, así
/// que un consumidor no podía distinguir «no se alcanza» de «esta versión no lo publica» — y la
/// descripción de la tool lo prometía sin condiciones, mientras `simulate_projection` sí devolvía
/// `null` para el mismo dato.
///
/// La segunda mitad fija lo que antes se cumplía por casualidad: `fire_target_series` no lleva
/// `month_index` propio, el consumidor la alinea **por posición**, y hasta 4.0.0 se construía con
/// un `map` sobre los índices mientras `points` usaba `filter_map`. Coincidían solo porque
/// `density_month_indices` nunca emite un índice fuera de rango.
#[tokio::test]
async fn retirement_fields_are_explicit_null_and_fire_series_is_parallel_to_points() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    // Horizonte corto: no se alcanza el objetivo FIRE.
    let short = tool_json(
        &mcp_post(&app, &token, tool_call("get_projection", json!({"months": 12}))).await,
    );
    for field in [
        "jubilacion_month_index",
        "jubilacion_date_ymd",
        "jubilacion_age",
    ] {
        assert!(
            short.get(field).is_some(),
            "`{field}` debe viajar aunque no haya cruce, como `null`: {short}"
        );
        assert!(short[field].is_null(), "`{field}` debería ser null: {short}");
    }

    // Y el paralelismo, en las dos densidades (la tool fuerza hybrid; monthly va por HTTP).
    for (label, body) in [
        ("mcp/hybrid", short.clone()),
        (
            "mcp/hybrid-default",
            tool_json(&mcp_post(&app, &token, tool_call("get_projection", json!({}))).await),
        ),
        (
            "http/monthly",
            app.get_with_cookie("/v1/projection/series?density=monthly", &owner.cookie)
                .await
                .json(),
        ),
        (
            "http/hybrid",
            app.get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
                .await
                .json(),
        ),
    ] {
        let points = body["points"].as_array().expect("points").len();
        let fire = body["fire_target_series"].as_array().expect("series").len();
        assert_eq!(fire, points, "{label}: la serie FIRE debe ser paralela a points");
        for a in body["asset_series"].as_array().expect("asset_series") {
            assert_eq!(
                a["values"].as_array().expect("values").len(),
                points,
                "{label}: los valores por activo también van paralelos a points"
            );
        }
    }
}

/// 5.0.0 (§D) — **`view: "household"` es 400 `household_not_simulable`.**
///
/// El hogar dejó de ser «las mismas cuentas con más filas»: es la SUMA de N simulaciones, una por
/// miembro y con la estrategia de cada uno. Un what-if sobre eso no tiene un plan único que mover
/// —¿el SWR de quién?, ¿el gasto extra de quién?—, así que devolver «algo» sería publicar un
/// escenario que no describe el plan de nadie. Se rechaza en el CORE (no en la capa MCP) para que
/// HTTP y MCP no puedan discrepar el día que haya ruta.
#[tokio::test]
async fn household_view_is_refused_with_a_typed_error() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(
        &app,
        &token,
        tool_call(
            "simulate_projection",
            json!({"view": "household", "extra_monthly_savings": "100"}),
        ),
    )
    .await;
    assert_eq!(
        resp["result"]["isError"], true,
        "el hogar no se simula: {resp}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("household_not_simulable"),
        "el error debe nombrar su código para que el agente pueda reaccionar: {text}"
    );

    // Y el default (`mine` desde R2) sigue simulando: lo que se rechaza es el hogar, no la tool.
    let ok = mcp_post(
        &app,
        &token,
        tool_call("simulate_projection", json!({"extra_monthly_savings": "100"})),
    )
    .await;
    let body = tool_json(&ok);
    assert_eq!(body["view"], "mine", "el default es mine: {body}");
    assert_eq!(body["baseline"]["strategy"], "asap", "eco de estrategia: {body}");
    assert_eq!(
        body["scenario"]["retirement_trigger"], "liquid_crossing",
        "eco del trigger: {body}"
    );
    // R8 en el what-if: con `asap` el mes efectivo ES el cruce, así que los dos campos coinciden.
    assert_eq!(
        body["baseline"]["jubilacion_month_index"], body["baseline"]["liquid_crossing_month_index"],
        "con asap el cruce es el trigger: {body}"
    );
}

/// **P11 — el crecimiento del ingreso y los escalones son ejes de CAJA, y se recortan (o no) en
/// la jubilación** (5.0.0, D30; solo MCP).
///
/// Lo que este test clava, en el orden en que se puede equivocar:
///
/// 1. **Es caja, no ingreso**: `income_monthly`, `net_recurring_monthly` y `savings_rate` salen
///    con delta 0 EXACTO. Si el eje entrara por `income_regular_monthly`, movería a la vez el
///    capital y el OBJETIVO (modo `current_income`) y el delta no significaría nada.
/// 2. **Mueve el resultado**: patrimonio final arriba y jubilación no más tarde.
/// 3. **El corte en la jubilación se publica** (`income_growth_stops_at_month_index`) — es el
///    número que hace medible la aproximación de la doble pasada.
/// 4. **Los escalones NO se recortan**: un escalón negativo lejano llega igual.
/// 5. **Anti-no-op**: un `0` en cualquiera de los dos es un 400, no un escenario mudo idéntico
///    al baseline (precedente `liability_override_empty`).
#[tokio::test]
async fn income_growth_and_steps_are_cash_axes_with_a_published_cut() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    let base = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    assert!(
        base["scenario"]["income_growth_stops_at_month_index"].is_null(),
        "sin el eje no hay corte que publicar: {base}"
    );

    let grown = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"income_growth_real_pct_annual": "2"}),
            ),
        )
        .await,
    );

    // 1. Es caja: el trío ingreso/neto recurrente/tasa de ahorro NO se mueve.
    assert_eq!(dec(&grown["deltas"]["income_monthly_delta"]), 0.0, "{grown}");
    assert_eq!(dec(&grown["deltas"]["net_recurring_monthly_delta"]), 0.0, "{grown}");
    assert_eq!(
        grown["scenario"]["savings_rate"], grown["baseline"]["savings_rate"],
        "la tasa de ahorro es sobre el neto RECURRENTE: {grown}"
    );
    // …pero el objetivo tampoco se mueve: el eje no reescribe la meta por la puerta de atrás.
    assert_eq!(
        grown["scenario"]["fire_target_base"], grown["baseline"]["fire_target_base"],
        "{grown}"
    );

    // 2. Y sin embargo el resultado sí cambia: más caja compuesta durante décadas.
    assert!(
        dec(&grown["deltas"]["final_net_worth_delta"]) > 0.0,
        "un 2 % real anual durante el horizonte tiene que dejar más patrimonio: {grown}"
    );
    let b = grown["baseline"]["jubilacion_month_index"].as_u64();
    let sc = grown["scenario"]["jubilacion_month_index"].as_u64();
    if let (Some(b), Some(sc)) = (b, sc) {
        assert!(sc <= b, "más ingreso no puede retrasar la jubilación: {sc} vs {b}");
    }

    // 3. El corte se publica, y solo en el escenario.
    assert!(
        grown["baseline"]["income_growth_stops_at_month_index"].is_null(),
        "el baseline no lleva el eje: {grown}"
    );
    let stop = grown["scenario"]["income_growth_stops_at_month_index"]
        .as_u64()
        .expect("el escenario publica dónde para el crecimiento");
    let horizon = grown["horizon_months"].as_u64().unwrap();
    assert!(stop <= horizon, "el corte cae dentro del horizonte: {stop} vs {horizon}");

    // 4. Un escalón cae donde se le dice, y `date` y `month_index` son el MISMO eje que el
    //    one-off (mes 1 = el mes civil del ancla).
    let anchor = base["anchor_date_ymd"].as_str().unwrap();
    let (y, m) = (
        anchor[0..4].parse::<i32>().unwrap(),
        anchor[5..7].parse::<u32>().unwrap(),
    );
    let (ty, tm) = if m + 3 > 12 { (y + 1, m + 3 - 12) } else { (y, m + 3) };
    let by_date = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"income_steps": [{"date": format!("{ty:04}-{tm:02}-15"), "delta_monthly": "500"}]}),
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
                json!({"income_steps": [{"month_index": 4, "delta_monthly": "500"}]}),
            ),
        )
        .await,
    );
    assert_eq!(by_date["deltas"], by_index["deltas"], "date ≡ month_index");
    assert!(
        dec(&by_date["deltas"]["final_net_worth_delta"]) > 0.0,
        "+500 €/mes desde el mes 4 tiene que dejar más patrimonio: {by_date}"
    );
    assert!(
        by_date["scenario"]["income_growth_stops_at_month_index"].is_null(),
        "los escalones no llevan corte: no se recortan en la jubilación: {by_date}"
    );
    // Un escalón NEGATIVO resta, y los escalones se acumulan entre sí.
    let down = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"income_steps": [
                    {"month_index": 4, "delta_monthly": "500"},
                    {"month_index": 60, "delta_monthly": "-500"}
                ]}),
            ),
        )
        .await,
    );
    assert!(
        dec(&down["deltas"]["final_net_worth_delta"])
            < dec(&by_date["deltas"]["final_net_worth_delta"]),
        "quitar el escalón a los 5 años deja MENOS que mantenerlo: {down}"
    );

    // 5. Anti-no-op y cotas: cada uno con su código estable.
    for (body, needle) in [
        (json!({"income_growth_real_pct_annual": "0"}), "income_growth_no_op"),
        (json!({"income_growth_real_pct_annual": "25"}), "income_growth_out_of_range"),
        (json!({"income_growth_real_pct_annual": "-11"}), "income_growth_out_of_range"),
        (
            json!({"income_steps": [{"month_index": 4, "delta_monthly": "0"}]}),
            "income_step_delta_zero",
        ),
        (
            json!({"income_steps": [{"delta_monthly": "100"}]}),
            "income_step_timing_ambiguous",
        ),
        (
            json!({"income_steps": [{"month_index": 4, "date": "2030-01-01", "delta_monthly": "100"}]}),
            "income_step_timing_ambiguous",
        ),
        (
            json!({"months": 120, "income_steps": [{"month_index": 500, "delta_monthly": "100"}]}),
            "income_step_month_out_of_range",
        ),
    ] {
        let envelope = mcp_post(&app, &token, tool_call("simulate_projection", body.clone())).await;
        let result = &envelope["result"];
        assert_eq!(result["isError"], true, "debía fallar con {body}: {envelope}");
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(needle), "{body} debe nombrar «{needle}» y dice: {text}");
    }
}


// ---------------------------------------------------------------------------------------------
// 5.0.0 WP5-2b — el PLAN como eje what-if (§E: P5, P8.b, P8.c)
// ---------------------------------------------------------------------------------------------

/// **P5 — «¿y si me jubilo a los 55?»**: `profile_overrides` cambia la estrategia entera sobre un
/// CLON del perfil, y la respuesta trae por lado lo que ese plan cuesta.
///
/// El baseline es `asap` (el default): se jubila por CRUCE y no tiene edad contra la que resolver
/// nada, así que sus KPIs de plan salen `null` — que NO es cero: es «esta estrategia no responde
/// a esa pregunta», y por eso los deltas también son `null` en vez de restar contra un hueco.
/// El escenario es `retire_at_age` a los 55: dispara por edad y publica el ahorro necesario, su
/// techo y el margen.
///
/// Predicho a mano con el hogar del `seed` (ingreso 3.000, gasto 1.000, 50.000 € al 5 %): el
/// sobrante mensual es **2.000 €** y no cambia en todo el horizonte, así que el techo de búsqueda
/// del escenario es exactamente `"2000.0000"`.
#[tokio::test]
async fn profile_overrides_simulate_the_whole_plan_and_publish_what_it_costs() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await; // nace 1990-01-01
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"profile_overrides": {"strategy": "retire_at_age",
                                             "target_retirement_age": 55}}),
            ),
        )
        .await,
    );

    // El eco por lado: sin él, un `jubilacion_months_delta` no distingue «el eje no movió nada»
    // de «la fecha la fija la edad, no el capital».
    assert_eq!(out["baseline"]["strategy"], "asap", "{out}");
    assert_eq!(out["baseline"]["retirement_trigger"], "liquid_crossing", "{out}");
    assert_eq!(out["scenario"]["strategy"], "retire_at_age", "{out}");
    assert_eq!(out["scenario"]["retirement_trigger"], "target_age", "{out}");

    // El baseline se dispara por cruce: no hay `R` contra el que resolver, y eso es `null`.
    for k in [
        "required_contribution_monthly",
        "required_contribution_search_ceiling",
        "underfunded",
        "disposable_monthly",
    ] {
        assert!(
            out["baseline"][k].is_null(),
            "`asap` no responde a {k}, y `null` no es cero: {out}"
        );
        assert!(
            !out["scenario"][k].is_null(),
            "`retire_at_age` sí lo responde: {k} en {out}"
        );
    }
    assert_eq!(
        out["scenario"]["required_contribution_search_ceiling"], "2000.0000",
        "3.000 − 1.000 = 2.000 €/mes de sobrante, constante: {out}"
    );
    // Un delta contra un «no aplica» sería un número inventado.
    assert!(out["deltas"]["required_contribution_monthly_delta"].is_null(), "{out}");
    assert!(out["deltas"]["disposable_monthly_delta"].is_null(), "{out}");
    // El margen es exactamente lo que sobra del techo.
    let techo = dec(&out["scenario"]["required_contribution_search_ceiling"]);
    let c = dec(&out["scenario"]["required_contribution_monthly"]);
    let margen = dec(&out["scenario"]["disposable_monthly"]);
    assert!((margen - (techo - c)).abs() < 0.0001, "{out}");
}

/// **`fire_number_mode` y `fire_number_manual_amount` vuelven a ser simulables** por
/// `profile_overrides`. WP4 los sacó de `fire_settings_overrides` al mudarlos al perfil (D13) y
/// se quedaron una ola sin eje what-if; esto es la regresión que impide que vuelva a pasar.
///
/// De paso pinea lo que el nombre del campo NO dice: `fire_number_manual_amount` es la
/// **necesidad ANUAL neta** en euros de hoy, no el capital objetivo — el mismo papel que
/// `12·gasto` en `annual_expense` (`netAnnualNeed` de `apps/web/src/lib/fire.ts` y
/// `FireNeed::Indexed` del motor coinciden en eso). Con impuestos fuera y el SWR por defecto:
///
/// ```text
/// objetivo = gross_up(40.000) / (3,5/100) = 40.000 / 0,035 = 1.142.857,1429 €
/// ```
#[tokio::test]
async fn the_fire_number_mode_axis_comes_back_through_profile_overrides() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxes_enabled": false}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "taxes off: {r:?}");
    let token = create_token(&app, &owner).await;

    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"profile_overrides": {"fire_number_mode": "manual",
                                             "fire_number_manual_amount": "40000"}}),
            ),
        )
        .await,
    );
    assert_eq!(out["baseline"]["fire_number_mode"], "annual_expense", "{out}");
    assert_eq!(out["scenario"]["fire_number_mode"], "manual", "{out}");
    let base = dec(&out["scenario"]["fire_target_base"]);
    assert!(
        (base - 1_142_857.1429).abs() < 0.01,
        "40.000 €/año declarados / 3,5 % = 1.142.857,14 €, llegó {base}: {out}"
    );
    // El baseline sale del gasto (12 × 1.000 / 3,5 %), así que el delta existe y es explicable.
    let baseline_base = dec(&out["baseline"]["fire_target_base"]);
    assert!(
        (baseline_base - 342_857.1429).abs() < 0.01,
        "12·1.000 / 3,5 % = 342.857,14 €, llegó {baseline_base}: {out}"
    );
    assert!(
        !out["deltas"]["fire_target_base_delta"].is_null(),
        "los dos lados tienen objetivo, así que el delta existe: {out}"
    );
}

/// **P8.c — la excedencia**: `income_pause` multiplica el ingreso GANADO durante su ventana, y la
/// respuesta publica los dos meses de jubilación y su diferencia.
///
/// Que el retraso sea `>= 0` no es una tautología: la pausa quita caja que la cascada habría
/// invertido, así que el cruce solo puede llegar igual o más tarde. Y el mes con pausa que
/// publica `income_pause` tiene que ser EL MISMO que el del escenario, porque el escenario lleva
/// la pausa aplicada — si divergieran, el KPI describiría una simulación que no se sirvió.
#[tokio::test]
async fn an_income_pause_delays_retirement_and_publishes_both_months() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"income_pause": {"from_month_index": 13, "months": 12,
                                        "income_fraction": "0"}}),
            ),
        )
        .await,
    );
    let pause = &out["income_pause"];
    assert!(!pause.is_null(), "se pidió el eje: {out}");
    let base = pause["baseline_month_index"].as_u64().expect("mes base");
    let paused = pause["paused_month_index"].as_u64().expect("mes con pausa");
    let delay = pause["retirement_delay_months"].as_i64().expect("retraso");
    assert_eq!(delay, paused as i64 - base as i64, "{out}");
    assert!(delay > 0, "un año sin cobrar retrasa la jubilación: {out}");
    assert_eq!(
        out["scenario"]["jubilacion_month_index"].as_u64(),
        Some(paused),
        "el escenario servido ES el que lleva la pausa: {out}"
    );
    assert_eq!(
        out["baseline"]["jubilacion_month_index"].as_u64(),
        Some(base),
        "sin otros overrides, el lado sin pausa coincide con el baseline: {out}"
    );

    // La ventana es SEMIABIERTA y media paga retrasa menos que no cobrar nada.
    let half = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"income_pause": {"from_month_index": 13, "months": 12,
                                        "income_fraction": "0.5"}}),
            ),
        )
        .await,
    );
    assert!(
        half["income_pause"]["retirement_delay_months"].as_i64().unwrap() <= delay,
        "media paga no puede retrasar MÁS que no cobrar: {half}"
    );

    // Sin el eje, el bloque no aparece: «no se preguntó» no puede leerse como «no hay retraso».
    let sin = tool_json(
        &mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await,
    );
    assert!(sin.get("income_pause").is_none(), "{sin}");
}

/// **P8.b — «¿cuánto más puedo gastar sin mover la fecha?»**, opt-in porque cuesta una bisección
/// entera sobre el motor.
///
/// Con el trigger por CRUCE (el `asap` del baseline) la respuesta es un margen real y acotado:
/// gastar más retrasa el cruce, así que el solve encuentra un punto por debajo del sobrante
/// entero (2.000 €/mes). Es lo contrario del trigger por EDAD, donde la fecha no depende del
/// gasto y la respuesta es el sobrante entero como SUELO honesto — el otro caso, comprobado
/// abajo con el mismo hogar.
#[tokio::test]
async fn the_extra_expense_solve_answers_how_much_more_fits_without_moving_the_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let cruce = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"solve": {"extra_monthly_expense_keeping_date": true}}),
            ),
        )
        .await,
    );
    let margen = dec(&cruce["max_extra_monthly_expense_keeping_date"]);
    assert!(
        (0.0..=2000.0).contains(&margen),
        "con trigger por cruce el margen cabe dentro del sobrante (2.000 €/mes), llegó {margen}: {cruce}"
    );

    // Con la EDAD mandando, el gasto no mueve la fecha: la respuesta es el sobrante entero, un
    // suelo («al menos esto»), no un infinito inventado.
    let edad = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"profile_overrides": {"strategy": "retire_at_age",
                                             "target_retirement_age": 55},
                       "solve": {"extra_monthly_expense_keeping_date": true}}),
            ),
        )
        .await,
    );
    assert_eq!(
        edad["max_extra_monthly_expense_keeping_date"], "2000.0000",
        "la edad no depende del gasto: el solve devuelve el techo entero como suelo: {edad}"
    );

    // Sin pedirlo, el campo no aparece.
    let sin = tool_json(
        &mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await,
    );
    assert!(
        sin.get("max_extra_monthly_expense_keeping_date").is_none(),
        "{sin}"
    );
}

/// **Anti-no-op de los ejes nuevos**: cada llamada que no puede mover nada devuelve un 400 con su
/// código estable, nunca un escenario idéntico al baseline sin explicación.
#[tokio::test]
async fn the_new_what_if_axes_refuse_calls_that_cannot_move_anything() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    for (body, needle) in [
        (json!({"profile_overrides": {}}), "profile_overrides_empty"),
        // Un patch que resuelve al perfil que ya tienes: el escenario saldría idéntico.
        (
            json!({"profile_overrides": {"strategy": "asap"}}),
            "profile_overrides_no_op",
        ),
        // El mismo eje por dos caminos a la vez es una intención contradictoria.
        (
            json!({"swr_pct": "3", "profile_overrides": {"swr_pct": "3"}}),
            "swr_pct_set_twice",
        ),
        // Las cotas del perfil son las MISMAS que al guardarlo: `retire_at_age` exige la edad.
        (
            json!({"profile_overrides": {"strategy": "retire_at_age"}}),
            "target_retirement_age_required",
        ),
        (
            json!({"income_pause": {"months": 12, "income_fraction": "0"}}),
            "income_pause_timing_ambiguous",
        ),
        (
            json!({"income_pause": {"from_month_index": 3, "from_date": "2030-01-01",
                                    "months": 12, "income_fraction": "0"}}),
            "income_pause_timing_ambiguous",
        ),
        (
            json!({"income_pause": {"from_month_index": 3, "months": 12,
                                    "income_fraction": "1"}}),
            "income_pause_fraction_out_of_range",
        ),
        (
            json!({"months": 120,
                   "income_pause": {"from_month_index": 500, "months": 12,
                                    "income_fraction": "0"}}),
            "income_pause_month_out_of_range",
        ),
        (
            json!({"solve": {"extra_monthly_expense_keeping_date": false}}),
            "solve_no_op",
        ),
    ] {
        let envelope = mcp_post(&app, &token, tool_call("simulate_projection", body.clone())).await;
        let result = &envelope["result"];
        assert_eq!(result["isError"], true, "debía fallar con {body}: {envelope}");
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(needle), "{body} debe nombrar «{needle}» y dice: {text}");
    }
}

// ---------------------------------------------------------------------------------------------
// El eje `monte_carlo` (5.0.0 WP6b, P3)
// ---------------------------------------------------------------------------------------------

/// **Un eje que AÑADE información en vez de mover el escenario.**
///
/// Tres cosas se pinean aquí:
///
/// 1. `monte_carlo` **solo**, con el cuerpo por lo demás vacío, es una petición VÁLIDA. Todos los
///    demás ejes tienen anti-no-op porque devolverían un escenario idéntico al baseline sin decir
///    por qué; éste no cambia el escenario, lo describe, y «¿qué probabilidad tiene mi plan tal
///    cual está?» es una pregunta legítima.
/// 2. Los cuatro campos aparecen **en los dos lados** y `success_probability_delta` en `deltas`.
/// 3. Sin el eje, los cuatro son `null` — no 0. Un cero se leería como «ningún escenario aguanta».
#[tokio::test]
async fn the_monte_carlo_axis_adds_probabilities_to_both_sides_without_moving_the_scenario() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    // Sin el eje: los cuatro campos van a `null` y no hay bloque `monte_carlo`.
    let sin = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    for lado in ["baseline", "scenario"] {
        for k in [
            "success_probability",
            "success_verdict",
            "underfunded_probability",
            "months_below_need_p50",
        ] {
            assert!(sin[lado][k].is_null(), "{lado}.{k} sin el eje: {sin}");
        }
    }
    assert!(sin["deltas"]["success_probability_delta"].is_null(), "{sin}");
    assert!(sin["monte_carlo"].is_null(), "sin el eje no hay bloque: {sin}");

    // Con el eje SOLO (sin ningún otro override): válido, y con las cuatro cifras en ambos lados.
    let con = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({"monte_carlo": {"paths": 24, "seed": "7"}}),
            ),
        )
        .await,
    );
    assert_eq!(con["monte_carlo"]["paths"], 24, "{con}");
    assert_eq!(
        con["monte_carlo"]["seed"], "7",
        "la semilla se ecoa como STRING: {con}"
    );
    assert_eq!(con["monte_carlo"]["success_threshold_pct"], 95, "{con}");
    for lado in ["baseline", "scenario"] {
        assert!(
            con[lado]["success_probability"].is_string(),
            "{lado} debe traer la probabilidad: {con}"
        );
        assert!(con[lado]["success_verdict"].is_string(), "{lado}: {con}");
        assert!(con[lado]["months_below_need_p50"].is_u64(), "{lado}: {con}");
        // Sin trigger por edad, la infra-financiación no aplica ni con el eje pedido.
        assert!(con[lado]["underfunded_probability"].is_null(), "{lado}: {con}");
    }
    // Sin ningún otro override los dos lados son el MISMO plan y la MISMA semilla ⇒ delta 0 exacto.
    assert_eq!(
        con["baseline"]["success_probability"], con["scenario"]["success_probability"],
        "misma semilla y mismo plan: las dos columnas deben coincidir: {con}"
    );
    assert_eq!(
        dec(&con["deltas"]["success_probability_delta"]),
        0.0,
        "{con}"
    );
    // Y el escenario sigue siendo el baseline: el eje no mueve NADA del plan.
    assert_eq!(
        con["deltas"]["jubilacion_months_delta"], 0,
        "el eje describe, no simula otra cosa: {con}"
    );

    // **No hay bandas en simulate**: son ~16 KB por lado y el fan chart vive en su endpoint.
    for lado in ["baseline", "scenario"] {
        assert!(con[lado]["points"].is_null(), "{lado} no lleva bandas: {con}");
    }
}

/// Con una estrategia por EDAD aparece `underfunded_probability`, y un plan que no llega la
/// publica > 0. Es el rojo de D17 en versión probabilística: «cuántos de estos mercados te dejan
/// llegar a los 55 sin el capital que necesitas».
#[tokio::test]
async fn an_age_strategy_publishes_the_underfunded_probability() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "simulate_projection",
                json!({
                    "monte_carlo": {"paths": 24},
                    "profile_overrides": {"strategy": "retire_at_age", "target_retirement_age": 45},
                }),
            ),
        )
        .await,
    );
    assert_eq!(out["scenario"]["retirement_trigger"], "target_age", "{out}");
    assert!(
        out["scenario"]["underfunded_probability"].is_string(),
        "con trigger por edad la pregunta existe: {out}"
    );
    // El baseline sigue siendo `asap` (por cruce), así que allí NO aplica — y por eso el campo es
    // `null` en una columna y un número en la otra sin que eso sea una incoherencia.
    assert!(
        out["baseline"]["underfunded_probability"].is_null(),
        "{out}"
    );
}

/// Las cotas del eje: `paths` fuera de `1..=1000` y una semilla que no es un `u64` son 400 con su
/// código. El techo del MCP es la MITAD del de HTTP a propósito (esta tool no cachea nada y un
/// agente en bucle es el llamante que más satura el semáforo).
#[tokio::test]
async fn the_monte_carlo_axis_enforces_its_bounds() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    for (body, needle) in [
        (json!({"monte_carlo": {"paths": 0}}), "paths_out_of_range"),
        (json!({"monte_carlo": {"paths": 1001}}), "paths_out_of_range"),
        (
            json!({"monte_carlo": {"paths": 8, "seed": "no-soy-un-numero"}}),
            "invalid",
        ),
    ] {
        let envelope = mcp_post(&app, &token, tool_call("simulate_projection", body.clone())).await;
        let result = &envelope["result"];
        assert_eq!(result["isError"], true, "debía fallar con {body}: {envelope}");
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(needle), "{body} debe nombrar «{needle}» y dice: {text}");
    }
}
