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
    // Se comprueba sobre el IMPORTE, no sobre el cociente. Desde 5.0.0 el hogar se jubila y
    // DRENA, así que el patrimonio del último mes es de tres cifras: dos importes publicados a la
    // escala monetaria (4 decimales) dan un cociente cuya última cifra ya es el redondeo de
    // publicación, y una tolerancia relativa de 1e-9 sobre él estaría pinneando ese redondeo y no
    // el deflactado. La tolerancia es exactamente la escala con la que se publica.
    let real_esperado = nominal / esperado;
    assert!(
        (real - real_esperado).abs() <= 1e-4,
        "real {real}, esperado {real_esperado} (nominal {nominal} / (1,03)^({meses}/12) = {esperado}): {esc}"
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
                json!({ "op_date": tenth_of_last_month(), "concept": concept, "amount": amount,
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
    assert!(dec(&d["final_net_worth_delta"]) > 0.0, "y deja más patrimonio: {d}");

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
    assert_eq!(dec(&esc["expense_total_monthly"]), 0.0, "{esc}");
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
    // Issue #27 §8. Estos valores ya se calculaban dentro de la simulación y se tiraban. Sin
    // ellos, respuestas correctas se leen como fallos.
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
        // 6.0.0 (D-7): el eco del plan de jubilación se retiró con él.
        for dead in ["fire_number_mode", "swr_pct", "expense_retirement_base_monthly", "strategy"] {
            assert!(k.get(dead).is_none(), "{lado}.{dead}: {k}");
        }
        assert_eq!(k["savings_income_basis"]["basis"], "budget", "{lado}: {k}");
        assert_eq!(k["savings_expense_basis"]["basis"], "budget", "{lado}: {k}");
        assert_eq!(k["savings_income_basis"]["avg_months"], 0, "{lado}: {k}");
        // Las tres bases: el presupuesto sembrado, sin la cuota de pasivo (que aquí no hay).
        assert_eq!(dec(&k["expense_base_monthly"]), 1000.0, "{lado}: {k}");
        assert_eq!(dec(&k["income_base_monthly"]), 3000.0, "{lado}: {k}");
        // `expense_base_monthly` NO es `expense_total_monthly`: aquella excluye el servicio de
        // deuda. Sin pasivos coinciden, y esa coincidencia es la que hay que poder comprobar.
        assert_eq!(dec(&k["expense_total_monthly"]), 1000.0, "{lado}: {k}");
        assert_eq!(dec(&k["income_base_monthly"]), dec(&k["income_monthly"]));
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

    // La respuesta de simulate es AUTOCONTENIDA: trae el ancla sin encadenar get_projection.
    assert_eq!(sim["anchor_date_ymd"], proj["anchor_date_ymd"], "{sim}");
    assert_eq!(sim["show_age_mode"], proj["show_age_mode"], "{sim}");
    assert_eq!(sim["viewer_birth_date"], proj["viewer_birth_date"], "{sim}");
    assert_eq!(sim["horizon_months"], proj["months"]);
    // 6.0.0 (D-7): ni la serie ni el what-if publican ya un plan de jubilación.
    for dead in ["jubilacion_month_index", "success_of_plan", "fire_number_classic_today"] {
        assert!(sim["baseline"].get(dead).is_none(), "{dead}: {sim}");
        assert!(proj.get(dead).is_none(), "{dead}: {proj}");
    }
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

/// Gasto REAL frente a ajuste NEUTRO de caja: los dos restan la misma caja, pero solo el gasto
/// real mueve el gasto total (y con él el neto recurrente). 6.0.0 (D-7): sin objetivo FIRE ni
/// fecha de jubilación, la diferencia se lee en los KPIs de flujo, no en una fecha.
#[tokio::test]
async fn real_expense_moves_the_expense_kpis_but_neutral_cash_adjustment_does_not() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    let real = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "200"})),
        )
        .await,
    );
    assert_eq!(dec(&real["deltas"]["expense_total_monthly_delta"]), 200.0, "{real}");
    assert!(dec(&real["deltas"]["final_net_worth_delta"]) < 0.0, "{real}");

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
    assert_eq!(dec(&neutral["deltas"]["expense_total_monthly_delta"]), 0.0, "{neutral}");
    assert_eq!(dec(&neutral["deltas"]["net_cash_monthly_delta"]), -200.0, "{neutral}");
    assert!(dec(&neutral["deltas"]["final_net_worth_delta"]) < 0.0, "{neutral}");

    let savings = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_savings": "300"})),
        )
        .await,
    );
    assert_eq!(dec(&savings["deltas"]["expense_total_monthly_delta"]), 0.0, "{savings}");
    assert!(dec(&savings["deltas"]["final_net_worth_delta"]) > 0.0, "{savings}");
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
    // **El descubierto por mes, por lado** (5.0.0, pase de correcciones de la revisión
    // adversarial): es la única columna que dice DÓNDE deja de cubrirse el plan —
    // `assets_depleted_month_index` da un mes y `uncovered_deficit_total` un total, y entre los
    // dos no se ve el perfil del hueco—. Este hogar ahorra, así que son ceros: un cero de verdad,
    // publicado, no una columna ausente.
    for k in ["baseline_unmet_need", "scenario_unmet_need"] {
        let v = series[k]
            .as_array()
            .unwrap_or_else(|| panic!("falta {k}: {by_date}"));
        assert_eq!(v.len(), n, "{k} comparte rejilla con month_indices: {by_date}");
        assert!(
            v.iter().all(|x| x.as_f64() == Some(0.0)),
            "este hogar no tiene descubierto: {k} = {v:?}"
        );
    }
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
                json!({ "op_date": tenth_of_last_month(), "concept": concept, "amount": amount,
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
            // 5.0.0: las PROBABILIDADES del plan y del sorteo van a 6 dp por la misma razón que
            // los ratios — no son dinero. Se nombran una a una: un importe con 6 decimales que
            // apareciera mañana lo seguiría cazando el barrido.
            || key.ends_with("success_of_plan")
            || key.ends_with("success_of_plan_delta")
            || key.ends_with("success_wilson_low")
            || key.ends_with("success_probability")
            || key.ends_with("probability")
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

/// 6.0.0 (D-7): `get_projection` ya no publica el plan de jubilación (`jubilacion_*`,
/// `plan_absent_reason`, éxito, capital necesario): esos campos NO viajan, ni como `null`. Lo que
/// queda paralelo a `points` es `asset_series`.
#[tokio::test]
async fn retirement_fields_are_gone_and_asset_series_are_parallel_to_points() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    let short = tool_json(
        &mcp_post(&app, &token, tool_call("get_projection", json!({"months": 12}))).await,
    );
    assert_eq!(short["horizon_basis"], "months_override", "{short}");
    for field in [
        "jubilacion_month_index",
        "jubilacion_date_ymd",
        "jubilacion_age",
        "retirement_date_basis",
        "success_of_plan",
        "needed_capital_today",
        "plan_absent_reason",
    ] {
        assert!(short.get(field).is_none(), "`{field}` se retiró en 6.0.0: {short}");
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
        assert!(
            body.get("fire_target_series").is_none(),
            "{label}: la serie del objetivo determinista se retiró en 5.0.0: {body}"
        );
        for a in body["asset_series"].as_array().expect("asset_series") {
            assert_eq!(
                a["values"].as_array().expect("values").len(),
                points,
                "{label}: los valores por activo van paralelos a points"
            );
        }
    }
}

/// 5.0.0 (§D) — **`view: "household"` es 400 `household_not_simulable`.**
///
/// El hogar dejó de ser «las mismas cuentas con más filas»: es la SUMA de N simulaciones, una por
/// miembro. Un what-if sobre eso no tiene un escenario único que mover —¿el gasto extra de
/// quién?—, así que devolver «algo» sería publicar un escenario que no describe a nadie. Se rechaza en el CORE (no en la capa MCP) para que
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
    assert!(body["baseline"].get("strategy").is_none(), "{body}");
}

/// **P11 — el crecimiento del ingreso y los escalones son ejes de CAJA** (5.0.0, D30; solo MCP).
/// 6.0.0 (D-7): sin jubilación no hay corte — el crecimiento aplica a todo el horizonte.
///
/// Lo que este test clava, en el orden en que se puede equivocar:
///
/// 1. **Es caja, no ingreso**: `income_monthly`, `net_recurring_monthly` y `savings_rate` salen
///    con delta 0 EXACTO. Si el eje entrara por `income_regular_monthly`, movería a la vez el
///    capital y el OBJETIVO (modo `current_income`) y el delta no significaría nada.
/// 2. **Mueve el resultado**: patrimonio final arriba.
/// 3. **Los escalones caen donde se les dice**, y un escalón negativo lejano resta.
/// 5. **Anti-no-op**: un `0` en cualquiera de los dos es un 400, no un escenario mudo idéntico
///    al baseline (precedente `liability_override_empty`).
#[tokio::test]
async fn income_growth_and_steps_are_cash_axes_with_a_published_cut() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    seed(&app, &owner).await;

    let base = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);

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

    // 2. Y sin embargo el resultado sí cambia: más caja compuesta durante décadas.
    assert!(
        dec(&grown["deltas"]["final_net_worth_delta"]) > 0.0,
        "un 2 % real anual durante el horizonte tiene que dejar más patrimonio: {grown}"
    );
    // 3. Un escalón cae donde se le dice, y `date` y `month_index` son el MISMO eje que el
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

    // 4. Anti-no-op y cotas: cada uno con su código estable.
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
// `income_pause` (5.0.0 WP5-2b) — sigue siendo un eje what-if sin jubilación (6.0.0, D-7)
// ---------------------------------------------------------------------------------------------

/// Una pausa de ingreso cuesta patrimonio, y sus cotas son 400 con código estable.
#[tokio::test]
async fn an_income_pause_costs_net_worth_and_enforces_its_bounds() {
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
    assert!(
        dec(&out["deltas"]["final_net_worth_delta"]) < 0.0,
        "un año sin ingresos deja menos patrimonio: {out}"
    );
    assert_eq!(
        dec(&out["deltas"]["income_monthly_delta"]),
        0.0,
        "la pausa no reescribe el ingreso recurrente publicado: {out}"
    );

    for (body, needle) in [
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
/// 1. `monte_carlo` **solo**, con el cuerpo por lo demás vacío, es una petición VÁLIDA.
/// 2. Sin el eje, los campos del sorteo son `null`/vacíos — no 0.
/// 3. 6.0.0 (D-7): sin fase de jubilación la probabilidad de éxito viaja a `null` con
///    `success_probability_absent_reason: "no_retirement_phase"` (el motor solo cuenta fallos
///    estando jubilado), en los DOS lados. El reparto por motivo sí se publica.
#[tokio::test]
async fn the_monte_carlo_axis_reports_no_success_without_a_retirement_phase() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let sin = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    for lado in ["baseline", "scenario"] {
        for k in ["success_probability", "sampling_error_pp", "failures_by_kind"] {
            assert!(sin[lado][k].is_null(), "{lado}.{k} sin el eje: {sin}");
        }
        assert!(
            sin[lado].get("failure_probability_by_age").is_none(),
            "{lado}: la tabla no viaja vacía, se omite: {sin}"
        );
        for dead in ["success_of_plan", "success_verdict", "months_below_need_p50"] {
            assert!(sin[lado].get(dead).is_none(), "{lado}.{dead}: {sin}");
        }
    }
    assert!(sin["monte_carlo"].is_null(), "sin el eje no hay bloque: {sin}");

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
    assert_eq!(con["monte_carlo"]["seed"], "7", "la semilla se ecoa como STRING: {con}");
    for lado in ["baseline", "scenario"] {
        let k = &con[lado];
        assert!(k["success_probability"].is_null(), "{lado}: {con}");
        assert_eq!(
            k["success_probability_absent_reason"], "no_retirement_phase",
            "{lado}: {con}"
        );
        let kinds = k["failures_by_kind"]
            .as_array()
            .unwrap_or_else(|| panic!("{lado} debe traer failures_by_kind: {con}"));
        assert_eq!(kinds.len(), 2, "{lado}: [F1, F2]: {con}");
        assert!(con[lado]["points"].is_null(), "{lado} no lleva bandas: {con}");
    }
    assert_eq!(
        dec(&con["deltas"]["final_net_worth_delta"]),
        0.0,
        "el eje describe, no simula otra cosa: {con}"
    );
}

/// Las cotas del eje: `paths` fuera de `1..=2500` y una semilla que no es un `u64` son 400 con su
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
        (json!({"monte_carlo": {"paths": 2501}}), "paths_out_of_range"),
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

/// `YYYY-MM-10` del mes anterior al actual: un mes CERRADO que cae dentro de cualquier ventana de
/// calendario (3 o 12 meses) sea cual sea el día de hoy. Las siembras llevaban «2026-06-10» y el
/// 10 de octubre de 2026 ya no caían en la ventana de 3 meses del override.
fn tenth_of_last_month() -> String {
    use chrono::Datelike;
    let today = chrono::Utc::now().date_naive();
    let (y, m) = if today.month() == 1 { (today.year() - 1, 12) } else { (today.year(), today.month() - 1) };
    format!("{y:04}-{m:02}-10")
}
