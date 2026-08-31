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
        // Modo manual sin importe: lo caza `validate_fire_settings`, no un `if` nuevo.
        (json!({"fire_number_mode": "manual"}), "fire_manual_amount"),
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
