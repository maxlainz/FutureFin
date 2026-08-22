//! Flujo MCP end-to-end sobre `/mcp` (Streamable HTTP, camino stateless 2026-07-28).
//!
//! Congela el contrato del servidor MCP: initialize, catálogo exacto de tools, paridad
//! byte a byte entre una tool y su endpoint HTTP hermano, densidad hybrid fija de la
//! proyección, errores de validación como tool-error legible, y el kill-switch
//! `mcp_enabled=false`.

mod common;

use axum::extract::Extension;
use axum::Router;
use common::{LoggedInOwner, TestApp};
use futurefin_api::routes;
use futurefin_api::state::AppState;
use std::sync::Arc;

const PROTOCOL: &str = "2026-07-28";

/// POST JSON-RPC a `/mcp` en modo stateless y devuelve el JSON del envelope de respuesta
/// (parsea tanto `application/json` directo como frames SSE `data: {...}`).
async fn mcp_post(app: &TestApp, bearer: &str, body: serde_json::Value) -> serde_json::Value {
    let mut builder = http::Request::builder()
        .method(http::Method::POST)
        .uri("/mcp")
        // Host obligatorio para rmcp (el oneshot del harness no lo pone solo).
        .header(http::header::HOST, "futurefin.test")
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL)
        .header(http::header::AUTHORIZATION, format!("Bearer {bearer}"));
    // SEP-2243: bajo 2026-07-28 los requests llevan headers de routing espejo del body.
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

    let content_type = resp
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = String::from_utf8(resp.body.clone()).expect("utf8 body");
    if content_type.starts_with("application/json") {
        return serde_json::from_str(&text).expect("json body");
    }
    // SSE: la respuesta JSON-RPC viaja en un frame `data: {...}`; nos quedamos con el
    // último frame con payload JSON válido.
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
            serde_json::json!({"label": "mcp tests"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

fn initialize_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "futurefin-tests", "version": "0"}
        }
    })
}

/// `_meta` obligatorio por-request del ciclo discover (SEP-2575/2567) en 2026-07-28.
fn request_meta() -> serde_json::Value {
    serde_json::json!({
        "io.modelcontextprotocol/protocolVersion": PROTOCOL,
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

fn tools_list_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {"_meta": request_meta()}
    })
}

fn tool_call_body(name: &str, arguments: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments, "_meta": request_meta()}
    })
}

/// Extrae el JSON del primer content-block de texto de un resultado de tools/call.
fn tool_text_json(envelope: &serde_json::Value) -> serde_json::Value {
    let result = &envelope["result"];
    assert_ne!(
        result["isError"], true,
        "tool devolvió error: {envelope}"
    );
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("sin content de texto: {envelope}"));
    serde_json::from_str(text).expect("content de la tool es JSON")
}

#[tokio::test]
async fn initialize_reports_futurefin_server_info() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, initialize_body()).await;
    assert_eq!(resp["result"]["serverInfo"]["name"], "futurefin", "{resp}");
    assert_eq!(
        resp["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(resp["result"]["capabilities"]["tools"].is_object());
    assert!(resp["result"]["instructions"].is_string());
}

#[tokio::test]
async fn tools_list_returns_exactly_the_v1_catalog() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    let mut names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array: {resp}"))
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "apply_categorization_rule",
            "capture_snapshot",
            "create_asset",
            "create_budget_entry",
            "create_categorization_rule",
            "create_category",
            "create_liability",
            "create_planning_flow",
            "create_transaction",
            "delete_asset",
            "delete_budget_entry",
            "delete_import",
            "delete_liability",
            "delete_planning_flow",
            "delete_recurring_rule",
            "delete_snapshot",
            "delete_transaction",
            "get_allocation_resolution",
            "get_budget",
            "get_category_monthly_series",
            "get_history",
            "get_history_cashflow",
            "get_projection",
            "get_settings",
            "get_summary",
            "get_transactions_summary",
            "list_allocation_rules",
            "list_assets",
            "list_categories",
            "list_categorization_rules",
            "list_liabilities",
            "list_planning_flows",
            "list_recurring_rules",
            "list_snapshots",
            "list_transaction_imports",
            "list_transaction_months",
            "list_transactions",
            "materialize_recurring",
            "reconcile_transfers",
            "simulate_projection",
            "unreconcile_transfer",
            "update_allocation_rule",
            "update_asset",
            "update_asset_value",
            "update_budget_entry",
            "update_fire_settings",
            "update_liability",
            "update_planning_flow",
            "update_transaction",
            "update_transactions",
        ],
        "catálogo congelado: cada tool nueva se añade aquí a conciencia"
    );
}

#[tokio::test]
async fn get_summary_tool_matches_http_endpoint_byte_for_byte() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Datos mínimos para que el summary no sea trivial.
    let cat = app.create_category(&owner, "asset", "Indexados").await;
    let create = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": cat,
                "name": "MSCI World",
                "current_value": "12345.67",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(create.status, http::StatusCode::CREATED, "{create:?}");

    let http_body = app.get_with_cookie("/v1/summary", &owner.cookie).await;
    assert_eq!(http_body.status, http::StatusCode::OK);
    let via_http = http_body.json();

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("get_summary", serde_json::json!({})),
    )
    .await;
    let via_tool = tool_text_json(&envelope);

    assert_eq!(
        via_tool, via_http,
        "la tool y el endpoint deben serializar EXACTAMENTE el mismo struct"
    );
    // El contrato Decimal-as-string sobrevive el camino MCP.
    assert!(via_tool["total_assets"].is_string(), "{via_tool}");
}

#[tokio::test]
async fn get_projection_is_hybrid_without_asset_series_and_caches() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat = app.create_category(&owner, "asset", "Fondo").await;
    let create = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": cat,
                "name": "Fondo global",
                "current_value": "1000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(create.status, http::StatusCode::CREATED);

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("get_projection", serde_json::json!({})),
    )
    .await;
    let proj = tool_text_json(&envelope);

    // Densidad hybrid: ~82 puntos, nunca la serie mensual completa (~841).
    let points = proj["points"].as_array().expect("points");
    assert!(
        points.len() < 150,
        "hybrid esperado (~82 puntos), llegaron {}",
        points.len()
    );
    assert_eq!(
        proj["asset_series"].as_array().map(|a| a.len()),
        Some(0),
        "asset_series vacío por defecto (include_asset_series=false)"
    );

    // Paridad con el endpoint HTTP en la misma densidad (misma respuesta cacheada).
    let via_http = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();
    assert_eq!(proj["points"], via_http["points"], "misma serie que HTTP hybrid");

    // La primera llamada pobló el cache compartido con el handler HTTP.
    let cache = app.state.projection_cache.read().await;
    assert!(!cache.is_empty(), "get_projection debe poblar el cache de proyección");

    // include_asset_series=true sí devuelve las series por activo.
    drop(cache);
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("get_projection", serde_json::json!({"include_asset_series": true})),
    )
    .await;
    let with_series = tool_text_json(&envelope);
    assert!(
        with_series["asset_series"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "con include_asset_series=true llegan las series por activo"
    );
}

#[tokio::test]
async fn validation_error_is_tool_error_with_http_error_body() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "get_transactions_summary",
            serde_json::json!({"avg_window": "42"}),
        ),
    )
    .await;
    let result = &envelope["result"];
    assert_eq!(result["isError"], true, "validación → tool error: {envelope}");
    let text = result["content"][0]["text"].as_str().unwrap();
    let body: serde_json::Value = serde_json::from_str(text).expect("mismo JSON {{error,message}}");
    assert_eq!(body["error"], "bad_request");
    assert!(
        body["message"].as_str().unwrap().contains("avg_window"),
        "{body}"
    );
}

#[tokio::test]
async fn view_mine_filters_to_token_user() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "mario", "member").await;

    // Un activo de cada usuario.
    let cat = app.create_category(&owner, "asset", "Cash").await;
    for (who, name, value) in [(&owner, "De alice", "100"), (&member, "De mario", "50")] {
        let r = app
            .post_json_with_cookie(
                "/v1/assets",
                serde_json::json!({"category_id": cat, "name": name, "current_value": value}),
                &who.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    let token = {
        let created = app
            .post_json_with_cookie(
                "/v1/api-tokens",
                serde_json::json!({"label": "de mario"}),
                &member.cookie,
            )
            .await;
        created.json()["token"].as_str().unwrap().to_string()
    };

    // household (default) ve ambos; mine solo el del dueño del token (mario).
    let envelope = mcp_post(&app, &token, tool_call_body("list_assets", serde_json::json!({}))).await;
    assert_eq!(tool_text_json(&envelope).as_array().unwrap().len(), 2);

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_assets", serde_json::json!({"view": "mine"})),
    )
    .await;
    let mine = tool_text_json(&envelope);
    assert_eq!(mine.as_array().unwrap().len(), 1, "{mine}");
    assert_eq!(mine[0]["name"], "De mario");
}

#[tokio::test]
async fn list_transactions_truncates_with_total_count() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat = app.create_category(&owner, "expense", "Comida").await;
    for i in 0..3 {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                serde_json::json!({
                    "op_date": format!("2026-07-{:02}", 10 + i),
                    "amount": "-10.00",
                    "kind": "expense",
                    "concept": format!("compra {i}"),
                    "category_id": cat,
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_transactions", serde_json::json!({"limit": 2})),
    )
    .await;
    let page = tool_text_json(&envelope);
    assert_eq!(page["total_count"], 3);
    assert_eq!(page["truncated"], true);
    assert_eq!(page["transactions"].as_array().unwrap().len(), 2);

    // limit fuera de rango → tool error bad_request.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_transactions", serde_json::json!({"limit": 900})),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true);
}

#[tokio::test]
async fn get_settings_returns_installation_and_role() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let envelope = mcp_post(&app, &token, tool_call_body("get_settings", serde_json::json!({}))).await;
    let settings = tool_text_json(&envelope);
    assert_eq!(settings["role"], "owner");
    assert!(settings["installation"]["base_currency"].is_string());
    assert!(settings["installation"]["fire_settings"]["swr_pct"].is_string());
}

#[tokio::test]
async fn mcp_disabled_returns_404() {
    // TestApp::spawn monta el MCP; aquí se construye el router a mano con mcp_enabled=false.
    let (pool, _schema) = common::isolated_pool().await;
    let state = Arc::new(AppState::new(
        env!("CARGO_PKG_VERSION"),
        pool.clone(),
        false,
        30,
        false,
        None,
    ));
    let router = Router::new()
        .merge(routes::app_router(&state))
        .layer(Extension(state.clone()));
    let app = TestApp {
        router,
        pool,
        schema: _schema,
        state,
    };

    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await; // el CRUD de tokens sigue montado

    let resp = app
        .request(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/mcp")
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    serde_json::to_vec(&initialize_body()).unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(
        resp.status,
        http::StatusCode::NOT_FOUND,
        "con FUTUREFIN_MCP_ENABLED=0 /mcp no existe: {resp:?}"
    );
}

#[tokio::test]
async fn tools_list_exposes_annotations_on_every_tool() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    for tool in resp["result"]["tools"].as_array().expect("tools array") {
        let name = tool["name"].as_str().unwrap();
        let ann = &tool["annotations"];
        assert!(
            ann.is_object(),
            "la tool {name} debe declarar annotations (sin ellas un cliente conforme asume el peor caso): {tool}"
        );
        assert!(
            ann["title"].is_string(),
            "la tool {name} debe declarar un title legible"
        );
        assert_eq!(
            ann["openWorldHint"], false,
            "el servidor solo toca su propia DB ({name})"
        );
        // Escrituras (issue #3): readOnlyHint false + hints de destructividad/idempotencia
        // coherentes con la tabla del issue. Todo lo demás es lectura.
        let is_write = name.starts_with("create_")
            || name.starts_with("update_")
            || name.starts_with("delete_")
            || matches!(
                name,
                "capture_snapshot" | "materialize_recurring" | "reconcile_transfers"
                    | "unreconcile_transfer" | "apply_categorization_rule"
            );
        if is_write {
            assert_eq!(ann["readOnlyHint"], false, "tool {name}");
            // `apply_categorization_rule` reescribe la categoría/kind de filas históricas: es
            // destructiva aunque no empiece por update_/delete_. Declararlo aquí es deliberado —
            // el resto del catálogo deriva sus hints del prefijo del nombre.
            let expect_destructive = name.starts_with("update_")
                || name.starts_with("delete_")
                || name == "apply_categorization_rule";
            assert_eq!(ann["destructiveHint"], expect_destructive, "tool {name}");
            let expect_idempotent = name.starts_with("update_")
                || name.starts_with("delete_")
                || matches!(
                    name,
                    "capture_snapshot" | "materialize_recurring" | "reconcile_transfers"
                        | "apply_categorization_rule"
                );
            assert_eq!(ann["idempotentHint"], expect_idempotent, "tool {name}");
        } else {
            assert_eq!(ann["readOnlyHint"], true, "tool {name}");
        }
    }
}

#[tokio::test]
async fn get_settings_includes_user_identity() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let envelope =
        mcp_post(&app, &token, tool_call_body("get_settings", serde_json::json!({}))).await;
    let settings = tool_text_json(&envelope);
    assert_eq!(settings["user"]["username"], "alice", "{settings}");
    assert!(settings["user"]["id"].is_string());
    // El shape histórico (installation + role) sigue intacto.
    assert_eq!(settings["role"], "owner");
    assert!(settings["installation"]["base_currency"].is_string());
}

#[tokio::test]
async fn get_history_asset_series_is_opt_in_and_windowed() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Un asset y una captura de snapshot para que la serie no sea vacía.
    let cat = app.create_category(&owner, "asset", "Fondo").await;
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": cat,
                "name": "MSCI World",
                "current_value": "10000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let captured = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({}),
            &owner.cookie,
        )
        .await;
    assert!(captured.status.is_success(), "{captured:?}");

    // HTTP sin params: asset_series presente (contrato REST intacto).
    let via_http = app.get_with_cookie("/v1/history/series", &owner.cookie).await;
    assert_eq!(via_http.status, http::StatusCode::OK);
    let http_json = via_http.json();
    assert!(
        !http_json["asset_series"].as_array().unwrap().is_empty(),
        "{http_json}"
    );

    // Tool sin params: asset_series omitida por defecto, puntos presentes.
    let envelope =
        mcp_post(&app, &token, tool_call_body("get_history", serde_json::json!({}))).await;
    let tool_json = tool_text_json(&envelope);
    assert!(tool_json["asset_series"].as_array().unwrap().is_empty());
    assert!(!tool_json["points"].as_array().unwrap().is_empty());

    // Opt-in explícito → idéntico al HTTP por defecto.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "get_history",
            serde_json::json!({"include_asset_series": true}),
        ),
    )
    .await;
    assert_eq!(tool_text_json(&envelope), http_json);

    // window_months acota la rejilla (con 1 solo snapshot de hoy la serie ya es corta;
    // el clamp no debe romper nada).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("get_history", serde_json::json!({"window_months": 1})),
    )
    .await;
    let windowed = tool_text_json(&envelope);
    assert!(windowed["points"].as_array().unwrap().len() <= 2, "{windowed}");
}

#[tokio::test]
async fn list_transactions_offset_paginates_in_sql() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat = app.create_category(&owner, "expense", "Comida").await;
    for i in 0..3 {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                serde_json::json!({
                    "op_date": format!("2026-07-{:02}", 10 + i),
                    "amount": "-10.00",
                    "kind": "expense",
                    "concept": format!("compra {i}"),
                    "category_id": cat,
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    // Página 1: limit 2 → 2 filas, total 3, truncated.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_transactions", serde_json::json!({"limit": 2})),
    )
    .await;
    let page1 = tool_text_json(&envelope);
    assert_eq!(page1["total_count"], 3);
    assert_eq!(page1["offset"], 0);
    assert_eq!(page1["truncated"], true);
    let page1_rows = page1["transactions"].as_array().unwrap().clone();
    assert_eq!(page1_rows.len(), 2);

    // Página 2: offset 2 → la fila restante, truncated false, sin solaparse con la página 1.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "list_transactions",
            serde_json::json!({"limit": 2, "offset": 2}),
        ),
    )
    .await;
    let page2 = tool_text_json(&envelope);
    assert_eq!(page2["total_count"], 3);
    assert_eq!(page2["truncated"], false);
    let page2_rows = page2["transactions"].as_array().unwrap();
    assert_eq!(page2_rows.len(), 1);
    let ids1: Vec<&str> = page1_rows.iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert!(!ids1.contains(&page2_rows[0]["id"].as_str().unwrap()));
}

/// Los filtros de búsqueda (3.8.0) por la vía MCP: mismo resultado que el GET, `total_count`
/// coherente con el conjunto FILTRADO (no con el total del hogar) y paginación que sigue bajando a
/// SQL. Si el `COUNT(*)` no compartiera los mismos filtros, `truncated` mentiría en cuanto se
/// buscara algo.
#[tokio::test]
async fn list_transactions_search_filters_match_http_and_paginate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    for (date, concept, amount) in [
        ("2026-07-01", "WWW.AMAZON* AAA", "-30.00"),
        ("2026-07-02", "WWW.AMAZON* BBB", "-40.00"),
        ("2026-07-03", "Café Módena", "-5.00"),
        ("2026-08-04", "AMAZON PRIME", "-8.99"),
    ] {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                serde_json::json!({ "op_date": date, "amount": amount, "kind": "expense",
                                    "concept": concept }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    // Tool con filtro de concepto: 3 de 4, y el total_count refleja el conjunto filtrado.
    let out = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_transactions", serde_json::json!({"concept_contains": "amazon"})),
        )
        .await,
    );
    assert_eq!(out["total_count"], 3, "{out}");
    assert_eq!(out["transactions"].as_array().unwrap().len(), 3);

    // Byte a byte con el GET equivalente.
    let http = app
        .get_with_cookie("/v1/transactions?concept_contains=amazon", &owner.cookie)
        .await;
    assert_eq!(
        out["transactions"], http.json(),
        "la tool debe devolver exactamente las filas del GET"
    );

    // Paginación CON filtro: el COUNT comparte los filtros, así que truncated es fiable.
    let page1 = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_transactions",
                serde_json::json!({"concept_contains": "amazon", "limit": 2}),
            ),
        )
        .await,
    );
    assert_eq!(page1["total_count"], 3);
    assert_eq!(page1["truncated"], true);
    assert_eq!(page1["transactions"].as_array().unwrap().len(), 2);

    let page2 = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_transactions",
                serde_json::json!({"concept_contains": "amazon", "limit": 2, "offset": 2}),
            ),
        )
        .await,
    );
    assert_eq!(page2["truncated"], false);
    assert_eq!(page2["transactions"].as_array().unwrap().len(), 1);

    // El plegado de tildes también por MCP.
    let out = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_transactions", serde_json::json!({"concept_contains": "modena"})),
        )
        .await,
    );
    assert_eq!(out["total_count"], 1, "{out}");

    // Importe con signo y rango de fechas combinados.
    let out = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_transactions",
                serde_json::json!({"max_amount": "-30", "date_from": "2026-07-01",
                                   "date_to": "2026-07-31"}),
            ),
        )
        .await,
    );
    assert_eq!(out["total_count"], 2, "gastos de 30 € o más en julio: {out}");

    // `month` + rango a la vez → error de dominio tipado, el MISMO 400 que por HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "list_transactions",
            serde_json::json!({"month": "2026-07", "date_from": "2026-07-01"}),
        ),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
}

/// Paridad byte a byte de `get_allocation_resolution` (3.8.0) contra su GET. Es una tool de
/// lectura: no hay cuarteto de escritura, la garantía es que devuelve exactamente el struct del
/// endpoint.
#[tokio::test]
async fn get_allocation_resolution_matches_http_endpoint() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    for (cat, amount) in [(&cat_inc, "3000"), (&cat_exp, "1000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                serde_json::json!({"category_id": cat, "amount": amount}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat_ast, "name": "Indexado",
                               "current_value": "1000", "is_liquid": true}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            serde_json::json!({"target_asset_id": asset_id, "kind": "remainder"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let http = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await;
    assert_eq!(http.status, http::StatusCode::OK, "{http:?}");
    let tool = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("get_allocation_resolution", serde_json::json!({})),
        )
        .await,
    );
    assert_eq!(tool, http.json(), "la tool debe devolver el struct del endpoint intacto");

    // Y no toca la cache de proyección: construye su propio input, no pasa por `*_cached`.
    // `settle_login_warmup` primero: el warm-up del login puebla la cache en background y sin
    // esperarlo esta aserción es una carrera (culpaba a la tool de lo que hizo el login).
    app.settle_login_warmup(app.installation_id().await).await;
    let tool_again = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("get_allocation_resolution", serde_json::json!({})),
        )
        .await,
    );
    assert_eq!(tool_again, http.json(), "estable entre llamadas");
    assert!(
        app.state.projection_cache.read().await.is_empty(),
        "get_allocation_resolution no debe poblar la cache de proyección"
    );
}

#[tokio::test]
async fn new_read_tools_match_http_endpoints() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Datos: categorías de varios scopes + un movimiento (para months/imports vacío no rompe).
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let _cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({
                "op_date": "2026-07-10",
                "amount": "-25.00",
                "kind": "expense",
                "concept": "mercado",
                "category_id": cat_exp,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // Paridad byte a byte tool ↔ endpoint para los listados extraídos en este cambio.
    for (tool, path, args) in [
        ("list_categories", "/v1/categories", serde_json::json!({})),
        (
            "list_allocation_rules",
            "/v1/allocation-rules",
            serde_json::json!({}),
        ),
        (
            "list_transaction_months",
            "/v1/transactions/months",
            serde_json::json!({}),
        ),
        (
            "list_transaction_imports",
            "/v1/transactions/imports",
            serde_json::json!({}),
        ),
        (
            "list_recurring_rules",
            "/v1/transactions/recurring",
            serde_json::json!({}),
        ),
        (
            "list_categorization_rules",
            "/v1/transactions/rules",
            serde_json::json!({}),
        ),
        (
            "get_history_cashflow",
            "/v1/history/cashflow",
            serde_json::json!({"include_curve": true}),
        ),
        (
            "get_category_monthly_series",
            "/v1/transactions/category-series?kind=expense",
            serde_json::json!({"kind": "expense"}),
        ),
    ] {
        let via_http = app.get_with_cookie(path, &owner.cookie).await;
        assert_eq!(via_http.status, http::StatusCode::OK, "{path}");
        let envelope = mcp_post(&app, &token, tool_call_body(tool, args)).await;
        assert_eq!(
            tool_text_json(&envelope),
            via_http.json(),
            "paridad {tool} ↔ {path}"
        );
    }
}

#[tokio::test]
async fn category_series_shapes_and_validates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat = app.create_category(&owner, "expense", "Comida").await;
    for (date, amount) in [("2026-06-05", "-40.00"), ("2026-07-10", "-25.00")] {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                serde_json::json!({
                    "op_date": date,
                    "amount": amount,
                    "kind": "expense",
                    "concept": "mercado",
                    "category_id": cat,
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "get_category_monthly_series",
            serde_json::json!({"kind": "expense", "window_months": 6}),
        ),
    )
    .await;
    let series = tool_text_json(&envelope);
    assert_eq!(series["kind"], "expense");
    assert_eq!(series["window_months"], 6);
    let entries = series["series"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{series}");
    let months = entries[0]["months"].as_array().unwrap();
    assert_eq!(months.len(), 6, "cero-relleno: un punto por mes de la ventana");
    // Magnitud ≥ 0 como string decimal (gasto −25 → "25.00").
    let julio = months.iter().find(|m| m["month"] == "2026-07").unwrap();
    assert_eq!(julio["total"], "25.00");

    // kind inválido → tool error tipado.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "get_category_monthly_series",
            serde_json::json!({"kind": "savings"}),
        ),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
}

#[tokio::test]
async fn list_snapshots_items_are_opt_in_and_year_validates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat = app.create_category(&owner, "asset", "Fondo").await;
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "F", "current_value": "5000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    let captured = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({}),
            &owner.cookie,
        )
        .await;
    assert!(captured.status.is_success(), "{captured:?}");

    // Default: cabecera con total pero items vacíos.
    let envelope =
        mcp_post(&app, &token, tool_call_body("list_snapshots", serde_json::json!({}))).await;
    let snaps = tool_text_json(&envelope);
    let arr = snaps.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr[0]["items"].as_array().unwrap().is_empty(), "{snaps}");
    assert!(arr[0]["total"].is_string());

    // include_items → detalle presente.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_snapshots", serde_json::json!({"include_items": true})),
    )
    .await;
    let snaps = tool_text_json(&envelope);
    assert!(!snaps.as_array().unwrap()[0]["items"].as_array().unwrap().is_empty());

    // Año fuera de rango → mismo error tipado que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_snapshots", serde_json::json!({"year": 1800})),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true);
    let text = envelope["result"]["content"][0]["text"].as_str().unwrap();
    let body: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(body["error"], "bad_request");
}

/// REGRESIÓN (issue #7 §4) — `view` desconocido por MCP devuelve tool-error, no el hogar entero.
///
/// Éste era el repro literal del issue: `list_transactions {"view":"no-existe-esta-vista"}` →
/// 200 con `total_count` del **hogar completo**. La tool no valida por su cuenta — comparte
/// `LedgerViewQuery::resolve` con el HTTP —, así que lo que fija este test es que el rechazo
/// **llegue** al cliente MCP como tool-error legible y no como un 200 con otros datos.
#[tokio::test]
async fn unknown_view_is_a_tool_error_not_the_whole_household() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("mcp_view_owner").await;
    let token = create_token(&app, &owner).await;

    for bad in ["no-existe-esta-vista", "MINE", "self"] {
        for tool in ["list_transactions", "get_summary", "get_history_cashflow"] {
            let resp = mcp_post(
                &app,
                &token,
                tool_call_body(tool, serde_json::json!({"view": bad})),
            )
            .await;
            assert_eq!(
                resp["result"]["isError"], true,
                "{tool} con view={bad} debería ser tool-error: {resp}"
            );
            let body: serde_json::Value = serde_json::from_str(
                resp["result"]["content"][0]["text"].as_str().expect("texto"),
            )
            .expect("json de error");
            assert_eq!(body["code"], "invalid_view", "{tool} view={bad}: {body}");
        }
    }

    // Y los válidos siguen sirviendo, `household` explícito incluido.
    for good in ["mine", "household"] {
        let resp = mcp_post(
            &app,
            &token,
            tool_call_body("list_transactions", serde_json::json!({"view": good})),
        )
        .await;
        assert_ne!(
            resp["result"]["isError"], true,
            "view={good} debería seguir sirviendo: {resp}"
        );
    }
}
