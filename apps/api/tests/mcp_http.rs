//! Flujo MCP end-to-end sobre `/mcp` (Streamable HTTP, camino stateless 2026-07-28).
//!
//! Congela el contrato del servidor MCP: initialize, catálogo exacto de tools, paridad
//! byte a byte entre una tool y su endpoint HTTP hermano, densidad hybrid fija de la
//! proyección, errores de validación como tool-error legible, y el kill-switch
//! `mcp_enabled=false`.

mod common;

use common::{LoggedInOwner, TempWebRoot, TestApp, TestConfig};

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
            "aggregate_transactions",
            "apply_categorization_rule",
            "capture_snapshot",
            "confirm_transfer_match",
            "create_allocation_rule",
            "create_asset",
            "create_batch",
            "create_budget_entry",
            "create_categorization_rule",
            "create_category",
            "create_liability",
            "create_planning_flow",
            "create_snapshot",
            "create_transaction",
            "deflate_amount",
            "delete_allocation_rule",
            "delete_asset",
            "delete_budget_entry",
            "delete_categorization_rule",
            "delete_category",
            "delete_import",
            "delete_liability",
            "delete_planning_flow",
            "delete_recurring_rule",
            "delete_snapshot",
            "delete_transaction",
            "find_duplicate_transactions",
            "get_allocation_resolution",
            "get_budget",
            "get_category_monthly_series",
            "get_history",
            "get_history_cashflow",
            "get_liability_schedule",
            "get_projection",
            "get_retirement_profile",
            "get_settings",
            "get_summary",
            "get_transactions_summary",
            "list_allocation_rules",
            "list_assets",
            "list_categories",
            "list_categorization_rules",
            "list_goals",
            "list_liabilities",
            "list_planning_flows",
            "list_recent_changes",
            "list_recurring_rules",
            "list_snapshots",
            "list_transaction_imports",
            "list_transaction_months",
            "list_transactions",
            "materialize_recurring",
            "reconcile_transfers",
            "simulate_projection",
            "suggest_transfer_matches",
            "unreconcile_transfer",
            "update_allocation_rule",
            "update_asset",
            "update_asset_value",
            "update_budget_entry",
            "update_categorization_rule",
            "update_category",
            "update_fire_settings",
            "update_installation_settings",
            "update_liability",
            "update_planning_flow",
            "update_retirement_profile",
            "update_snapshot",
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

/// **`members[].series` es opt-in en la tool, y la respuesta del hogar cabe en el contexto.**
///
/// Mismo criterio y mismo default que `asset_series`, tomado con la medida delante: por HTTP el
/// agregado de dos miembros a densidad `hybrid` pesa ~34 KB y **11,7 KB son las series por
/// miembro** (~5,9 KB cada una, lineal con el tamaño del hogar). Un modelo no dibuja: los hitos
/// de cada persona ya viajan en `members[]` como enteros. Se deja pedible —y no retirada— porque
/// el token de un miembro NO puede pedir el `view=mine` de otro, así que esta es la única vía
/// para ver su curva.
#[tokio::test]
async fn get_projection_household_omits_member_series_unless_asked() {
    /// Tope del payload de UNA lectura de proyección del hogar en la tool. No persigue el byte:
    /// caza el crecimiento lineal (un campo nuevo por punto se multiplica por ~78 puntos y por el
    /// número de miembros). Si se pone rojo, recorta lo que se publica, no subas la constante.
    const TOOL_HOUSEHOLD_MAX_BYTES: usize = 32_000;

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let token = create_token(&app, &owner).await;

    for (u, tag) in [(&owner, "A"), (&bob, "B")] {
        let cat = app.create_category(u, "asset", &format!("Fondos {tag}")).await;
        let r = app
            .post_json_with_cookie(
                "/v1/assets",
                serde_json::json!({"category_id": cat, "name": format!("Indexado {tag}"),
                                   "current_value": "20000"}),
                &u.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("get_projection", serde_json::json!({"view": "household"})),
    )
    .await;
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .expect("texto de la tool")
        .to_string();
    let proj: serde_json::Value = serde_json::from_str(&text).expect("json");
    let members = proj["members"].as_array().expect("members");
    assert_eq!(members.len(), 2, "{members:?}");
    for m in members {
        assert_eq!(
            m["series"].as_array().map(|a| a.len()),
            Some(0),
            "series por miembro vacía por defecto: {m}"
        );
        // …pero los hitos y el horizonte propio SÍ viajan: es lo que sustituye a la curva.
        assert!(m["horizon_months"].as_u64().is_some_and(|v| v > 0), "{m}");
        assert!(m["username"].is_string(), "{m}");
    }
    println!("get_projection household/hybrid sin series por miembro: {} B", text.len());
    assert!(
        text.len() <= TOOL_HOUSEHOLD_MAX_BYTES,
        "la lectura del hogar por MCP pesa {} B y el tope es {TOOL_HOUSEHOLD_MAX_BYTES}",
        text.len()
    );

    // Con el flag sí llegan, en la misma rejilla que `points`.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "get_projection",
            serde_json::json!({"view": "household", "include_member_series": true}),
        ),
    )
    .await;
    let with = tool_text_json(&envelope);
    let grid: Vec<serde_json::Value> = with["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|p| p["month_index"].clone())
        .collect();
    for m in with["members"].as_array().expect("members") {
        let own: Vec<serde_json::Value> = m["series"]
            .as_array()
            .expect("series")
            .iter()
            .map(|p| p["month_index"].clone())
            .collect();
        assert_eq!(own, grid, "misma rejilla que points: {m}");
    }
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

    // `household` EXPLÍCITO (desde 5.0.0 ya no es el default, R2) ve ambos; `mine` solo el del
    // dueño del token (mario). Desde la Fase 5 la tool envuelve el array en `{view, assets}` — y
    // el eco de `view` es justo lo que hacía falta aquí: con un solo activo por usuario los dos
    // arrays podían coincidir sin que nada dijera qué scope se aplicó.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_assets", serde_json::json!({"view": "household"})),
    )
    .await;
    let all = tool_text_json(&envelope);
    assert_eq!(all["view"], "household", "{all}");
    assert_eq!(all["assets"].as_array().unwrap().len(), 2, "{all}");

    // Y omitir el parámetro es `mine`: el default cambió, y con él la población sobre la que
    // responde un agente que no pide scope.
    let omitido = tool_text_json(
        &mcp_post(&app, &token, tool_call_body("list_assets", serde_json::json!({}))).await,
    );
    assert_eq!(omitido["view"], "mine", "5.0.0: sin `view` la tool filtra a lo del token: {omitido}");
    assert_eq!(omitido["assets"].as_array().unwrap().len(), 1, "{omitido}");

    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_assets", serde_json::json!({"view": "mine"})),
    )
    .await;
    let mine = tool_text_json(&envelope);
    assert_eq!(mine["view"], "mine", "{mine}");
    assert_eq!(mine["assets"].as_array().unwrap().len(), 1, "{mine}");
    assert_eq!(mine["assets"][0]["name"], "De mario");
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
    // 5.0.0: `fire_settings` es lo COMPARTIDO del hogar; el SWR y los otros tres ejes movidos
    // viven en `get_retirement_profile` (D13).
    assert!(settings["installation"]["fire_settings"]["taxable_gain_ratio"].is_string());
    assert!(
        settings["installation"]["fire_settings"]["swr_pct"].is_null(),
        "el SWR ya no es del hogar: {settings}"
    );
}

/// Shell mínimo de la SPA, para montar el `ServeDir` del binario publicado.
const SHELL: &str = "<!doctype html><html><head></head><body><div id=\"root\"></div></body></html>";

/// El kill-switch tiene que fallar **limpio en la imagen que se publica**, no solo en el router
/// de laboratorio (issue #85, hallazgo 1).
///
/// Antes este test construía el router **sin SPA**, así que confirmaba un 404 que en producción
/// no ocurría: allí el fallback final es un `ServeDir` con fallback al `index.html`, y `ServeDir`
/// **no llama a su fallback para métodos distintos de GET/HEAD**. El resultado real era
/// `POST /mcp` → **405 con cuerpo vacío** y
/// `GET /.well-known/oauth-authorization-server` → **200 `text/html`** con el shell de la SPA:
/// el conector fallaba al parsear JSON y enseñaba «connection failed» sin causa — un control de
/// seguridad que, al activarse, se diagnostica como avería.
///
/// Ahora las rutas se montan siempre y el switch cambia el handler (misma doctrina que
/// `/v1/auth/sso`, D18). El test monta `WEB_STATIC_ROOT` con `spa::mount_static_spa`, la MISMA
/// función que llama `main.rs`.
#[tokio::test]
async fn mcp_disabled_answers_json_even_with_the_spa_mounted() {
    let web = TempWebRoot::with_index(SHELL);
    let app = TestApp::spawn_with(TestConfig {
        mcp_disabled: true,
        web_static_root: Some(web.path.clone()),
        ..Default::default()
    })
    .await;

    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await; // el CRUD de tokens sigue montado

    // Primero: el `ServeDir` está de verdad ahí. Sin esta comprobación el resto del test podría
    // pasar por no haber montado nada, que es justo el fallo que veníamos a cerrar.
    let spa = app.get("/una-ruta-cualquiera-de-la-spa").await;
    assert_eq!(spa.status, http::StatusCode::OK);
    assert!(
        spa.headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/html"),
        "el fallback SPA debería estar sirviendo el shell: {spa:?}"
    );

    // POST /mcp: 404 JSON con código estable, NUNCA 405 mudo.
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
        "con FUTUREFIN_MCP_ENABLED=0 /mcp responde 404, no 405: {resp:?}"
    );
    assert_json_mcp_disabled(&resp);

    // Y el protocolo OAuth, que es lo que consulta el conector ANTES de llegar a /mcp.
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-authorization-server/mcp",
    ] {
        let resp = app.get(path).await;
        assert_eq!(resp.status, http::StatusCode::NOT_FOUND, "{path}: {resp:?}");
        assert_json_mcp_disabled(&resp);
    }
    for path in ["/oauth/register", "/oauth/token", "/oauth/revoke"] {
        let resp = app.post_json(path, serde_json::json!({})).await;
        assert_eq!(resp.status, http::StatusCode::NOT_FOUND, "{path}: {resp:?}");
        assert_json_mcp_disabled(&resp);
    }

    // `/oauth/authorize` sigue siendo de la SPA: con el switch echado tampoco se lo queda el API.
    let authorize = app.get("/oauth/authorize").await;
    assert_eq!(authorize.status, http::StatusCode::OK);
}

fn assert_json_mcp_disabled(resp: &common::ResponseParts) {
    let ct = resp
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("application/json"),
        "debe ser JSON (si es text/html, el fallback SPA se lo está tragando): {ct}"
    );
    assert_eq!(resp.json()["code"], "mcp_disabled", "{resp:?}");
}

/// Con el MCP encendido y el `ServeDir` montado, `/mcp` sigue llegando a rmcp: la ruta del API
/// gana al fallback estático. (Es la otra mitad del test anterior: montar el SPA no puede
/// tragarse el endpoint bueno.)
#[tokio::test]
async fn mcp_still_works_behind_the_static_fallback() {
    let web = TempWebRoot::with_index(SHELL);
    let app = TestApp::spawn_with(TestConfig {
        web_static_root: Some(web.path.clone()),
        ..Default::default()
    })
    .await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, initialize_body()).await;
    assert_eq!(resp["result"]["protocolVersion"], PROTOCOL, "{resp}");
}

/// Hallazgo 3: la validación de `Origin` de rmcp estaba apagada (su default es lista vacía).
///
/// El dato que decide si esto rompe a Claude Desktop / Claude Code: rmcp
/// (`validate_origin_header`) devuelve `Ok(())` cuando **falta** la cabecera, aunque la lista NO
/// esté vacía. Los clientes sin navegador no mandan `Origin` y siguen entrando; los dos primeros
/// asserts fijan justamente eso.
#[tokio::test]
async fn mcp_validates_the_origin_header() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let post = |origin: Option<&'static str>| {
        let token = token.clone();
        async move {
            let mut builder = http::Request::builder()
                .method(http::Method::POST)
                .uri("/mcp")
                .header(http::header::HOST, "futurefin.test")
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::ACCEPT, "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROTOCOL)
                .header("Mcp-Method", "initialize")
                .header(http::header::AUTHORIZATION, format!("Bearer {token}"));
            if let Some(o) = origin {
                builder = builder.header(http::header::ORIGIN, o);
            }
            builder
                .body(axum::body::Body::from(
                    serde_json::to_vec(&initialize_body()).unwrap(),
                ))
                .unwrap()
        }
    };

    // Sin Origin (Claude Desktop, Claude Code, curl): pasa.
    let sin = app.request(post(None).await).await;
    assert_eq!(
        sin.status,
        http::StatusCode::OK,
        "una request sin Origin NO se puede rechazar: rompería a los clientes sin navegador"
    );
    // Origin de la lista (el default de CORS_ORIGINS incluye este): pasa.
    let permitido = app.request(post(Some("http://127.0.0.1:8080")).await).await;
    assert_eq!(permitido.status, http::StatusCode::OK, "{permitido:?}");
    // Origin ajeno (el vector de DNS rebinding desde una pestaña cualquiera): 403.
    let ajeno = app.request(post(Some("https://malicioso.example")).await).await;
    assert_eq!(
        ajeno.status,
        http::StatusCode::FORBIDDEN,
        "un Origin fuera de CORS_ORIGINS debe rechazarse: {ajeno:?}"
    );
}

/// Hallazgos 4 y 5: `/mcp` tiene su propia capa CORS —**sin `allow_credentials`**— y su
/// preflight admite las cabeceras que un cliente MCP de navegador manda de verdad.
#[tokio::test]
async fn mcp_preflight_is_complete_and_grants_no_cookie_access() {
    let app = TestApp::spawn().await;

    let resp = app
        .request(
            http::Request::builder()
                .method(http::Method::OPTIONS)
                .uri("/mcp")
                .header(http::header::ORIGIN, "http://127.0.0.1:8080")
                .header(http::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    http::header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "authorization, content-type, mcp-protocol-version, mcp-session-id, \
                     last-event-id",
                )
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert!(
        resp.status.is_success(),
        "el preflight de /mcp debe pasar: {resp:?}"
    );
    let allowed = resp
        .headers
        .get(http::header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    for h in [
        "mcp-protocol-version", // obligatoria fuera de initialize desde 2025-06-18
        "last-event-id",        // reanudación de SSE
        "mcp-session-id",
        "authorization",
    ] {
        assert!(allowed.contains(h), "falta {h} en allow-headers: {allowed}");
    }
    // `Access-Control-Expose-Headers` no viaja en el preflight (es cabecera de la respuesta
    // real), así que se comprueba sobre una petición de verdad — un 401 sin Bearer sirve, y de
    // paso fija que `WWW-Authenticate` es legible: sin exponerla, un cliente de navegador no
    // puede leer el `resource_metadata=` del 401 y nunca descubre el authorization server.
    let real = app
        .request(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/mcp")
                .header(http::header::HOST, "futurefin.test")
                .header(http::header::ORIGIN, "http://127.0.0.1:8080")
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(
                    serde_json::to_vec(&initialize_body()).unwrap(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(real.status, http::StatusCode::UNAUTHORIZED, "{real:?}");
    let exposed = real
        .headers
        .get(http::header::ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    for h in ["mcp-protocol-version", "mcp-session-id", "www-authenticate"] {
        assert!(exposed.contains(h), "falta {h} en expose-headers: {exposed}");
    }
    assert!(
        real.headers
            .get(http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            .is_none(),
        "tampoco en la respuesta real: {real:?}"
    );

    // La mitad que importa del hallazgo 4: `/mcp` NO concede acceso con cookie…
    assert!(
        resp.headers
            .get(http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            .is_none(),
        "la capa CORS de /mcp no debe permitir credenciales: {resp:?}"
    );
    // …mientras que el API sí, para el MISMO origen. Son dos capas distintas, y por eso añadir
    // un origen para un cliente MCP de navegador ya no regala la cookie a /v1/backup/user-export.
    let api_preflight = app
        .request(
            http::Request::builder()
                .method(http::Method::OPTIONS)
                .uri("/v1/backup/user-export")
                .header(http::header::ORIGIN, "http://127.0.0.1:8080")
                .header(http::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(http::header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(
        api_preflight
            .headers
            .get(http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "el API sí usa cookie: su capa mantiene allow-credentials"
    );
}

#[tokio::test]
async fn simulate_cash_axes_carry_their_bound_in_the_json_schema() {
    // Issue #27, «Nota sobre la descripción»: la cota `>= 0` de los ejes de caja vivía SOLO en
    // la prosa de la descripción. Un cliente no la ve como restricción — la ve como texto, y
    // solo si lo lee entero; el error llegaba en runtime. `months` sí la llevaba
    // (`schemars(range)`), pero `range` no aplica a strings decimales: la forma correcta es
    // `regex(pattern)`. Es declarativo (rmcp deserializa con serde_json y no valida contra el
    // schema), así que esto fija la DESCRIPCIÓN del contrato, no su cumplimiento.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    let tool = resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|t| t["name"] == "simulate_projection")
        .expect("simulate_projection en el catálogo")
        .clone();
    let props = &tool["inputSchema"]["properties"];

    for axis in ["extra_monthly_cash_adjustment", "extra_monthly_savings"] {
        let pattern = props[axis]["pattern"].as_str().unwrap_or_else(|| {
            panic!("{axis} debe publicar su cota como `pattern` en el schema: {}", props[axis])
        });
        assert!(
            pattern.contains("\\d"),
            "el patrón de {axis} debe describir un decimal no negativo, y es {pattern}"
        );
    }
    // `months` conserva su cota numérica: las dos formas conviven según el tipo del parámetro.
    assert_eq!(props["months"]["minimum"], 12);
    assert_eq!(props["months"]["maximum"], 840);
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
            // Verbos fuera de la convención `create_/update_/delete_`. `confirm_transfer_match`
            // (4.4.0) escribe: enlaza las dos patas de un par y las saca de todos los agregados
            // de flujo. Que su nombre empiece por `confirm_` es deliberado —el argumento es la
            // confirmación de una propuesta del servidor, no un par de ids— y por eso el brazo
            // tiene que ser explícito aquí en vez de derivarse del prefijo.
            || matches!(
                name,
                "capture_snapshot" | "materialize_recurring" | "reconcile_transfers"
                    | "unreconcile_transfer" | "apply_categorization_rule"
                    | "confirm_transfer_match"
            );
        if is_write {
            assert_eq!(ann["readOnlyHint"], false, "tool {name}");
            // `apply_categorization_rule` reescribe la categoría/kind de filas históricas: es
            // destructiva aunque no empiece por update_/delete_. Declararlo aquí es deliberado —
            // el resto del catálogo deriva sus hints del prefijo del nombre.
            // `materialize_recurring` no empieza por update_/delete_ pero PODA instancias
            // (`pruned` en la respuesta) y su ámbito es la instalación entera, así que puede
            // borrar movimientos de otro miembro. Con `destructiveHint: false` un cliente MCP
            // conforme no pedía permiso al humano antes de invocarla.
            let expect_destructive = name.starts_with("update_")
                || name.starts_with("delete_")
                || matches!(
                    name,
                    // `unreconcile_transfer` no borra filas, pero persiste un rechazo que solo
                    // se limpia volviendo a conciliar el par a mano — y esa acción NO está
                    // expuesta como tool. Desde el chat es irreversible.
                    "apply_categorization_rule" | "materialize_recurring" | "unreconcile_transfer"
                );
            assert_eq!(ann["destructiveHint"], expect_destructive, "tool {name}");
            let expect_idempotent = name.starts_with("update_")
                || name.starts_with("delete_")
                || matches!(
                    name,
                    "capture_snapshot" | "materialize_recurring" | "reconcile_transfers"
                        | "apply_categorization_rule"
                        // Reconfirmar un par ya conciliado devuelve el mismo par sin escribir
                        // nada: `confirm_transfer_match_core` resuelve también los pares YA
                        // casados entre sí, justo para poder anunciarlo.
                        | "confirm_transfer_match"
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
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    // #150: "Indexado" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.

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

/// **Quién NO está en este bucle, y por qué.** Salir de aquí es legítimo cuando la tool envuelve
/// deliberadamente la respuesta; lo que no vale es ablandar la aserción para que pase. Cada
/// salida tiene su prueba de paridad de CONTENIDO en otro test:
///
/// - `list_categorization_rules` (4.0.0) y `list_transactions`: paginan y devuelven
///   `{total_count, offset, truncated, <entidad>}` mientras el GET sigue sirviendo el array
///   entero (contrato REST intacto). Cubiertas por
///   `list_categorization_rules_paginates_without_changing_the_http_contract` y por
///   `list_tools_echo_the_applied_view_and_keep_content_parity`.
/// - **Fase 5 (issue #86)** — `list_assets`, `list_liabilities`, `list_planning_flows`,
///   `list_allocation_rules`, `list_transaction_months`: ecoan la vista aplicada en `{view, …}`.
///   El eco no puede venir de la core porque el GET es un array desnudo a propósito, así que lo
///   pone la tool. Cubiertas por `list_tools_echo_the_applied_view_and_keep_content_parity`.
/// - **Fase 5** — `list_transaction_imports`: eco de `view` **y** paginación. Cubierta por
///   `list_transaction_imports_paginates_and_echoes_the_view`.
/// - **Fase 5** — `list_snapshots`: pagina y suprime el detalle por ítem por defecto (own-user,
///   sin `view`). Cubierta por `list_snapshots_paginates_and_declares_item_suppression`.
///
/// Lo que queda aquí son las tools que siguen siendo `to_tool_result(core(...))` a pelo: su
/// promesa ES la identidad byte a byte con el GET.
#[tokio::test]
async fn new_read_tools_match_http_endpoints() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Datos: categorías de varios scopes + un movimiento (para months/imports vacío no rompe).
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let _cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let cat_liab = app.create_category(&owner, "liability", "Préstamos").await;
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

    // Un pasivo activo: hace que `get_budget` traiga la partida derivada de su cuota — la fila
    // sólo prueba algo si los dos lados tienen datos que contradecirse. (`list_liabilities` salió
    // del bucle en la Fase 5, pero el pasivo sigue haciendo falta para el presupuesto.)
    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_liab,
                "expense_category_id": cat_exp,
                "label": "Hipoteca",
                "principal": "50000",
                "payment_amount": "300",
                "payment_frequency": "monthly",
                "payment_end_date": "2090-01-01",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");
    let liab_id = liab.json()["id"].as_str().unwrap().to_string();
    let budget = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            serde_json::json!({"category_id": cat_exp, "amount": "400"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(budget.status, http::StatusCode::CREATED, "{budget:?}");

    // Paridad byte a byte tool ↔ endpoint para los listados extraídos en este cambio.
    for (tool, path, args) in [
        ("list_categories", "/v1/categories", serde_json::json!({})),
        // Fase 0 (issue #81): `get_budget` llevaba desde su alta sin ninguna aserción de
        // paridad — sólo aparecía como una cadena en el catálogo congelado. Es
        // `to_tool_result(core(...))` directo, así que la paridad byte a byte es exactamente
        // el contrato que promete (y desde la Fase 5 su `view` lo ecoa la core, no la tool).
        ("get_budget", "/v1/budget", serde_json::json!({})),
        (
            "list_recurring_rules",
            "/v1/transactions/recurring",
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
        // Fase 6 (issue #87). Las siete lecturas nuevas devuelven un OBJETO cuya core ya ecoa
        // `view`, así que la tool es `to_tool_result(core(...))` a pelo y la paridad byte a byte
        // es exactamente el contrato que prometen — ninguna necesita el sobre de
        // NOTA-VIEW-ENVELOPE. La única que se queda fuera es `list_recent_changes`, porque su
        // `now` es el instante de la consulta: dos llamadas no pueden coincidir byte a byte, y
        // su paridad se prueba aparte ignorando ese campo.
        (
            "aggregate_transactions",
            "/v1/transactions/aggregate",
            serde_json::json!({}),
        ),
        (
            "find_duplicate_transactions",
            "/v1/transactions/duplicates",
            serde_json::json!({}),
        ),
        (
            "suggest_transfer_matches",
            "/v1/transactions/transfer-matches",
            serde_json::json!({}),
        ),
        (
            "list_goals",
            "/v1/allocation-rules/goals",
            serde_json::json!({}),
        ),
        (
            "deflate_amount",
            "/v1/projection/deflate?amount=1000&month_index=120",
            serde_json::json!({"amount": "1000", "month_index": 120}),
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

    // `get_liability_schedule` va suelto porque su ruta lleva el id en el path.
    let path = format!("/v1/liabilities/{liab_id}/schedule");
    let via_http = app.get_with_cookie(&path, &owner.cookie).await;
    assert_eq!(via_http.status, http::StatusCode::OK, "{via_http:?}");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body(
            "get_liability_schedule",
            serde_json::json!({"liability_id": liab_id}),
        ),
    )
    .await;
    assert_eq!(
        tool_text_json(&envelope),
        via_http.json(),
        "paridad get_liability_schedule ↔ {path}"
    );
}

/// `list_recent_changes` ↔ `GET /v1/changes`, **ignorando `now`**.
///
/// Es la única lectura de la Fase 6 que no puede compararse byte a byte: `now` es el instante en
/// que se resolvió la consulta (y el `since` del siguiente sondeo), así que dos llamadas
/// consecutivas difieren ahí por diseño. Todo lo demás —incluidos los avisos que hacen honesta a
/// esta lectura: `covers_deletions`, `deletions_absent_reason` y `tables_missing_updated_at`—
/// tiene que ser idéntico, y se comprueba aquí.
#[tokio::test]
async fn recent_changes_tool_matches_the_endpoint_except_for_now() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat = app.create_category(&owner, "expense", "Comida").await;
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({
                "op_date": "2026-07-10", "amount": "-25.00", "kind": "expense",
                "concept": "mercado", "category_id": cat,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let since = "2000-01-01";
    let via_http = app
        .get_with_cookie(&format!("/v1/changes?since={since}"), &owner.cookie)
        .await;
    assert_eq!(via_http.status, http::StatusCode::OK, "{via_http:?}");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_recent_changes", serde_json::json!({"since": since})),
    )
    .await;

    let mut a = via_http.json();
    let mut b = tool_text_json(&envelope);
    // El aviso que hace honesta a esta lectura, comprobado antes de recortar nada.
    assert_eq!(a["covers_deletions"], false, "{a}");
    assert_eq!(a["deletions_absent_reason"], "no_tombstones", "{a}");
    assert_eq!(
        a["tables_missing_updated_at"],
        serde_json::json!(["categories", "allocation_rules"]),
        "las dos tablas sin updated_at se publican, no se omiten en silencio: {a}"
    );
    for v in [&mut a, &mut b] {
        v.as_object_mut().unwrap().remove("now");
    }
    assert_eq!(b, a, "paridad list_recent_changes ↔ /v1/changes (sin `now`)");
}

/// La capacidad `prompts` (Fase 6, issue #87): tres flujos estáticos, sin argumentos y sin I/O.
///
/// **Qué cliente los ve**: el conector remoto de claude.ai soporta hoy sólo `tools` — prompts y
/// resources no están soportados todavía en MCP remoto—, así que estos guiones sirven a Claude
/// Code (donde aparecen como `/mcp__<servidor>__<prompt>`) y a los clientes MCP genéricos. El
/// test existe igualmente: lo que se publica tiene que estar bien publicado.
#[tokio::test]
async fn prompts_are_listed_and_retrievable() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // La capacidad se anuncia en `initialize`: sin esto un cliente conforme ni pregunta.
    let init = mcp_post(&app, &token, initialize_body()).await;
    assert!(
        init["result"]["capabilities"]["prompts"].is_object(),
        "el servidor debe anunciar la capacidad `prompts`: {init}"
    );

    let listed = mcp_post(
        &app,
        &token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "prompts/list",
            "params": {"_meta": request_meta()}
        }),
    )
    .await;
    let mut names: Vec<String> = listed["result"]["prompts"]
        .as_array()
        .unwrap_or_else(|| panic!("prompts array: {listed}"))
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "amortizar_o_invertir",
            "auditoria_categorizacion",
            "revision_mensual",
        ],
        "catálogo congelado de prompts"
    );
    for prompt in listed["result"]["prompts"].as_array().unwrap() {
        assert!(prompt["title"].is_string(), "title legible: {prompt}");
        assert!(prompt["description"].is_string(), "descripción: {prompt}");
        // Sin argumentos a propósito: interpolar texto del cliente dentro de un guion que el
        // modelo lee como instrucciones es exactamente lo que no queremos.
        assert!(prompt.get("arguments").is_none(), "sin argumentos: {prompt}");
    }

    let got = mcp_post(
        &app,
        &token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "prompts/get",
            "params": {"name": "revision_mensual", "_meta": request_meta()}
        }),
    )
    .await;
    let messages = got["result"]["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("messages: {got}"));
    assert_eq!(messages.len(), 1, "{got}");
    assert_eq!(messages[0]["role"], "user", "{got}");
    let body = messages[0]["content"]["text"].as_str().unwrap();
    // Las tres salvedades que este flujo existe para no dejar que un modelo se salte.
    for needle in [
        "savings_source",
        "reconciled_excluded_count",
        "`null` no es cero",
    ] {
        assert!(
            body.contains(needle),
            "el guion de la revisión mensual debe nombrar «{needle}»: {body}"
        );
    }

    // Un nombre desconocido es un `invalid_params` que NOMBRA los que existen. Va por
    // `app.request` y no por `mcp_post` porque rmcp devuelve los errores de protocolo con
    // HTTP 400, no dentro de un envelope 200 (que es como viajan los errores de TOOL).
    let bad = app
        .request(
            http::Request::builder()
                .method(http::Method::POST)
                .uri("/mcp")
                .header(http::header::HOST, "futurefin.test")
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::ACCEPT, "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROTOCOL)
                .header("Mcp-Method", "prompts/get")
                .header("Mcp-Name", "no_existe")
                .header(http::header::AUTHORIZATION, format!("Bearer {token}"))
                .body(axum::body::Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 4,
                        "method": "prompts/get",
                        "params": {"name": "no_existe", "_meta": request_meta()}
                    }))
                    .unwrap(),
                ))
                .expect("build MCP request"),
        )
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST, "{bad:?}");
    let body = String::from_utf8(bad.body.clone()).expect("utf8");
    assert!(
        body.contains("revision_mensual"),
        "el error debe listar los prompts disponibles: {body}"
    );
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

    // Default: cabecera con total pero items vacíos — y desde la Fase 5 el sobre paginado dice
    // que el vacío es SUPRESIÓN (`items_included: false`, `item_count > 0`), no un snapshot vacío.
    let envelope =
        mcp_post(&app, &token, tool_call_body("list_snapshots", serde_json::json!({}))).await;
    let snaps = tool_text_json(&envelope);
    let arr = snaps["snapshots"].as_array().unwrap();
    assert!(!arr.is_empty(), "{snaps}");
    assert!(arr[0]["items"].as_array().unwrap().is_empty(), "{snaps}");
    assert_eq!(arr[0]["items_included"], false, "{snaps}");
    assert!(arr[0]["item_count"].as_i64().unwrap() > 0, "{snaps}");
    assert!(arr[0]["total"].is_string());

    // include_items → detalle presente y la supresión declarada como tal.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call_body("list_snapshots", serde_json::json!({"include_items": true})),
    )
    .await;
    let snaps = tool_text_json(&envelope);
    let first = &snaps["snapshots"][0];
    assert!(!first["items"].as_array().unwrap().is_empty(), "{snaps}");
    assert_eq!(first["items_included"], true, "{snaps}");

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

/// REGRESIÓN (auditoría MCP §4) — `view` desconocido por MCP devuelve tool-error, no el hogar entero.
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

/// REGRESIÓN (auditoría MCP §9) — la tool pagina; el GET sigue devolviendo el conjunto entero.
///
/// Es la única lista del catálogo que **crece con el uso normal**: `learn_rule` inserta una regla
/// por concepto distinto en cada import con `learn_rules = true`, así que una instalación con dos
/// años de extractos devolvía ~100 reglas de una tacada. Para un agente eso es una porción notable
/// de su ventana de contexto gastada sin pedirlo.
#[tokio::test]
async fn list_categorization_rules_paginates_without_changing_the_http_contract() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    for i in 0..5 {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions/rules",
                serde_json::json!({
                    "match_kind": "substring", "pattern": format!("COMERCIO {i}"),
                    "assign_kind": "expense",
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    // HTTP: array desnudo con las 5, sin sobre. El contrato REST no se toca.
    let http = app
        .get_with_cookie("/v1/transactions/rules", &owner.cookie)
        .await
        .json();
    assert_eq!(http.as_array().expect("array").len(), 5, "{http}");

    // MCP: sobre con total_count, y `truncated` dice la verdad en las dos direcciones.
    let page = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_categorization_rules", serde_json::json!({"limit": 2})),
        )
        .await,
    );
    assert_eq!(page["total_count"], 5, "{page}");
    assert_eq!(page["offset"], 0, "{page}");
    assert_eq!(page["truncated"], true, "{page}");
    assert_eq!(page["rules"].as_array().expect("rules").len(), 2, "{page}");

    let last = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_categorization_rules",
                serde_json::json!({"limit": 2, "offset": 4}),
            ),
        )
        .await,
    );
    assert_eq!(last["truncated"], false, "la última página no está truncada: {last}");
    assert_eq!(last["rules"].as_array().expect("rules").len(), 1, "{last}");

    // Y las reglas que sirve la página son las mismas que sirve el GET, en el mismo orden.
    let full = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_categorization_rules", serde_json::json!({"limit": 200})),
        )
        .await,
    );
    assert_eq!(full["rules"], http, "el contenido paginado debe ser el del GET");

    // Cota de `limit`.
    let bad = mcp_post(
        &app,
        &token,
        tool_call_body("list_categorization_rules", serde_json::json!({"limit": 999})),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true, "{bad}");
}

// ---------------------------------------------------------------------------
// Fase 5 (issue #86) — el sobre de los listados: eco de `view` y paginación.
// ---------------------------------------------------------------------------

/// **Los listados ecoan la vista aplicada; el GET sigue devolviendo un array.**
///
/// El agujero que cierra: en una instalación de un solo usuario, `view: "mine"` y `view` omitido
/// devolvían arrays **byte a byte idénticos**. Un cliente no podía distinguir «mine coincide con
/// el hogar» de «el parámetro se ignoró», y en un hogar de dos personas ésa es exactamente la
/// pregunta que decide si la cifra que está citando es la del hogar o la suya. Las respuestas de
/// objeto ya ecoan `view` desde su core; los listados no pueden (su GET es un array desnudo a
/// propósito y meterle un sobre rompería la SPA), así que el eco lo pone la tool.
///
/// Por eso estas tools salen del bucle byte a byte de `new_read_tools_match_http_endpoints`. Lo
/// que se sigue exigiendo —y se exige aquí— es la paridad de **contenido**: `envelope[clave]` debe
/// ser exactamente lo que devuelve el GET con el mismo scope.
#[tokio::test]
async fn list_tools_echo_the_applied_view_and_keep_content_parity() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat_asset = app.create_category(&owner, "asset", "Fondos").await;
    let cat_liab = app.create_category(&owner, "liability", "Préstamos").await;
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "name": "Fondo", "category_id": cat_asset, "current_value": "10000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");

    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_liab, "expense_category_id": cat_exp, "label": "Hipoteca",
                "principal": "50000", "payment_amount": "300", "payment_frequency": "monthly",
                "payment_end_date": "2090-01-01",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");

    let flow = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            serde_json::json!({
                "title": "Paga extra", "category_id": cat_inc, "expected_amount": "1200",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(flow.status, http::StatusCode::CREATED, "{flow:?}");

    // #150: "Fondo" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.

    let txn = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({
                "op_date": "2026-07-10", "amount": "-25.00", "kind": "expense",
                "concept": "mercado", "category_id": cat_exp,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(txn.status, http::StatusCode::CREATED, "{txn:?}");

    // (tool, GET, clave del sobre). El `limit` alto de `list_transactions` neutraliza su
    // paginación: lo que se compara aquí es el CONTENIDO, no el tamaño de página.
    let cases: &[(&str, &str, &str, serde_json::Value)] = &[
        ("list_assets", "/v1/assets", "assets", serde_json::json!({})),
        ("list_liabilities", "/v1/liabilities", "liabilities", serde_json::json!({})),
        ("list_planning_flows", "/v1/planning/flows", "planning_flows", serde_json::json!({})),
        (
            "list_allocation_rules",
            "/v1/allocation-rules",
            "allocation_rules",
            serde_json::json!({}),
        ),
        (
            "list_transaction_months",
            "/v1/transactions/months",
            "months",
            serde_json::json!({}),
        ),
        (
            "list_transactions",
            "/v1/transactions",
            "transactions",
            serde_json::json!({"limit": 500}),
        ),
        (
            "list_transaction_imports",
            "/v1/transactions/imports",
            "imports",
            serde_json::json!({}),
        ),
    ];

    for (tool, path, key, extra) in cases {
        for view in ["household", "mine"] {
            let mut args = extra.clone();
            args["view"] = serde_json::Value::String(view.to_string());
            let envelope = tool_text_json(&mcp_post(&app, &token, tool_call_body(tool, args)).await);

            assert_eq!(
                envelope["view"], view,
                "{tool} debe ecoar la vista aplicada, no ignorarla: {envelope}"
            );
            let sep = if path.contains('?') { '&' } else { '?' };
            let via_http = app
                .get_with_cookie(&format!("{path}{sep}view={view}"), &owner.cookie)
                .await;
            assert_eq!(via_http.status, http::StatusCode::OK, "{path}");
            assert_eq!(
                envelope[key],
                via_http.json(),
                "paridad de contenido {tool}[{key}] ↔ {path}?view={view}"
            );
        }

        // El GET sigue siendo un ARRAY: el sobre es de la tool, no del contrato REST. Si esto
        // empieza a fallar, alguien envolvió el endpoint HTTP y rompió la SPA.
        let bare = app.get_with_cookie(path, &owner.cookie).await;
        assert!(bare.json().is_array(), "{path} debe seguir sirviendo un array: {bare:?}");
    }
}

/// **`list_snapshots` pagina y DECLARA que ha suprimido el detalle.**
///
/// Dos agujeros a la vez. (1) El listado no tenía cota ninguna: un usuario que fotografía su
/// patrimonio cada mes acumula dos snapshots al mes y los recibía todos. (2) Sin `include_items`
/// la tool devolvía `items: []`, exactamente el mismo JSON que un snapshot sin ningún ítem —
/// «no te he mandado el detalle» y «aquí no hay nada» eran indistinguibles, con un `total` de
/// miles de euros al lado para rematar la contradicción. La supresión vive ahora en la core, que
/// es donde puede publicar `item_count` e `items_included`.
///
/// Sin `view`: el CRUD de snapshots es own-user y la tool no se inventa un scope que no tiene.
#[tokio::test]
async fn list_snapshots_paginates_and_declares_item_suppression() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let year = chrono::Utc::now().date_naive().format("%Y").to_string().parse::<i32>().unwrap() - 2;
    for month in 1..=3 {
        let date = format!("{year}-{month:02}-15");
        let r = app
            .post_json_with_cookie(
                "/v1/history/snapshots",
                serde_json::json!({
                    "kind": "asset",
                    "snapshot_date": date,
                    "items": [
                        {"label": "Cash", "value": "1000"},
                        {"label": "Bolsa", "value": "2000"},
                    ],
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "backfill {date}: {r:?}");
    }

    // Sin `view` en el sobre: la tool es own-user y no lo inventa.
    let page = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_snapshots", serde_json::json!({"limit": 2})),
        )
        .await,
    );
    assert!(page.get("view").is_none(), "list_snapshots es own-user, no debe ecoar view: {page}");
    assert_eq!(page["total_count"], 3, "{page}");
    assert_eq!(page["offset"], 0, "{page}");
    assert_eq!(page["truncated"], true, "{page}");
    assert_eq!(page["snapshots"].as_array().expect("snapshots").len(), 2, "{page}");

    // Supresión declarada, no adivinada: `items` vacío PERO `item_count` = 2.
    let first = &page["snapshots"][0];
    assert_eq!(first["items_included"], false, "{first}");
    assert_eq!(first["item_count"], 2, "{first}");
    assert_eq!(first["items"].as_array().expect("items").len(), 0, "{first}");

    let last = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_snapshots", serde_json::json!({"limit": 2, "offset": 2})),
        )
        .await,
    );
    assert_eq!(last["truncated"], false, "la última página no está truncada: {last}");
    assert_eq!(last["snapshots"].as_array().expect("snapshots").len(), 1, "{last}");

    // Con el detalle pedido, el contenido es el del GET (que siempre lo incluye).
    let http = app.get_with_cookie("/v1/history/snapshots", &owner.cookie).await;
    assert_eq!(http.status, http::StatusCode::OK, "{http:?}");
    assert!(http.json().is_array(), "el GET sigue sirviendo un array: {http:?}");
    let full = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_snapshots",
                serde_json::json!({"limit": 200, "include_items": true}),
            ),
        )
        .await,
    );
    assert_eq!(full["snapshots"], http.json(), "el contenido paginado debe ser el del GET");

    let bad = mcp_post(
        &app,
        &token,
        tool_call_body("list_snapshots", serde_json::json!({"limit": 999})),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true, "{bad}");
}

/// **`list_transaction_imports` pagina y ecoa la vista.** Crece un lote por cada CSV importado y
/// no tenía cota. La paginación baja a SQL (`list_imports_page`), el GET sigue devolviendo el
/// conjunto entero, y el puente `list_imports_core` —que existía sólo para no romper el build de
/// la capa MCP— desaparece con este cambio.
#[tokio::test]
async fn list_transaction_imports_paginates_and_echoes_the_view() {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    for i in 0..3 {
        // Conceptos distintos: la huella canónica deduplica filas idénticas y sin esto el
        // segundo import no crearía movimiento alguno.
        let csv = format!(
            "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
             1{i}/06/2026;1{i}/06/2026;COMPRA {i};-1{i},00;EUR\n"
        );
        let b64 = B64.encode(csv.as_bytes());
        let prev = app
            .post_json_with_cookie(
                "/v1/transactions/import/preview",
                serde_json::json!({"source": "myinvestor", "file_b64": b64}),
                &owner.cookie,
            )
            .await;
        assert_eq!(prev.status, http::StatusCode::OK, "preview {i}: {prev:?}");
        let pj = prev.json();
        let decisions: Vec<serde_json::Value> = pj["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| {
                serde_json::json!({
                    "kind": r["suggested_kind"],
                    "category_id": r["suggested_category_id"],
                })
            })
            .collect();
        let conf = app
            .post_json_with_cookie(
                "/v1/transactions/import/confirm",
                serde_json::json!({
                    "source": "myinvestor",
                    "file_b64": b64,
                    "file_sha256": pj["file_sha256"],
                    "decisions": decisions,
                    "learn_rules": false,
                }),
                &owner.cookie,
            )
            .await;
        assert!(conf.status.is_success(), "confirm {i}: {conf:?}");
    }

    let page = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_transaction_imports",
                // `household` explícito desde 5.0.0 (R2): las 3 importaciones son de dos personas.
                serde_json::json!({"limit": 2, "view": "household"}),
            ),
        )
        .await,
    );
    assert_eq!(page["view"], "household", "{page}");
    assert_eq!(page["total_count"], 3, "{page}");
    assert_eq!(page["truncated"], true, "{page}");
    assert_eq!(page["imports"].as_array().expect("imports").len(), 2, "{page}");

    let last = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body(
                "list_transaction_imports",
                serde_json::json!({"limit": 2, "offset": 2, "view": "mine"}),
            ),
        )
        .await,
    );
    assert_eq!(last["view"], "mine", "{last}");
    assert_eq!(last["truncated"], false, "{last}");
    assert_eq!(last["imports"].as_array().expect("imports").len(), 1, "{last}");

    // Contenido == GET (que sigue siendo el array entero, sin sobre).
    let http = app
        .get_with_cookie("/v1/transactions/imports", &owner.cookie)
        .await;
    assert_eq!(http.status, http::StatusCode::OK, "{http:?}");
    assert!(http.json().is_array(), "el GET sigue sirviendo un array: {http:?}");
    let full = tool_text_json(
        &mcp_post(
            &app,
            &token,
            tool_call_body("list_transaction_imports", serde_json::json!({"limit": 200})),
        )
        .await,
    );
    assert_eq!(full["imports"], http.json(), "el contenido paginado debe ser el del GET");

    let bad = mcp_post(
        &app,
        &token,
        tool_call_body("list_transaction_imports", serde_json::json!({"limit": 0})),
    )
    .await;
    assert_eq!(bad["result"]["isError"], true, "{bad}");
}

// ---------------------------------------------------------------------------
// Fase 0 del plan de mejora del MCP (issue #81) — congelar el CONTRATO, no sólo los nombres.
// ---------------------------------------------------------------------------

/// Firma congelable de una tool: qué parámetros publica, cuáles exige, y una señal de su
/// descripción.
fn tool_signature(tool: &serde_json::Value) -> serde_json::Value {
    let schema = &tool["inputSchema"];
    let mut properties: Vec<String> = schema["properties"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    properties.sort();
    let mut required: Vec<String> = schema["required"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default();
    // Ordenados: lo que es contrato es el CONJUNTO de obligatorios, no el orden en que
    // schemars decida emitirlos.
    required.sort();

    // Restricciones del schema ENTERO, recorrido recursivamente. `properties` + `required`
    // congelan QUÉ parámetros hay; esto congela QUÉ SE ACEPTA en cada uno.
    let constraints = collect_constraints_of(schema);
    let constraints_canonical = canonical_constraints_text(&constraints);

    let description = tool["description"].as_str().unwrap_or("");
    serde_json::json!({
        "name": tool["name"].as_str().unwrap_or_default(),
        "properties": properties,
        "required": required,
        // El cuerpo legible de las restricciones. A diferencia de la descripción (27 KB de
        // prosa, ilegible en un diff y por eso sólo hasheada), esto cabe en una línea por
        // nodo: se guarda ENTERO a propósito, porque un gate cuyo fallo no se puede leer se
        // "arregla" regenerando el fixture a ciegas — justo lo que este test viene a impedir.
        "constraints": constraints_as_json(&constraints),
        // Y su hash corto, la señal de una línea: "algo del contrato de esta tool se movió".
        "constraints_sha256_12": sha256_12(&constraints_canonical),
        // Señal de la descripción: longitud + hash corto. Congelar los 27 KB de prosa del
        // catálogo dentro del test lo volvería ilegible y nadie revisaría el diff; con esto,
        // vaciar, invertir o volver falsa una descripción rompe el test igual, y el diff del
        // fixture cabe en una línea. En 4.0.0 se encontraron TRES descripciones falsas, todas
        // por auditoría manual: esto es lo que convierte esa auditoría en un gate.
        "description_len": description.chars().count(),
        "description_sha256_12": sha256_12(description),
    })
}

// ---------------------------------------------------------------------------
// Fase 2 (issue #83) — congelar también las RESTRICCIONES, no sólo los nombres de parámetro.
//
// `properties` + `required` + el hash de la descripción dejaban ciega justo la superficie que
// la Fase 2 construyó: `additionalProperties: false`, los `enum`, los `pattern` y las cotas
// numéricas. Con sólo aquello, borrar mañana un `#[serde(deny_unknown_fields)]` o un
// `#[schemars(extend("enum" = [...]))]` no rompía un solo test: los parámetros seguirían
// llamándose igual y la descripción seguiría diciendo lo mismo — mentira incluida.
// ---------------------------------------------------------------------------

/// Claves de un nodo de JSON Schema que **son contrato de entrada**: lo que un cliente puede o
/// no puede mandar. Todo lo demás (`description`, `title`, `default`, `examples`) es prosa y
/// queda deliberadamente fuera — la prosa ya la cubre `description_sha256_12`.
const CONSTRAINT_KEYS: &[&str] = &[
    "type",
    "enum",
    "const",
    "format",
    "$ref",
    "additionalProperties",
    "pattern",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minItems",
    "maxItems",
    "uniqueItems",
    "required",
];

/// Claves cuyo valor es un CONJUNTO, no una secuencia: se ordenan antes de renderizar. Reordenar
/// `["mine", "household"]` no cambia lo que el servidor acepta, y un gate que salta por eso
/// entrena a la gente a regenerar sin mirar. (El ORDEN de los `enum` sí lo fija, tool a tool,
/// `enumerated_params_publish_a_real_enum_in_the_json_schema`.)
const SET_VALUED_KEYS: &[&str] = &["enum", "type", "required"];

/// Renderizado canónico de un valor JSON: **claves de objeto ordenadas**, siempre. Es la pieza
/// que hace el hash estable: ni schemars ni serde_json prometen un orden de emisión, y sin esto
/// una actualización de dependencia movería los 52 hashes sin que cambiara ningún contrato.
fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body: Vec<String> = keys
                .iter()
                .map(|k| format!("{k}:{}", canonical_json(&map[k.as_str()])))
                .collect();
            format!("{{{}}}", body.join(","))
        }
        serde_json::Value::Array(items) => {
            let body: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", body.join(","))
        }
        scalar => scalar.to_string(),
    }
}

fn canonical_constraint(key: &str, v: &serde_json::Value) -> String {
    match v.as_array() {
        Some(items) if SET_VALUED_KEYS.contains(&key) => {
            let mut rendered: Vec<String> = items.iter().map(canonical_json).collect();
            rendered.sort();
            format!("[{}]", rendered.join(","))
        }
        _ => canonical_json(v),
    }
}

/// Recorre el schema entero y anota, por posición, las restricciones que publica ese nodo.
///
/// Baja por `properties`, `items`, `$defs` (donde viven los anidados de `simulate_projection`),
/// los combinadores `anyOf`/`oneOf`/`allOf` (schemars emite ahí algunos `Option<T>`) y un
/// `additionalProperties` que sea un sub-schema en vez de `false`. Rutas: `$` la raíz,
/// `$.months` una propiedad, `$.kinds[]` los items de un array, `$defs.Nombre.campo` un anidado.
fn collect_constraints(
    node: &serde_json::Value,
    path: &str,
    out: &mut std::collections::BTreeMap<String, String>,
) {
    let Some(obj) = node.as_object() else {
        return;
    };

    let mut parts: Vec<String> = CONSTRAINT_KEYS
        .iter()
        .filter_map(|key| obj.get(*key).map(|v| format!("{key}={}", canonical_constraint(key, v))))
        .collect();
    parts.sort();
    if !parts.is_empty() {
        out.insert(path.to_string(), parts.join(" "));
    }

    if let Some(children) = obj.get("properties").and_then(|v| v.as_object()) {
        let mut names: Vec<&String> = children.keys().collect();
        names.sort();
        for name in names {
            collect_constraints(&children[name.as_str()], &format!("{path}.{name}"), out);
        }
    }
    if let Some(defs) = obj.get("$defs").and_then(|v| v.as_object()) {
        let mut names: Vec<&String> = defs.keys().collect();
        names.sort();
        for name in names {
            collect_constraints(&defs[name.as_str()], &format!("$defs.{name}"), out);
        }
    }
    match obj.get("items") {
        Some(serde_json::Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                collect_constraints(item, &format!("{path}[{i}]"), out);
            }
        }
        Some(item) => collect_constraints(item, &format!("{path}[]"), out),
        None => {}
    }
    if let Some(extra @ serde_json::Value::Object(_)) = obj.get("additionalProperties") {
        collect_constraints(extra, &format!("{path}.*"), out);
    }
    for combinator in ["anyOf", "oneOf", "allOf"] {
        if let Some(branches) = obj.get(combinator).and_then(|v| v.as_array()) {
            for (i, branch) in branches.iter().enumerate() {
                collect_constraints(branch, &format!("{path}|{combinator}[{i}]"), out);
            }
        }
    }
}

fn collect_constraints_of(schema: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    collect_constraints(schema, "$", &mut out);
    out
}

/// Texto que se hashea: una línea `ruta\trestricciones` por nodo, en orden de ruta (el
/// `BTreeMap` ya lo garantiza). Mismo schema ⇒ mismo texto ⇒ mismo hash, venga en el orden
/// que venga del serializador.
fn canonical_constraints_text(constraints: &std::collections::BTreeMap<String, String>) -> String {
    constraints
        .iter()
        .map(|(path, value)| format!("{path}\t{value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Descompone una línea `clave=valor clave=valor …` en sus pares.
///
/// Se corta buscando ` clave=` con las claves CONOCIDAS, nunca por espacios: un `pattern` puede
/// llevar un espacio dentro y partir a ciegas inventaría restricciones que nadie escribió.
fn split_constraints(line: &str) -> std::collections::BTreeMap<String, String> {
    let mut cuts: Vec<usize> = vec![0];
    for key in CONSTRAINT_KEYS {
        let needle = format!(" {key}=");
        let mut from = 0;
        while let Some(i) = line[from..].find(&needle) {
            cuts.push(from + i + 1);
            from += i + 1;
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = std::collections::BTreeMap::new();
    for (i, start) in cuts.iter().enumerate() {
        let end = cuts.get(i + 1).copied().unwrap_or(line.len());
        if let Some((k, v)) = line[*start..end].trim_end().split_once('=') {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

/// Diferencia legible entre las restricciones de un mismo nodo, restricción a restricción.
/// Decir «cambió» y volcar las dos líneas enteras deja al lector buscando la diferencia a ojo;
/// lo que hay que leer de un vistazo es QUÉ dejó de validarse.
fn constraint_delta_lines(tool: &str, path: &str, before: &str, after: &str) -> String {
    let (b, a) = (split_constraints(before), split_constraints(after));
    let mut keys: Vec<&String> = b.keys().chain(a.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut out = String::new();
    for key in keys {
        match (b.get(key), a.get(key)) {
            (Some(x), None) => out.push_str(&format!(
                "      ✗ {tool} → {path}: PERDIDA la restricción `{key}` (validaba {x})\n"
            )),
            (None, Some(y)) => out.push_str(&format!(
                "      + {tool} → {path}: restricción nueva `{key}` = {y}\n"
            )),
            (Some(x), Some(y)) if x != y => out.push_str(&format!(
                "      ~ {tool} → {path}: `{key}` {x} → {y}\n"
            )),
            _ => {}
        }
    }
    out
}

fn constraints_as_json(
    constraints: &std::collections::BTreeMap<String, String>,
) -> serde_json::Value {
    serde_json::Value::Object(
        constraints
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect(),
    )
}

fn sha256_12(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(s.as_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn catalog_fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp-catalog.json")
}

/// Congela el **contrato de entrada** de TODAS las tools, no sólo sus nombres.
/// (Sin contar cuántas son: el número quedó obsoleto dos veces — `52` describía la Fase 5 y el
/// catálogo lleva 70 desde 5.0.0. Cuéntalas con
/// `jq '.tools | length' apps/api/tests/fixtures/mcp-catalog.json`.)
///
/// `tools_list_returns_exactly_the_v1_catalog` (arriba) compara un `Vec<String>` de nombres y
/// nada más: con él en verde se puede vaciar una descripción, invertir su sentido, quitar un
/// parámetro obligatorio o añadir uno nuevo sin que falle un solo test. Este hermano fija, por
/// tool: las claves de `inputSchema.properties` ordenadas, el array `required`, una señal de
/// la descripción (longitud + SHA-256 corto) y —desde la Fase 2 (issue #83)— las
/// **restricciones** del schema recorrido recursivamente (`constraints` +
/// `constraints_sha256_12`).
///
/// Las restricciones son la superficie que la Fase 2 acaba de construir y que este congelador
/// no miraba: `additionalProperties: false`, los `enum`, los `pattern`, las cotas
/// `minimum`/`maximum`/`minLength`/`minItems`, el `type` y el `required` **a cada nivel** —
/// bajando por `properties`, `items`, `$defs` (los anidados de `simulate_projection`), los
/// combinadores y un `additionalProperties` que sea sub-schema. Sin esto, borrar un
/// `deny_unknown_fields` o un `enum` dejaba el catálogo idéntico a ojos de todos los tests.
///
/// El hash es **estable por construcción**: cada valor se renderiza con las claves de objeto
/// ordenadas, los conjuntos (`enum`, `type`, `required`) se ordenan también, y las rutas van en
/// un `BTreeMap`. Ni el orden de emisión de schemars ni el del serializador entran en el hash.
///
/// **Regenerar el fixture cuando el cambio es intencionado** (mismo patrón que
/// `UPDATE_ERROR_CODES=1` en `error_codes_parity.rs`):
///
/// ```text
/// UPDATE_MCP_CATALOG=1 TEST_DATABASE_URL="postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test" \
///   cargo test -p futurefin-api --test mcp_http -- tools_list_freezes_the_input_contract
/// ```
///
/// …y después revisa el diff de `tests/fixtures/mcp-catalog.json` como parte del PR: un
/// `description_sha256_12` que se mueve sin que la descripción deba cambiar es la señal, y una
/// línea de `constraints` que desaparece es una restricción que alguien acaba de perder.
#[tokio::test]
async fn tools_list_freezes_the_input_contract_of_every_tool() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    let mut tools: Vec<serde_json::Value> = resp["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array: {resp}"))
        .iter()
        .map(tool_signature)
        .collect();
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    let generated = serde_json::json!({
        "_doc": "Contrato de entrada de las tools MCP. GENERADO — no editar a mano: \
                 UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test mcp_http -- \
                 tools_list_freezes_the_input_contract. `description_sha256_12` son los 6 \
                 primeros bytes del SHA-256 de la descripción publicada. `constraints` son las \
                 restricciones del inputSchema por posición ($ = raíz, $.x = propiedad, $.x[] = \
                 items, $defs.T.x = anidado), y `constraints_sha256_12` su hash: cubren \
                 additionalProperties, enum, pattern, type, required y las cotas numéricas y de \
                 longitud a cada nivel.",
        "tool_count": tools.len(),
        "tools": tools,
    });
    let pretty = format!("{}\n", serde_json::to_string_pretty(&generated).unwrap());

    if std::env::var("UPDATE_MCP_CATALOG").is_ok() {
        std::fs::write(catalog_fixture_path(), &pretty).expect("escribir el fixture");
        eprintln!("fixture regenerado con {} tools", tools.len());
        return;
    }

    let current = std::fs::read_to_string(catalog_fixture_path()).unwrap_or_default();
    if current == pretty {
        return;
    }

    // Diff útil: qué tool cambió y en qué campo. Un `assert_eq!` de dos JSON de 52 entradas es
    // ilegible en la salida de cargo, y una diferencia ilegible se "arregla" regenerando a
    // ciegas — que es exactamente lo que este test existe para impedir.
    let old: serde_json::Value = serde_json::from_str(&current).unwrap_or(serde_json::Value::Null);
    let empty = vec![];
    let old_tools = old["tools"].as_array().unwrap_or(&empty);
    let mut report = String::new();
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        match old_tools.iter().find(|t| t["name"] == tool["name"]) {
            None => report.push_str(&format!("  + tool NUEVA: {name}\n")),
            Some(before) if before != tool => {
                for field in [
                    "properties",
                    "required",
                    "description_len",
                    "description_sha256_12",
                    "constraints_sha256_12",
                ] {
                    if before[field] != tool[field] {
                        report.push_str(&format!(
                            "  ~ {name}.{field}: {} → {}\n",
                            before[field], tool[field]
                        ));
                    }
                }
                // Un hash que no cuadra no se puede revisar: hay que decir QUÉ restricción se
                // movió y DÓNDE. Sin este bloque el único arreglo practicable sería regenerar
                // el fixture sin mirar, que es la forma de que este gate deje de existir sin
                // que nadie lo borre.
                let empty = serde_json::Map::new();
                let before_c = before["constraints"].as_object().unwrap_or(&empty);
                let after_c = tool["constraints"].as_object().unwrap_or(&empty);
                let mut paths: Vec<&String> = before_c.keys().chain(after_c.keys()).collect();
                paths.sort();
                paths.dedup();
                let text = |v: Option<&serde_json::Value>| {
                    v.and_then(|v| v.as_str()).unwrap_or("").to_string()
                };
                for path in paths {
                    let (b, a) = (text(before_c.get(path)), text(after_c.get(path)));
                    if b == a {
                        continue;
                    }
                    if a.is_empty() {
                        report.push_str(&format!("      ✗ NODO DESAPARECIDO en {name} → {path}\n"));
                    } else if b.is_empty() {
                        report.push_str(&format!("      + nodo nuevo en {name} → {path}\n"));
                    }
                    report.push_str(&constraint_delta_lines(name, path, &b, &a));
                }
            }
            Some(_) => {}
        }
    }
    for before in old_tools {
        if !tools.iter().any(|t| t["name"] == before["name"]) {
            report.push_str(&format!(
                "  - tool RETIRADA: {}\n",
                before["name"].as_str().unwrap_or("?")
            ));
        }
    }
    if report.is_empty() {
        report.push_str("  (sólo cambió el encabezado o el formato del fichero)\n");
    }
    panic!(
        "tests/fixtures/mcp-catalog.json no coincide con el catálogo que sirve /mcp:\n{report}\n\
         Si el cambio es intencionado, regenera con:\n  \
         UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test mcp_http -- \
         tools_list_freezes_the_input_contract\n\
         …y revisa el diff en el PR (una descripción que cambia de hash sin motivo es la señal \
         que este test busca). Una línea «PERDIDA la restricción» casi nunca es intencionada: \
         significa que una tool acaba de dejar de validar algo que validaba — regenerar el \
         fixture ahí no arregla nada, sólo borra la prueba."
    );
}

/// **`#[ignore]` A PROPÓSITO — diana de la Fase 2 (issue #83).**
///
/// Hoy falla, y eso es lo esperado. Medido al escribirlo (2026-08-28): **51 de las 52 tools de entonces**
/// aceptan propiedades desconocidas. `#[serde(deny_unknown_fields)]` aparece dos veces en
/// `src/mcp/server.rs`, pero una de ellas es `FireSettingsOverrideParam`, un struct ANIDADO
/// dentro de `SimulateParams` — no es el struct de params de ninguna tool. La única tool cuyo
/// `inputSchema` publica hoy `additionalProperties: false` es `simulate_projection`.
///
/// No se ablanda: un parámetro mal escrito por un cliente (`cap` en vez de `cap_kind` +
/// `cap_value`, el bug real de la auditoría MCP §5) se descarta hoy **en silencio** y la
/// llamada devuelve 200 sin haber hecho lo que se le pidió.
///
/// El test se escribió ignorado para que ese trabajo tuviera diana. **Cerrado en la Fase 2**
/// (2026-08-28, sobre las 52 de entonces): todas publican `additionalProperties: false`, incluidas las cuatro que
/// no tenían struct de params (`get_settings`, `list_recurring_rules`, `materialize_recurring`,
/// `reconcile_transfers`) — sin struct, rmcp emite un schema vacío que acepta cualquier campo,
/// así que se les dio `NoParams`. El `#[ignore]` se retira aquí: a partir de ahora, añadir una
/// tool sin `deny_unknown_fields` rompe el build de tests.
#[tokio::test]
async fn every_input_schema_forbids_unknown_properties() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    let offenders: Vec<String> = resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter(|t| t["inputSchema"]["additionalProperties"] != false)
        .map(|t| t["name"].as_str().unwrap_or("?").to_string())
        .collect();
    assert!(
        offenders.is_empty(),
        "estas tools aceptan propiedades desconocidas en silencio (falta \
         `#[serde(deny_unknown_fields)]` en su struct de Params): {offenders:?}"
    );
}


/// Fase 5 (issue #86) — **el catálogo cabe en el contexto**.
///
/// Las descripciones de `tools/list` viajan ENTERAS en cada conversación, se use el MCP o no.
/// Antes de esta fase sumaban 37.214 caracteres (~36 KB) en las 52 tools de entonces, con cinco por encima de
/// 1.200 y una de 3.821 — y la estrategia fallaba justo donde importa: en la auditoría en vivo
/// la descripción de `get_summary` (2.278) llegó al cliente **truncada**, cortada en mitad de
/// una advertencia sobre inconsistencia entre tools. Una advertencia que no llega no protege de
/// nada, así que la prosa larga no era «más segura»: era menos.
///
/// El tope no es estético. La disciplina que impone es la que arregló el problema: **lo que se
/// puede comprobar en la respuesta no se explica en la descripción**. Cuando una advertencia
/// deja de caber, la salida correcta casi siempre es un campo de procedencia
/// (`*_absent_reason`, `*_basis`, `has_data`…) que se lee en el momento de mirar la cifra, o
/// una línea en el `instructions` del servidor si es transversal a varias tools — no recortar
/// el aviso hasta que deje de avisar.
///
/// Si este test falla, NO subas la constante: mueve la prosa a uno de esos dos sitios.
#[tokio::test]
async fn tool_descriptions_stay_within_the_context_budget() {
    /// Tope por descripción. 600 nace de la medida, no de la estética: con todas las tools por
    /// debajo, el catálogo entero cabe holgadamente en `TOTAL_BUDGET`.
    const PER_TOOL_MAX: usize = 600;
    /// Tope del catálogo entero. Deja margen para tools nuevas sin volver a los ~36 KB.
    const TOTAL_BUDGET: usize = 24_000;

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    let tools = resp["result"]["tools"].as_array().expect("tools array");

    let mut total = 0usize;
    let mut offenders: Vec<(usize, String)> = Vec::new();
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or("?").to_string();
        let len = tool["description"].as_str().unwrap_or("").chars().count();
        total += len;
        if len > PER_TOOL_MAX {
            offenders.push((len, name));
        }
    }
    offenders.sort_by(|a, b| b.0.cmp(&a.0));
    assert!(
        offenders.is_empty(),
        "descripciones por encima de {PER_TOOL_MAX} caracteres: {offenders:?}. Mueve lo que \
         sobra a un campo de procedencia de la respuesta o al `instructions` del servidor — \
         subir la constante reintroduce el truncado que esta fase arregló"
    );
    assert!(
        total <= TOTAL_BUDGET,
        "el catálogo suma {total} caracteres de descripciones y el presupuesto es \
         {TOTAL_BUDGET} (antes de la Fase 5 eran 37.214)"
    );
}

/// Fase 2 (issue #83) — **los enumerados son `enum` en el JSON Schema, no prosa**.
///
/// Antes de la Fase 2, los ~25 parámetros enumerados del catálogo eran `Option<String>` con la
/// lista de valores solo en el `///` y la validación en runtime. El cliente lee el esquema ANTES
/// que la descripción (y a veces la descripción se trunca), así que `view: "MINE"` o
/// `resolution: "hourly"` se escribían con total confianza y el error llegaba después.
///
/// Se comprueba una muestra representativa de las tres familias —vista, dominio del ledger y
/// configuración FIRE— más las dos formas que no son un `Option<String>` suelto: un campo
/// OBLIGATORIO (`get_category_monthly_series.kind`) y un array cuyo `enum` va en los `items`
/// (`capture_snapshot.kinds`).
#[tokio::test]
async fn enumerated_params_publish_a_real_enum_in_the_json_schema() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let resp = mcp_post(&app, &token, tools_list_body()).await;
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    let schema_of = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} en el catálogo"))["inputSchema"]
            .clone()
    };

    let cases: &[(&str, &str, &[&str])] = &[
        ("get_summary", "view", &["mine", "household"]),
        ("list_transactions", "kind", &["expense", "income", "savings"]),
        ("list_categories", "scope", &["asset", "liability", "income", "expense"]),
        ("get_transactions_summary", "avg_window", &["3", "6", "12", "ytd", "all"]),
        ("get_history_cashflow", "resolution", &["weekly", "daily"]),
        ("list_snapshots", "kind", &["asset", "liability"]),
        ("apply_categorization_rule", "apply_to_existing", &["uncategorized", "all"]),
        ("update_allocation_rule", "cap_kind", &["amount", "months_expense", "income_multiple"]),
        (
            "update_liability",
            "repayment_model",
            &["fixed_payments", "french", "interest_only", "revolving"],
        ),
        ("create_liability", "payment_frequency", &["monthly", "weekly"]),
        (
            "update_categorization_rule",
            "match_kind",
            &["substring", "prefix", "exact"],
        ),
        (
            "update_fire_settings",
            "savings_source",
            &["budget", "transactions_avg", "budget_income_real_expense"],
        ),
        // 5.0.0: `fire_number_mode` se mudó al perfil de jubilación por usuario (D13).
        (
            "update_retirement_profile",
            "fire_number_mode",
            &["manual", "annual_expense", "current_income"],
        ),
        (
            "update_retirement_profile",
            "strategy",
            &["asap", "retire_at_age", "coast", "partial", "pension_bridge"],
        ),
        ("update_fire_settings", "expense_avg_window_mode", &["data", "calendar"]),
        ("simulate_projection", "view", &["mine", "household"]),
    ];
    for (tool, param, expected) in cases {
        let schema = schema_of(tool);
        let got = schema["properties"][param]["enum"]
            .as_array()
            .unwrap_or_else(|| {
                panic!(
                    "{tool}.{param} debe publicar `enum` en el schema, y publica {}",
                    schema["properties"][param]
                )
            })
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>();
        assert_eq!(got, *expected, "{tool}.{param}");
    }

    // Campo obligatorio: mismo trato, y además sigue en `required`.
    let series = schema_of("get_category_monthly_series");
    assert_eq!(series["properties"]["kind"]["enum"], serde_json::json!(["expense", "income"]));
    assert!(
        series["required"]
            .as_array()
            .expect("required")
            .contains(&serde_json::json!("kind")),
        "kind sigue siendo obligatorio: {series}"
    );

    // Array: el `enum` va en `items`, no en la propiedad (un `enum` en la raíz diría que el
    // ARRAY entero vale "asset", que es justo lo que no se quiere decir).
    let capture = schema_of("capture_snapshot");
    assert_eq!(
        capture["properties"]["kinds"]["items"]["enum"],
        serde_json::json!(["asset", "liability"]),
        "{capture}"
    );
    assert!(
        capture["properties"]["kinds"]["enum"].is_null(),
        "el enum de un array va en items, no en la raíz: {capture}"
    );

    // --- Barrido: la tabla de arriba fija los VALORES, pero se queda vieja sola. Esto pilla al
    // parámetro enumerado NUEVO que nadie añadió a la tabla, usando la convención del propio
    // repo: una descripción que enumera alternativas entrecomilladas separadas por `|`.
    //
    // Excepciones, con su porqué (sin la explicación nadie sabrá si se pueden quitar):
    const NOT_ENUMS: &[(&str, &str)] = &[
        // El catálogo de presets de banco crece con cada importador nuevo, y la lista de la
        // descripción es de EJEMPLOS (acaba en «…»), no el dominio cerrado del parámetro.
        ("create_categorization_rule", "source"),
        ("update_categorization_rule", "source"),
    ];
    let mut swept = 0usize;
    for tool in resp["result"]["tools"].as_array().expect("tools array") {
        let name = tool["name"].as_str().unwrap_or("?");
        let Some(props) = tool["inputSchema"]["properties"].as_object() else {
            continue;
        };
        for (param, schema) in props {
            let is_string = schema["type"] == "string"
                || schema["type"]
                    .as_array()
                    .is_some_and(|t| t.contains(&serde_json::json!("string")));
            let desc = schema["description"].as_str().unwrap_or("");
            // «"a" | "b"»: la forma con la que este repo escribe un enumerado en prosa.
            let looks_enumerated = is_string
                && desc.contains("\" | \"")
                && desc.matches('"').count() >= 4;
            if !looks_enumerated || NOT_ENUMS.contains(&(name, param.as_str())) {
                continue;
            }
            swept += 1;
            assert!(
                schema["enum"].is_array(),
                "{name}.{param} enumera valores en su descripción pero no publica `enum` en el \
                 schema (o es un enumerado y le falta el `#[schemars(extend(\"enum\" = …))]`, o \
                 no lo es y le falta una fila en NOT_ENUMS con el porqué): {schema}"
            );
        }
    }
    assert!(swept >= 10, "el barrido solo miró {swept} parámetros enumerados");
}

/// Fase 2 (issue #83) — **las cotas de los strings viajan en el esquema**.
///
/// Hermano generalizado de `simulate_cash_axes_carry_their_bound_in_the_json_schema`: los
/// importes decimales viajan como string, así que `range` no les sirve y su cota vivía en la
/// prosa. Dos patrones y no uno, porque el signo es semántica: el `amount` de un movimiento
/// acepta negativo (el gasto ES negativo) y el de una partida de presupuesto no.
///
/// Barre TODO el catálogo —**incluidos los objetos anidados** de `simulate_projection`
/// (`one_off_expense`, `asset_return_overrides`, `fire_settings_overrides`, `tax_brackets`)— en
/// vez de listar campos: un parámetro decimal nuevo sin patrón, o un UUID, o una fecha, falla
/// aquí sin que nadie tenga que acordarse de ampliar el test.
#[tokio::test]
async fn every_decimal_uuid_and_date_param_carries_its_pattern() {
    const SIGNED: &str = r"^-?\d+(\.\d+)?$";
    const NON_NEGATIVE: &str = r"^\d+(\.\d+)?$";
    const YMD: &str = r"^\d{4}-\d{2}-\d{2}$";
    const YM: &str = r"^\d{4}-\d{2}$";
    const UUID: &str =
        r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";
    const MATCH_ID: &str = r"^[0-9a-f]{24}$";

    /// Los ÚNICOS decimales con signo, nombrados uno a uno: el signo es una decisión de dominio
    /// y una lista por sufijo la borraría (`create_budget_entry.amount` es > 0 y
    /// `create_transaction.amount` no).
    const SIGNED_PARAMS: &[(&str, &str)] = &[
        ("create_transaction", "amount"),
        ("update_transaction", "amount"),
        ("list_transactions", "min_amount"),
        ("list_transactions", "max_amount"),
        // Fase 6: la agregación comparte los filtros del listado, así que comparte su signo.
        ("aggregate_transactions", "min_amount"),
        ("aggregate_transactions", "max_amount"),
        // El ítem del lote es el mismo alta que `create_transaction`: gasto negativo.
        ("create_batch", "amount"),
        // Deflactar un patrimonio negativo es una pregunta legítima.
        ("deflate_amount", "amount"),
        ("simulate_projection", "extra_monthly_expense"),
        ("create_asset", "expected_annual_return_percent"),
        ("update_asset", "expected_annual_return_percent"),
        ("update_asset_value", "expected_annual_return_percent"),
        ("simulate_projection", "expected_annual_return_percent"),
        // #146 (4.9.0): la inflación admite negativos ([−2, 50] — deflación sostenida).
        ("simulate_projection", "annual_inflation_percent"),
        ("update_fire_settings", "annual_inflation_assumption_percent"),
    ];

    /// Recorre un schema entero y devuelve `(nombre_del_parámetro, subschema)` de cada
    /// propiedad, bajando por `properties`, `items` y `$defs`.
    fn walk(schema: &serde_json::Value, out: &mut Vec<(String, serde_json::Value)>) {
        match schema {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    if k == "properties" {
                        if let Some(props) = v.as_object() {
                            for (name, sub) in props {
                                out.push((name.clone(), sub.clone()));
                                walk(sub, out);
                            }
                        }
                    } else if k == "items" || k == "$defs" || k == "definitions" {
                        walk(v, out);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let resp = mcp_post(&app, &token, tools_list_body()).await;

    let mut checked = 0usize;
    for tool in resp["result"]["tools"].as_array().expect("tools array") {
        let name = tool["name"].as_str().unwrap_or("?");
        let mut params = Vec::new();
        walk(&tool["inputSchema"], &mut params);
        for (param, schema) in params {
            // Solo strings: los enteros llevan `range`, cubierto por los otros tests.
            let is_string = schema["type"] == "string"
                || schema["type"]
                    .as_array()
                    .is_some_and(|t| t.contains(&serde_json::json!("string")));
            if !is_string {
                continue;
            }
            // `match_id` acaba en `_id` y NO es un UUID: es el hash corto de una propuesta de
            // `suggest_transfer_matches`, y ése es justo el punto (no nombra una fila, nombra
            // un par que el servidor considera candidato). Publica su formato real.
            let expected = if (name, param.as_str()) == ("confirm_transfer_match", "match_id") {
                MATCH_ID
            } else if param.ends_with("_id") {
                UUID
            } else if param.ends_with("_date") || param == "op_date" || param == "date" {
                YMD
            } else if param == "month" || param == "from_month" {
                YM
            } else if param.ends_with("_amount")
                || param.ends_with("_percent")
                || param == "amount"
                || param == "principal"
                || param == "current_value"
                || param == "purchase_price"
                || param == "swr_pct"
                || param == "cap_value"
                || param == "pct"
                || param == "up_to"
            {
                if SIGNED_PARAMS.contains(&(name, param.as_str())) {
                    SIGNED
                } else {
                    NON_NEGATIVE
                }
            } else {
                continue;
            };
            checked += 1;
            assert_eq!(
                schema["pattern"].as_str(),
                Some(expected),
                "{name}.{param} debe publicar su formato como `pattern`; publica {schema}"
            );
        }
    }
    // Suelo defensivo: si un refactor deja el barrido sin encontrar nada, el test pasaría vacío.
    assert!(checked >= 60, "el barrido solo miró {checked} parámetros");
}

