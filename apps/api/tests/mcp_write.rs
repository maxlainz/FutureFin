//! Tools MCP de escritura (issue #3, tramo 1): gate de rol + toggle vivo `mcp_write_enabled`,
//! paridad de la fila creada con el camino HTTP, y contrato de cache FULL/COND/NONE por el
//! camino MCP (espejo de `transactions_projection_cache.rs` / `history_snapshots.rs`).

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
    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    let cat = app.create_category(&owner, "expense", "Comida").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;

    // --- Modo A (budget): create_transaction vía MCP NO invalida (COND inactiva) -------------
    app.warm_household(&owner.cookie, &key).await;
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
    assert!(
        app.cache_contains(&key).await,
        "modo A: las transacciones no son inputs del engine — la cache debe sobrevivir"
    );

    // --- capture_snapshot: NONE en cualquier modo (contrato D12) ------------------------------
    let envelope = mcp_post(&app, &token, tool_call("capture_snapshot", json!({}))).await;
    let snap = tool_json(&envelope);
    assert!(snap["snapshot_date"].is_string(), "{snap}");
    assert!(
        app.cache_contains(&key).await,
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
    app.assert_invalidated(&key, "create_planning_flow").await;

    // --- Modo B: create_transaction vía MCP SÍ invalida (COND activa) -------------------------
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    app.warm_household(&owner.cookie, &key).await;
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
    app.assert_invalidated(&key, "create_transaction (modo B)").await;
}

#[tokio::test]
async fn update_asset_and_update_liability_share_cores_and_invalidate_full() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = app.installation_id().await;
    let key = app.household_key(iid);

    let cat_asset = app.create_category(&owner, "asset", "Fondos").await;
    let cat_asset2 = app.create_category(&owner, "asset", "Cash").await;
    let cat_liab = app.create_category(&owner, "liability", "Hipotecas").await;
    let cat_exp = app.create_category(&owner, "expense", "Vivienda").await;

    // Seed por las tools de alta (mismas cores que HTTP, ya cubiertas en esta suite).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_asset",
            json!({"name": "Fondo indexado", "category_id": cat_asset, "current_value": "10000", "purchase_price": "9000"}),
        ),
    )
    .await;
    let asset_id = tool_json(&envelope)["id"].as_str().unwrap().to_string();

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_liability",
            json!({"label": "Hipoteca", "category_id": cat_liab, "expense_category_id": cat_exp, "principal": "120000", "apr_percent": "3.10"}),
        ),
    )
    .await;
    let liability_id = tool_json(&envelope)["id"].as_str().unwrap().to_string();

    // --- update_asset: body completo (rename + recategorizar + iliquidez + borrar el precio
    // de compra) y contrato FULL — misma core `patch_asset_core` que el PATCH HTTP. ------------
    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_asset",
            json!({"asset_id": asset_id, "name": "Fondo global", "category_id": cat_asset2, "is_liquid": false, "clear_purchase_price": true}),
        ),
    )
    .await;
    let updated = tool_json(&envelope);
    assert!(updated["resumen"].as_str().unwrap().contains("Fondo global"), "{updated}");
    assert!(updated["resumen"].as_str().unwrap().contains("ilíquido"), "{updated}");
    app.assert_invalidated(&key, "update_asset").await;

    let listed = app.get_with_cookie("/v1/assets", &owner.cookie).await;
    assert_eq!(listed.status, http::StatusCode::OK);
    let rows = listed.json();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == asset_id.as_str())
        .expect("activo editado por MCP visible por HTTP")
        .clone();
    assert_eq!(row["name"], "Fondo global");
    assert_eq!(row["is_liquid"], false);
    assert_eq!(row["category_id"], cat_asset2.as_str());
    assert!(
        row.get("purchase_price").is_none_or(serde_json::Value::is_null),
        "clear_purchase_price debía borrar el precio de compra: {row}"
    );

    // purchase_price y clear_purchase_price a la vez es contradictorio → 400 sin tocar nada.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_asset",
            json!({"asset_id": asset_id, "purchase_price": "1", "clear_purchase_price": true}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");

    // --- update_liability: la asimetría que empujaba a borrar y recrear. Edita TAE y plan de
    // pago sobre la MISMA fila (misma core `patch_liability_core` que el PATCH) + FULL. --------
    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_liability",
            json!({"liability_id": liability_id, "apr_percent": "2.10", "payment_amount": "650", "payment_frequency": "monthly"}),
        ),
    )
    .await;
    let updated = tool_json(&envelope);
    assert_eq!(updated["id"], liability_id.as_str());
    app.assert_invalidated(&key, "update_liability").await;

    let listed = app.get_with_cookie("/v1/liabilities", &owner.cookie).await;
    assert_eq!(listed.status, http::StatusCode::OK);
    let rows = listed.json();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == liability_id.as_str())
        .expect("pasivo editado por MCP visible por HTTP")
        .clone();
    let apr: rust_decimal::Decimal = row["apr_percent"].as_str().unwrap().parse().unwrap();
    assert_eq!(apr, "2.10".parse::<rust_decimal::Decimal>().unwrap());
    let cuota: rust_decimal::Decimal = row["payment_amount"].as_str().unwrap().parse().unwrap();
    assert_eq!(cuota, "650".parse::<rust_decimal::Decimal>().unwrap());

    // Error de dominio compartido con el PATCH: sin ningún campo → 400 de la core.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_liability", json!({"liability_id": liability_id})),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(
        body["message"].as_str().unwrap().contains("at least one field"),
        "{body}"
    );

    // Y ambas tools pasan por require_mcp_write: toggle OFF → cortadas en vivo.
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({"mcp_write_enabled": false}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK);
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_liability", json!({"liability_id": liability_id, "apr_percent": "1.90"})),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(body["message"].as_str().unwrap().starts_with("mcp_write_disabled"), "{body}");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_asset", json!({"asset_id": asset_id, "name": "x"})),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(body["message"].as_str().unwrap().starts_with("mcp_write_disabled"), "{body}");
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

// ---------------------------------------------------------------------------
// Tramo 2 — assets / liabilities / budget / allocation / delete_recurring_rule
// ---------------------------------------------------------------------------

#[tokio::test]
async fn asset_tools_create_update_and_reject_absurd_returns() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    let cat = app.create_category(&owner, "asset", "Fondos").await;

    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_asset",
            json!({"name": "Depósito", "category_id": cat, "current_value": "10000", "expected_annual_return_percent": "3"}),
        ),
    )
    .await;
    let created = tool_json(&envelope);
    let asset_id = created["id"].as_str().unwrap().to_string();
    assert!(created["resumen"].as_str().unwrap().contains("Depósito"));
    app.assert_invalidated(&key, "create_asset").await;

    // update_asset_value: valor anterior/nuevo + FULL.
    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_asset_value",
            json!({"asset_id": asset_id, "current_value": "10500", "expected_annual_return_percent": "-20"}),
        ),
    )
    .await;
    let updated = tool_json(&envelope);
    let num = |v: &serde_json::Value| v.as_str().unwrap().parse::<f64>().unwrap();
    assert_eq!(num(&updated["valor_anterior"]), 10000.0);
    assert_eq!(num(&updated["valor_nuevo"]), 10500.0);
    assert_eq!(num(&updated["expected_annual_return_percent"]), -20.0);
    app.assert_invalidated(&key, "update_asset_value").await;

    // Cota compartida con el PATCH HTTP: retorno <= -100 → bad_request.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_asset_value",
            json!({"asset_id": asset_id, "expected_annual_return_percent": "-100"}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(body["message"].as_str().unwrap().contains("-100"));

    // Sin campos → bad_request.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_asset_value", json!({"asset_id": asset_id})),
    )
    .await;
    tool_error(&envelope, "bad_request");
}

#[tokio::test]
async fn liability_create_with_derived_principal() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "liability", "Préstamos").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    // 3.4.0: `expense_category_id` es obligatoria — con scope equivocado, mismo 400 que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_liability",
            json!({"label": "Coche", "category_id": cat, "expense_category_id": cat,
                   "principal": "1000"}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(body["message"].as_str().unwrap().contains("expense_category_id"));

    // Modo derive sin plan completo → mismos 400 que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_liability",
            json!({"label": "Coche", "category_id": cat, "expense_category_id": exp_cat,
                   "derive_principal_from_plan": true, "payment_amount": "300"}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(body["message"].as_str().unwrap().contains("payment_frequency"));

    // Plan completo → el principal se deriva (> 0) y queda marcado.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_liability",
            json!({"label": "Coche", "category_id": cat, "expense_category_id": exp_cat,
                   "derive_principal_from_plan": true,
                   "payment_amount": "300", "payment_frequency": "monthly",
                   "payment_end_date": "2028-12-01", "apr_percent": "5"}),
        ),
    )
    .await;
    let created = tool_json(&envelope);
    assert_eq!(created["principal_derived_from_plan"], true);
    assert!(created["resumen"].as_str().unwrap().contains("Coche"));
}

#[tokio::test]
async fn budget_tools_move_projection_and_validate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    let cat = app.create_category(&owner, "expense", "Ocio").await;

    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("create_budget_entry", json!({"category_id": cat, "amount": "150"})),
    )
    .await;
    let created = tool_json(&envelope);
    assert_eq!(created["amount_monthly"].as_str().unwrap().parse::<f64>().unwrap(), 150.0);
    app.assert_invalidated(&key, "create_budget_entry").await;

    // «Sube el presupuesto de ocio a 250» + exclusión mutua validada.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_budget_entry",
            json!({"id": created["id"], "amount": "250"}),
        ),
    )
    .await;
    let updated = tool_json(&envelope);
    assert_eq!(updated["amount_monthly"].as_str().unwrap().parse::<f64>().unwrap(), 250.0);

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_budget_entry",
            json!({"id": created["id"], "ends_at_retirement": true, "expense_end_date": "2030-01-01"}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");
}

#[tokio::test]
async fn allocation_rule_update_respects_sink_invariant() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;

    // Un activo + su regla sink (remainder sin cap) por HTTP.
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let rule = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({"target_asset_id": asset_id, "kind": "remainder"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(rule.status, http::StatusCode::CREATED, "{rule:?}");
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    // Capar el único sink lo destruiría → mismo error tipado que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_allocation_rule",
            json!({"rule_id": rule_id, "cap_kind": "amount", "cap_value": "5000"}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(
        body["message"].as_str().unwrap().contains("remainder_required"),
        "{body}"
    );

    // enabled=false es un cambio legal y devuelve antes/después.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_allocation_rule",
            json!({"rule_id": rule_id, "enabled": false}),
        ),
    )
    .await;
    let out = tool_json(&envelope);
    assert_eq!(out["antes"]["enabled"], true);
    assert_eq!(out["despues"]["enabled"], false);
}

#[tokio::test]
async fn delete_recurring_rule_previews_then_deletes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Gimnasio").await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-01", "concept": "gym", "amount": "-30.00", "kind": "expense", "category_id": cat, "recurring": true}),
        ),
    )
    .await;
    let created = tool_json(&envelope);
    let rule_id = created["recurring_rule_id"].as_str().unwrap().to_string();

    // Sin confirm → preview con la plantilla; NO borra.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_recurring_rule", json!({"id": rule_id})),
    )
    .await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["preview"], true);
    assert_eq!(preview["confirm_required"], true);
    assert_eq!(preview["effects"]["rule"]["id"].as_str().unwrap(), rule_id);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM recurring_transaction_rules")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "el preview no debe borrar nada");

    // Con confirm → borra; la instancia materializada sobrevive (SET NULL).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_recurring_rule", json!({"id": rule_id, "confirm": true})),
    )
    .await;
    let out = tool_json(&envelope);
    assert_eq!(out["deleted"], true);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM recurring_transaction_rules")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let txns: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM transactions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(txns >= 1, "las instancias sobreviven al borrado de la plantilla");

    // Repetir el borrado → not_found (idempotencia observable).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_recurring_rule", json!({"id": rule_id, "confirm": true})),
    )
    .await;
    tool_error(&envelope, "not_found");
}

// ---------------------------------------------------------------------------
// Tramo 3 — deletes con preview/confirm + update_fire_settings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_fire_settings_merges_field_by_field_and_is_owner_only() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Tramos fiscales personalizados por HTTP (objeto completo, como hace la SPA).
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {
                "fire_number_mode": "annual_expense",
                "fire_number_manual_amount": null,
                "swr_pct": "4",
                "taxes_enabled": true,
                "tax_brackets": [{"up_to": "10000", "pct": "10"}, {"up_to": null, "pct": "25"}],
                "savings_source": "budget"
            }}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    // Preview (sin confirm): before/after validado, nada persiste.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_fire_settings", json!({"swr_pct": "3.0"})),
    )
    .await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["preview"], true);
    assert_eq!(preview["effects"]["before"]["swr_pct"], "4");
    assert_eq!(preview["effects"]["after"]["swr_pct"], "3.0");
    let stored = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(stored.json()["installation"]["fire_settings"]["swr_pct"], "4");

    // Confirm: cambia SOLO swr — los tax_brackets personalizados sobreviven (el bug del
    // #[serde(default)] que un PATCH parcial por HTTP sí dispararía).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_fire_settings", json!({"swr_pct": "3.0", "confirm": true})),
    )
    .await;
    let applied = tool_json(&envelope);
    assert_eq!(applied["applied"], true);
    let stored = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    let fs = stored.json()["installation"]["fire_settings"].clone();
    assert_eq!(fs["swr_pct"], "3.0");
    assert_eq!(
        fs["tax_brackets"].as_array().unwrap().len(),
        2,
        "los tramos personalizados NO se resetean: {fs}"
    );
    assert_eq!(fs["tax_brackets"][0]["pct"], "10");

    // Cotas del PATCH real re-aplicadas.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_fire_settings", json!({"swr_pct": "5", "confirm": true})),
    )
    .await;
    tool_error(&envelope, "bad_request");

    // Member (escritor) NO puede: el gate es Owner, no role_can_write.
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let member_token = create_token_for(&app, &member.cookie).await;
    let envelope = mcp_post(
        &app,
        &member_token,
        tool_call("update_fire_settings", json!({"swr_pct": "3.5", "confirm": true})),
    )
    .await;
    tool_error(&envelope, "forbidden");

    // Cambiar savings_source por MCP invalida la proyección (FULL).
    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_fire_settings",
            json!({"savings_source": "transactions_avg", "confirm": true}),
        ),
    )
    .await;
    let _ = tool_json(&envelope);
    app.assert_invalidated(&key, "update_fire_settings").await;
}

#[tokio::test]
async fn destructive_deletes_preview_then_execute() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;

    // Transacción vinculada a un asset → el preview de delete_asset cuenta el SET NULL.
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let txn = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-07-01", "concept": "aporte", "amount": "-200.00",
                   "kind": "savings", "linked_asset_id": asset_id}),
            &owner.cookie,
        )
        .await;
    assert_eq!(txn.status, http::StatusCode::CREATED, "{txn:?}");
    let txn_id = txn.json()["id"].as_str().unwrap().to_string();

    // delete_transaction: preview trae el movimiento completo; confirm borra.
    let envelope = mcp_post(&app, &token, tool_call("delete_transaction", json!({"id": txn_id}))).await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["preview"], true);
    assert_eq!(preview["effects"]["transaction"]["concept"], "aporte");
    assert_eq!(app.count_rows("transactions").await, 1);
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_transaction", json!({"id": txn_id, "confirm": true})),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_eq!(app.count_rows("transactions").await, 0);

    // Re-crear la transacción vinculada para el preview del asset.
    let txn = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-07-02", "concept": "aporte2", "amount": "-100.00",
                   "kind": "savings", "linked_asset_id": asset_id}),
            &owner.cookie,
        )
        .await;
    assert_eq!(txn.status, http::StatusCode::CREATED);

    // delete_asset: preview con efectos (1 transacción a desvincular); confirm borra el asset
    // y la transacción sobrevive desvinculada.
    let envelope = mcp_post(&app, &token, tool_call("delete_asset", json!({"id": asset_id}))).await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["effects"]["unlinked"]["transactions_unlinked"], 1, "{preview}");
    assert_eq!(app.count_rows("assets").await, 1);
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_asset", json!({"id": asset_id, "confirm": true})),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_eq!(app.count_rows("assets").await, 0);
    assert_eq!(app.count_rows("transactions").await, 1, "la transacción sobrevive");
    let unlinked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM transactions WHERE linked_asset_id IS NULL",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(unlinked, 1, "desvinculada, no borrada");

    // delete_snapshot: capture + preview (items_deleted) + confirm.
    let asset2 = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Otro", "current_value": "500"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset2.status, http::StatusCode::CREATED);
    let snap = tool_json(
        &mcp_post(&app, &token, tool_call("capture_snapshot", json!({"kinds": ["asset"]}))).await,
    );
    let snap_id = snap["snapshots"][0]["id"].as_str().unwrap().to_string();
    let envelope = mcp_post(&app, &token, tool_call("delete_snapshot", json!({"id": snap_id}))).await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["effects"]["snapshot"]["items_deleted"], 1, "{preview}");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_snapshot", json!({"id": snap_id, "confirm": true})),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_eq!(app.count_rows("history_snapshots").await, 0);

    // delete_budget_entry y delete_planning_flow: preview → confirm.
    let entry = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("create_budget_entry", json!({"category_id": cat_exp, "amount": "100"})),
        )
        .await,
    );
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_budget_entry", json!({"id": entry["id"]})),
    )
    .await;
    assert_eq!(tool_json(&envelope)["preview"], true);
    assert_eq!(app.count_rows("budget_entries").await, 1);
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_budget_entry", json!({"id": entry["id"], "confirm": true})),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_eq!(app.count_rows("budget_entries").await, 0);

    let flow = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_planning_flow",
                json!({"title": "Viaje", "category_id": cat_exp, "expected_amount": "600"}),
            ),
        )
        .await,
    );
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_planning_flow", json!({"id": flow["id"], "confirm": true})),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_eq!(app.count_rows("planning_flows").await, 0);
}

#[tokio::test]
async fn delete_import_previews_txn_count_and_cascades() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Import CSV de 2 filas por HTTP (preview → confirm, patrón de transactions_projection_cache).
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               01/06/2026;01/06/2026;SUPER;-10,00;EUR\n\
               02/06/2026;02/06/2026;LUZ;-20,00;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({"source": "myinvestor", "file_b64": b64}),
            &owner.cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({"source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                   "decisions": [{"kind": "expense"}, {"kind": "expense"}], "learn_rules": false}),
            &owner.cookie,
        )
        .await;
    assert!(c.status.is_success(), "{c:?}");
    assert_eq!(app.count_rows("transactions").await, 2);

    let batches = tool_json(
        &mcp_post(&app, &token, tool_call("list_transaction_imports", json!({}))).await,
    );
    let import_id = batches[0]["id"].as_str().unwrap().to_string();

    let envelope = mcp_post(&app, &token, tool_call("delete_import", json!({"id": import_id}))).await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["effects"]["transactions_deleted"], 2, "{preview}");
    assert_eq!(app.count_rows("transactions").await, 2, "el preview no borra");

    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_import", json!({"id": import_id, "confirm": true})),
    )
    .await;
    let _ = tool_json(&envelope);
    assert_eq!(app.count_rows("transactions").await, 0, "cascada del lote");
    assert_eq!(app.count_rows("transaction_imports").await, 0);
}

// ---------------------------------------------------------------------------
// Conciliación de transferencias (3.5.0): tools reconcile_transfers / unreconcile_transfer
// ---------------------------------------------------------------------------

/// Las dos tools nuevas respetan el toggle vivo y el rol, comparten core con HTTP (cero deriva)
/// y siguen el contrato de cache COND: en modo B, desconciliar por MCP invalida.
#[tokio::test]
async fn reconcile_tools_share_core_and_respect_write_gates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Par auto-conciliable creado por HTTP (−120/+120 a 1 día). El alta de la segunda pata ya
    // concilia → el pase MCP debe ser punto fijo (0 pares).
    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-10", "concept": "Salida", "amount": "-120", "kind": "expense" }),
            &owner.cookie,
        )
        .await
        .json();
    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-11", "concept": "Entrada", "amount": "120", "kind": "income" }),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(b["transfer_counterpart_id"], a["id"], "precondición: conciliadas");

    let envelope = mcp_post(&app, &token, tool_call("reconcile_transfers", json!({}))).await;
    let body = tool_json(&envelope);
    assert_eq!(body["pairs_created"].as_u64(), Some(0), "punto fijo vía MCP: {body}");

    // Desconciliar por MCP (modo B para verificar la invalidación COND).
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    app.warm_household(&owner.cookie, &key).await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("unreconcile_transfer", json!({"transaction_id": a["id"]})),
    )
    .await;
    let body = tool_json(&envelope);
    assert!(body["transaction"]["transfer_counterpart_id"].is_null(), "{body}");
    assert!(body["counterpart"]["transfer_counterpart_id"].is_null(), "{body}");
    app.assert_invalidated(&key, "unreconcile_transfer").await;

    // La fila desconciliada por MCP es indistinguible por HTTP (mismo core).
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await
        .json();
    for t in list.as_array().unwrap() {
        assert!(t["transfer_counterpart_id"].is_null(), "sueltas también por HTTP: {t}");
    }

    // Repetir el desconcilie → 400 not_reconciled compartido con HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("unreconcile_transfer", json!({"transaction_id": a["id"]})),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(body["message"].as_str().unwrap().contains("not_reconciled"), "{body}");

    // Toggle vivo: con la escritura MCP apagada, el pase se corta con mcp_write_disabled.
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({"mcp_write_enabled": false}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let envelope = mcp_post(&app, &token, tool_call("reconcile_transfers", json!({}))).await;
    let body = tool_error(&envelope, "bad_request");
    assert!(
        body["message"].as_str().unwrap().contains("mcp_write_disabled"),
        "{body}"
    );
}

/// Cuarteto de la tool `apply_categorization_rule` (3.8.0): preview no persiste, confirm ejecuta,
/// la escritura es indistinguible de la del endpoint HTTP, el contrato de cache es COND y el
/// toggle vivo `mcp_write_enabled` corta.
#[tokio::test]
async fn apply_categorization_rule_previews_then_executes_and_respects_gates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat_ast = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat_ast, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    // Modo C: las transacciones son inputs del engine → el backfill debe invalidar.
    app.patch_json_with_cookie(
        "/v1/installation",
        json!({ "fire_settings": { "savings_source": "budget_income_real_expense" } }),
        &owner.cookie,
    )
    .await;

    // Filas que la regla reclasificará a `expense`. Nacen POSITIVAS y como `income` a propósito:
    // es el caso real de una devolución (llega en positivo, el importador la marca `income` por el
    // signo) que se recategoriza a gasto para que **netee** contra el gasto del mes. Desde 4.0.0 el
    // alta manual exige que el signo cuadre con el kind, pero la recategorización —en lote o por
    // regla— sigue pudiendo dejar un `expense` positivo: es contabilidad correcta, y por eso el
    // guard no alcanza a esta ruta (issue #7 §3).
    for concept in ["WWW.AMAZON* AAA", "AMAZON PRIME"] {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({ "op_date": "2026-06-10", "concept": concept, "amount": "20",
                        "kind": "income" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    let rule = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({ "match_kind": "substring", "pattern": "AMAZON", "assign_kind": "expense",
                    "assign_category_id": compras }),
            &owner.cookie,
        )
        .await;
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    let iid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let key = app.household_key(iid);

    // 1. PREVIEW: no escribe y no invalida.
    app.warm_household(&owner.cookie, &key).await;
    let preview = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "apply_categorization_rule",
                json!({"rule_id": rule_id, "apply_to_existing": "all"}),
            ),
        )
        .await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["effects"]["would_match"], 2, "{preview}");
    assert_eq!(preview["effects"]["would_change_kind"], 2, "{preview}");
    assert_eq!(
        preview["effects"]["moves_projection_in_modes_b_and_c"], true,
        "el aviso de proyección debe salir ANTES de ejecutar: {preview}"
    );
    assert_eq!(preview["effects"]["sample"].as_array().unwrap().len(), 2);
    assert!(app.cache_contains(&key).await, "el preview no debe invalidar");
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["kind"], "income", "el preview no debe escribir: {t}");
    }

    // 2. CONFIRM: escribe, y el resultado es indistinguible vía HTTP.
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "apply_categorization_rule",
                json!({"rule_id": rule_id, "apply_to_existing": "all", "confirm": true}),
            ),
        )
        .await,
    );
    assert_eq!(out["updated"], 2, "{out}");
    assert_eq!(out["resumen"].as_array().unwrap().len(), 2, "{out}");
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["kind"], "expense", "{t}");
        assert_eq!(t["category_id"], json!(compras), "{t}");
    }

    // 3. Contrato de cache: COND, y en modo C invalida.
    app.assert_invalidated(&key, "apply_categorization_rule por MCP").await;

    // 4. Toggle vivo: con la escritura desactivada, la tool corta.
    app.patch_json_with_cookie(
        "/v1/installation",
        json!({"mcp_write_enabled": false}),
        &owner.cookie,
    )
    .await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "apply_categorization_rule",
            json!({"rule_id": rule_id, "apply_to_existing": "all", "confirm": true}),
        ),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
    assert!(
        envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("mcp_write_disabled"),
        "{envelope}"
    );

    // 5. Un viewer nunca escribe (rol vivo, sin congelar en el token).
    app.patch_json_with_cookie(
        "/v1/installation",
        json!({"mcp_write_enabled": true}),
        &owner.cookie,
    )
    .await;
    let viewer = app
        .register_and_approve_member(&owner, "victor", "viewer")
        .await;
    let viewer_token = create_token_for(&app, &viewer.cookie).await;
    let envelope = mcp_post(
        &app,
        &viewer_token,
        tool_call(
            "apply_categorization_rule",
            json!({"rule_id": rule_id, "apply_to_existing": "all", "confirm": true}),
        ),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
}

/// Cuarteto de `update_transactions` (3.8.0): escritura indistinguible del PATCH HTTP en lote,
/// todo-o-nada, contrato COND y toggle vivo.
#[tokio::test]
async fn update_transactions_batch_shares_core_and_respects_gates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat_ast = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat_ast, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    app.patch_json_with_cookie(
        "/v1/installation",
        json!({ "fire_settings": { "savings_source": "transactions_avg" } }),
        &owner.cookie,
    )
    .await;

    // Positivas y `income`: el lote las pasará a `expense`, que es la reclasificación de una
    // devolución. El alta manual ya no acepta un `income` negativo (`amount_sign_mismatch`), y el
    // lote sigue sin poder tocar el importe — solo reclasifica.
    let mut ids = Vec::new();
    for i in 0..3 {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({ "op_date": format!("2026-06-{:02}", 10 + i),
                        "concept": format!("COMPRA {i}"), "amount": "10", "kind": "income" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
        ids.push(r.json()["id"].as_str().unwrap().to_string());
    }

    let iid: uuid::Uuid = sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let key = app.household_key(iid);
    app.warm_household(&owner.cookie, &key).await;

    // 1. Escritura por la tool.
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_transactions",
                json!({"ids": ids, "kind": "expense", "category_id": compras}),
            ),
        )
        .await,
    );
    assert_eq!(out["updated"], 3, "{out}");
    assert_eq!(out["resumen"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(out["resumen_truncated"], false, "{out}");

    // 2. Indistinguible vía HTTP.
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["kind"], "expense", "{t}");
        assert_eq!(t["category_id"], json!(compras), "{t}");
    }

    // 3. Cache COND en modo B.
    app.assert_invalidated(&key, "update_transactions por MCP").await;

    // 4. Todo o nada con un id inventado: cero filas tocadas y el error lo nombra.
    let fake = uuid::Uuid::new_v4().to_string();
    let mut mixed = ids.clone();
    mixed.push(fake.clone());
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_transactions",
            json!({"ids": mixed, "notes": "no debería aplicarse"}),
        ),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
    let text = envelope["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&fake), "el error debe nombrar el id culpable: {text}");
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert!(t["notes"].is_null(), "ninguna nota debía escribirse: {t}");
    }

    // 5. Toggle vivo.
    app.patch_json_with_cookie(
        "/v1/installation",
        json!({"mcp_write_enabled": false}),
        &owner.cookie,
    )
    .await;
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_transactions", json!({"ids": ids, "notes": "x"})),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");
    assert!(envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("mcp_write_disabled"));
}
