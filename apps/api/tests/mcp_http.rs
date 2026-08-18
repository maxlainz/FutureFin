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
            "get_budget",
            "get_history",
            "get_projection",
            "get_settings",
            "get_summary",
            "get_transactions_summary",
            "list_assets",
            "list_liabilities",
            "list_planning_flows",
            "list_transactions",
        ],
        "catálogo v1 congelado"
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
        // Lecturas → readOnlyHint true. Las tools de escritura (issue #3) declaran false y
        // este assert se ramifica por tool cuando existan.
        assert_eq!(ann["readOnlyHint"], true, "tool {name}");
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
