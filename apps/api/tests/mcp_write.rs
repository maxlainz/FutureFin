//! Tools MCP de escritura (issue #3, tramo 1): gate de rol + toggle vivo `mcp_write_enabled`,
//! paridad de la fila creada con el camino HTTP, y contrato de cache FULL/COND/NONE por el
//! camino MCP (espejo de `transactions_projection_cache.rs` / `history_snapshots.rs`).

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{Density, ProjectionCacheKey};
use serde_json::json;
use uuid::Uuid;

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

/// Asserta que la llamada devolvió tool-error con el código esperado y devuelve el body.
fn tool_error(envelope: &serde_json::Value, code: &str) -> serde_json::Value {
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
    let body: serde_json::Value =
        serde_json::from_str(envelope["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["error"], code, "{body}");
    body
}

async fn create_token_for(app: &TestApp, cookie: &str) -> String {
    let created = app
        .post_json_with_cookie("/v1/api-tokens", json!({"label": "write tests"}), cookie)
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

async fn create_token(app: &TestApp, owner: &LoggedInOwner) -> String {
    create_token_for(app, &owner.cookie).await
}

async fn installation_id(app: &TestApp) -> Uuid {
    sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .expect("installation id")
}

fn household_key(iid: Uuid) -> ProjectionCacheKey {
    ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Household,
        owner_user_id: None,
        density: Density::Monthly,
    }
}

async fn present(app: &TestApp, key: &ProjectionCacheKey) -> bool {
    app.state.projection_cache.read().await.contains_key(key)
}

async fn warm(app: &TestApp, cookie: &str, key: &ProjectionCacheKey) {
    let r = app.get_with_cookie("/v1/projection/series", cookie).await;
    assert_eq!(r.status, http::StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(present(app, key).await, "la cache debería estar caliente tras el GET");
}

async fn assert_invalidated(app: &TestApp, key: &ProjectionCacheKey, what: &str) {
    for _ in 0..40 {
        if !present(app, key).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("la mutación MCP «{what}» debía invalidar la cache de proyección");
}

async fn set_mode(app: &TestApp, cookie: &str, source: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": source } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode {source}: {r:?}");
}

#[tokio::test]
async fn viewer_role_cannot_write() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vera", "viewer").await;
    let token = create_token_for(&app, &viewer.cookie).await;
    let cat = app.create_category(&owner, "expense", "Comida").await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-01", "concept": "x", "amount": "-1.00", "kind": "expense", "category_id": cat}),
        ),
    )
    .await;
    tool_error(&envelope, "forbidden");

    // Y sigue pudiendo leer.
    let envelope = mcp_post(&app, &token, tool_call("get_summary", json!({}))).await;
    let _ = tool_json(&envelope);
}

#[tokio::test]
async fn write_toggle_cuts_writes_live_without_restart() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Comida").await;
    let args = json!({"op_date": "2026-07-01", "concept": "cena", "amount": "-20.00", "kind": "expense", "category_id": cat});

    // Toggle OFF por cookie → la siguiente llamada de escritura ya está cortada.
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({"mcp_write_enabled": false}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK);
    let envelope = mcp_post(&app, &token, tool_call("create_transaction", args.clone())).await;
    let body = tool_error(&envelope, "bad_request");
    assert!(
        body["message"].as_str().unwrap().starts_with("mcp_write_disabled"),
        "{body}"
    );

    // Las lecturas siguen funcionando con el toggle apagado.
    let envelope = mcp_post(&app, &token, tool_call("get_summary", json!({}))).await;
    let _ = tool_json(&envelope);

    // Toggle ON → la escritura vuelve, sin reinicio.
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({"mcp_write_enabled": true}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK);
    let envelope = mcp_post(&app, &token, tool_call("create_transaction", args)).await;
    let created = tool_json(&envelope);
    assert!(created["id"].is_string(), "{created}");
}

#[tokio::test]
async fn create_transaction_writes_the_same_row_as_http() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Comida").await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-02", "concept": "cena", "amount": "-23.50", "kind": "expense", "category_id": cat, "notes": "con amigos"}),
        ),
    )
    .await;
    let created = tool_json(&envelope);
    let id = created["id"].as_str().unwrap();
    assert!(created["resumen"].as_str().unwrap().contains("cena"), "{created}");
    assert_eq!(created["category_name"], "Comida");

    // La fila es indistinguible de una creada por HTTP: el GET la sirve con el mismo shape.
    let listed = app.get_with_cookie("/v1/transactions", &owner.cookie).await;
    let rows = listed.json();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .expect("fila creada por MCP visible por HTTP")
        .clone();
    assert_eq!(row["amount"], "-23.5000");
    assert_eq!(row["kind"], "expense");
    assert_eq!(row["source"], "manual");

    // Reenviar el mismo movimiento NO es 409: los duplicados manuales son legítimos (el ordinal
    // de huella toma MAX+1) — mismo contrato que HTTP. La descripción de la tool lo avisa.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-02", "concept": "cena", "amount": "-23.50", "kind": "expense", "category_id": cat}),
        ),
    )
    .await;
    let dup = tool_json(&envelope);
    assert_ne!(dup["id"], created["id"]);

    // update_transaction recategoriza y el owner-guard da 404 sobre movimientos ajenos.
    let cat_ocio = app.create_category(&owner, "expense", "Ocio").await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_transaction", json!({"id": id, "category_id": cat_ocio})),
    )
    .await;
    let updated = tool_json(&envelope);
    assert_eq!(updated["category_name"], "Ocio");

    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let member_token = create_token_for(&app, &member.cookie).await;
    let envelope = mcp_post(
        &app,
        &member_token,
        tool_call("update_transaction", json!({"id": id, "concept": "hackeo"})),
    )
    .await;
    tool_error(&envelope, "not_found");
}

#[tokio::test]
async fn cache_contract_cond_none_and_full_via_mcp() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = installation_id(&app).await;
    let key = household_key(iid);
    let cat = app.create_category(&owner, "expense", "Comida").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;

    // --- Modo A (budget): create_transaction vía MCP NO invalida (COND inactiva) -------------
    warm(&app, &owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-03", "concept": "a", "amount": "-1.00", "kind": "expense", "category_id": cat}),
        ),
    )
    .await;
    let _ = tool_json(&envelope);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        present(&app, &key).await,
        "modo A: las transacciones no son inputs del engine — la cache debe sobrevivir"
    );

    // --- capture_snapshot: NONE en cualquier modo (contrato D12) ------------------------------
    let envelope = mcp_post(&app, &token, tool_call("capture_snapshot", json!({}))).await;
    let snap = tool_json(&envelope);
    assert!(snap["snapshot_date"].is_string(), "{snap}");
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        present(&app, &key).await,
        "los snapshots no son inputs del engine — la cache debe sobrevivir"
    );

    // --- create_planning_flow: FULL --------------------------------------------------------
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_planning_flow",
            json!({"title": "IRPF", "category_id": cat, "expected_amount": "800"}),
        ),
    )
    .await;
    let flow = tool_json(&envelope);
    assert!(flow["resumen"].as_str().unwrap().contains("IRPF"));
    assert_invalidated(&app, &key, "create_planning_flow").await;

    // --- Modo B: create_transaction vía MCP SÍ invalida (COND activa) -------------------------
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    warm(&app, &owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-04", "concept": "b", "amount": "1500.00", "kind": "income", "category_id": cat_inc}),
        ),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_invalidated(&app, &key, "create_transaction (modo B)").await;
}

#[tokio::test]
async fn recurring_create_and_materialize_are_idempotent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Gimnasio").await;

    // "Hoy" del servidor para elegir una fecha 2 meses atrás (backfill de meses cerrados).
    let r = app.get_with_cookie("/v1/history/series", &owner.cookie).await;
    let today =
        chrono::NaiveDate::parse_from_str(r.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d")
            .unwrap();
    let (y, m) = {
        let zero = (today.year() as i64) * 12 + (today.month() as i64 - 1) - 2;
        ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
    };
    use chrono::Datelike;
    let start = format!("{y:04}-{m:02}-05");

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": start, "concept": "gym", "amount": "-30.00", "kind": "expense", "category_id": cat, "recurring": true}),
        ),
    )
    .await;
    let created = tool_json(&envelope);
    assert!(created["recurring_rule_id"].is_string(), "{created}");

    // La plantilla es visible por la tool de lectura y ya quedó materializada hasta el último
    // mes cerrado (el alta backfillea) → materialize es un no-op idempotente.
    let envelope = mcp_post(&app, &token, tool_call("list_recurring_rules", json!({}))).await;
    let rules = tool_json(&envelope);
    assert_eq!(rules.as_array().unwrap().len(), 1);

    let envelope = mcp_post(&app, &token, tool_call("materialize_recurring", json!({}))).await;
    let out = tool_json(&envelope);
    assert_eq!(out["rules_processed"], 1);
    assert_eq!(out["materialized"], 0, "el alta ya backfilleó: {out}");
}

#[tokio::test]
async fn category_and_rule_creation_with_conflicts() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call("create_category", json!({"scope": "expense", "name": "Mascotas"})),
    )
    .await;
    let cat = tool_json(&envelope);
    let cat_id = cat["id"].as_str().unwrap().to_string();
    assert_eq!(cat["scope"], "expense");

    // Duplicado en el mismo scope → conflict.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("create_category", json!({"scope": "expense", "name": "Mascotas"})),
    )
    .await;
    tool_error(&envelope, "conflict");

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_categorization_rule",
            json!({"pattern": "KIWOKO", "source": "myinvestor", "assign_kind": "expense", "assign_category_id": cat_id}),
        ),
    )
    .await;
    let rule = tool_json(&envelope);
    assert!(rule["resumen"].as_str().unwrap().contains("KIWOKO"), "{rule}");

    // Duplicado (source, pattern) con source concreto → conflict (la UNIQUE de la tabla; con
    // source NULL Postgres no colisiona — contrato idéntico al HTTP).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_categorization_rule",
            json!({"pattern": "KIWOKO", "source": "myinvestor", "assign_kind": "expense", "assign_category_id": cat_id}),
        ),
    )
    .await;
    tool_error(&envelope, "conflict");

    // Scope inválido → bad_request tipado.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("create_category", json!({"scope": "nope", "name": "X"})),
    )
    .await;
    tool_error(&envelope, "bad_request");
}

#[tokio::test]
async fn planning_flow_update_and_due_date_tristate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Impuestos").await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_planning_flow",
            json!({"title": "IRPF", "category_id": cat, "expected_amount": "800", "due_date": "2026-10-15", "show_in_chart": true}),
        ),
    )
    .await;
    let flow = tool_json(&envelope);
    let flow_id = flow["id"].as_str().unwrap().to_string();
    assert!(flow["resumen"].as_str().unwrap().contains("2026-10-15"));

    // clear_due_date borra la fecha (tri-state del PATCH).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_planning_flow",
            json!({"id": flow_id, "clear_due_date": true, "expected_amount": "900"}),
        ),
    )
    .await;
    let updated = tool_json(&envelope);
    assert!(!updated["resumen"].as_str().unwrap().contains("2026-10-15"), "{updated}");
    assert!(updated["resumen"].as_str().unwrap().contains("900"));

    // due_date + clear_due_date a la vez → bad_request.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_planning_flow",
            json!({"id": updated["id"], "due_date": "2026-11-01", "clear_due_date": true}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");

    // amount <= 0 → mismo 400 que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_planning_flow",
            json!({"title": "X", "category_id": cat, "expected_amount": "0"}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");
}
