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

/// Confirma en DOS FASES una tool con `confirm_token` (Fase 3, issue #84): previsualiza, saca el
/// token del preview y repite la llamada con `confirm` + `confirm_token`. Devuelve el envelope de
/// la confirmación.
async fn preview_then_confirm(
    app: &TestApp,
    bearer: &str,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let preview = tool_json(&mcp_post(app, bearer, tool_call(name, args.clone())).await);
    assert_eq!(
        preview["preview"], true,
        "se esperaba un preview de {name}: {preview}"
    );
    let ct = preview["confirm_token"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} debe emitir confirm_token en su preview: {preview}"))
        .to_string();
    let mut confirmed = args;
    confirmed["confirm"] = json!(true);
    confirmed["confirm_token"] = json!(ct);
    mcp_post(app, bearer, tool_call(name, confirmed)).await
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
    assert!(created["summary"].as_str().unwrap().contains("cena"), "{created}");
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
    let key = app.default_view_key(iid, owner.user_id);
    let cat = app.create_category(&owner, "expense", "Comida").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;

    // --- Modo A (budget): create_transaction vía MCP NO invalida (COND inactiva) -------------
    app.warm_default_view(&owner.cookie, &key).await;
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
    assert!(flow["summary"].as_str().unwrap().contains("IRPF"));
    app.assert_invalidated(&key, "create_planning_flow").await;

    // --- Modo B: create_transaction vía MCP SÍ invalida (COND activa) -------------------------
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    app.warm_default_view(&owner.cookie, &key).await;
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
    let key = app.default_view_key(iid, owner.user_id);

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
            // Francés explícito con plan (#144): declarar TIN exige un modelo que devengue.
            json!({"label": "Hipoteca", "category_id": cat_liab, "expense_category_id": cat_exp, "principal": "120000", "repayment_model": "french", "apr_percent": "3.10", "payment_amount": "600", "payment_frequency": "monthly"}),
        ),
    )
    .await;
    let liability_id = tool_json(&envelope)["id"].as_str().unwrap().to_string();

    // --- update_asset: body completo (rename + recategorizar + iliquidez + borrar el precio
    // de compra) y contrato FULL — misma core `patch_asset_core` que el PATCH HTTP. ------------
    app.warm_default_view(&owner.cookie, &key).await;
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
    assert!(updated["summary"].as_str().unwrap().contains("Fondo global"), "{updated}");
    assert!(updated["summary"].as_str().unwrap().contains("ilíquido"), "{updated}");
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
    app.warm_default_view(&owner.cookie, &key).await;
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
    // El modelo de amortización viaja en el listado desde 4.2.0; desde #144 la fila nace
    // francesa (el alta con TIN lo exige) y el update no la ha movido.
    assert_eq!(row["repayment_model"], "french");

    // «Mi préstamo es francés»: fijar por MCP el modelo que ya tiene es un no-op válido (la
    // fila tiene TIN 2,10 y cuota mensual 650, estado coherente) y se ve por HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_liability",
            json!({"liability_id": liability_id, "repayment_model": "french"}),
        ),
    )
    .await;
    tool_json(&envelope);
    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == liability_id.as_str())
        .expect("pasivo visible")
        .clone();
    assert_eq!(row["repayment_model"], "french");

    // Un literal fuera del dominio no llega a la core: por MCP el parámetro es un String suelto
    // (no hay serde que lo rechace como en HTTP), así que el 400 es nuestro y nombra las opciones.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_liability",
            json!({"liability_id": liability_id, "repayment_model": "aleman"}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    let msg = body["message"].as_str().unwrap();
    assert!(msg.starts_with("repayment_model_invalid"), "{body}");
    assert!(msg.contains("must be one of"), "{body}");

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

/// Fase 5 (issue #86) — **`type_tag` deja de ser una dimensión de solo lectura**.
///
/// `get_summary.liabilities_by_type_tag` desglosa la deuda por una etiqueta que el usuario
/// escribe libremente… y que hasta ahora ninguna tool podía escribir: `create_liability` y
/// `update_liability` mandaban `type_tag: None` fijo. Desde MCP el desglose existía, pero todos
/// los pasivos que un agente diera de alta caían en la línea `type_tag: null` y no había forma
/// de sacarlos de ahí sin abrir la SPA. Es el hueco exacto que la §2.2 de la skill de paridad
/// llama «una dimensión que se lee pero no se escribe».
///
/// Cubre el ciclo entero: alta con etiqueta → aparece en el desglose de `get_summary` → cambio
/// de etiqueta → borrado con cadena vacía (el tri-estado del PATCH sin inventar un `clear_*`).
#[tokio::test]
async fn liability_type_tag_is_writable_and_reaches_the_summary_breakdown() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    let cat_liab = app.create_category(&owner, "liability", "Préstamos").await;
    let cat_exp = app.create_category(&owner, "expense", "Vivienda").await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_liability",
            json!({
                "label": "Hipoteca", "type_tag": "hipoteca", "category_id": cat_liab,
                "expense_category_id": cat_exp, "principal": "120000",
            }),
        ),
    )
    .await;
    let liability_id = tool_json(&envelope)["id"].as_str().unwrap().to_string();

    // Misma core que el POST HTTP: la fila es indistinguible por HTTP.
    let row_tag = |app_rows: serde_json::Value| -> serde_json::Value {
        app_rows
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == liability_id.as_str())
            .expect("pasivo visible por HTTP")["type_tag"]
            .clone()
    };
    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    assert_eq!(row_tag(rows), "hipoteca");

    // Y llega al desglose que sólo se podía LEER desde MCP.
    let summary = tool_json(
        &mcp_post(&app, &token, tool_call("get_summary", json!({}))).await,
    );
    let line = summary["liabilities_by_type_tag"]
        .as_array()
        .expect("liabilities_by_type_tag")
        .iter()
        .find(|l| l["type_tag"] == "hipoteca")
        .cloned()
        .unwrap_or_else(|| panic!("sin línea 'hipoteca': {summary}"));
    assert_eq!(line["type_tag"], "hipoteca", "{line}");

    // Cambiar la etiqueta.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_liability",
            json!({"liability_id": liability_id, "type_tag": "vivienda"}),
        ),
    )
    .await;
    tool_json(&envelope);
    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    assert_eq!(row_tag(rows), "vivienda");

    // Cadena vacía = borrar (el tri-estado del PATCH; `None` conservaría la actual). El campo
    // desaparece del wire, que es como el handler publica «sin etiqueta».
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_liability", json!({"liability_id": liability_id, "type_tag": ""})),
    )
    .await;
    tool_json(&envelope);
    let rows = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    assert!(
        row_tag(rows).is_null(),
        "la cadena vacía debía borrar el type_tag"
    );
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

    // Desde la Fase 3 poda bajo dos fases: el preview declara que NO puede dar cifras y emite el
    // token; la confirmación lo consume.
    let preview = tool_json(
        &mcp_post(&app, &token, tool_call("materialize_recurring", json!({}))).await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert!(
        preview["effects"]["side_effects"]["would_prune"].is_null(),
        "el preview no puede inventarse un número que la core no sabe dar: {preview}"
    );
    assert_eq!(preview["effects"]["side_effects"]["your_recurring_rules"], 1, "{preview}");
    let envelope = preview_then_confirm(&app, &token, "materialize_recurring", json!({})).await;
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
            json!({"pattern": "TIENDA MASCOTAS NORTE", "source": "myinvestor", "assign_kind": "expense", "assign_category_id": cat_id}),
        ),
    )
    .await;
    let rule = tool_json(&envelope);
    assert!(rule["summary"].as_str().unwrap().contains("TIENDA MASCOTAS NORTE"), "{rule}");

    // Duplicado (source, pattern) con source concreto → conflict (la UNIQUE de la tabla; con
    // source NULL Postgres no colisiona — contrato idéntico al HTTP).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_categorization_rule",
            json!({"pattern": "TIENDA MASCOTAS NORTE", "source": "myinvestor", "assign_kind": "expense", "assign_category_id": cat_id}),
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
    assert!(flow["summary"].as_str().unwrap().contains("2026-10-15"));

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
    assert!(!updated["summary"].as_str().unwrap().contains("2026-10-15"), "{updated}");
    assert!(updated["summary"].as_str().unwrap().contains("900"));

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
    let key = app.default_view_key(iid, owner.user_id);
    let cat = app.create_category(&owner, "asset", "Fondos").await;

    app.warm_default_view(&owner.cookie, &key).await;
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
    assert!(created["summary"].as_str().unwrap().contains("Depósito"));
    app.assert_invalidated(&key, "create_asset").await;

    // update_asset_value: valor anterior/nuevo + FULL.
    app.warm_default_view(&owner.cookie, &key).await;
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
            // Francés explícito desde #144 (declarar TIN exige un modelo que devengue); el
            // principal derivado pasa a ser el valor actual al 5 %, que es lo que este test
            // quiere: la derivación de verdad, no la Σ ingenua.
            json!({"label": "Coche", "category_id": cat, "expense_category_id": exp_cat,
                   "derive_principal_from_plan": true, "repayment_model": "french",
                   "payment_amount": "300", "payment_frequency": "monthly",
                   "payment_end_date": "2028-12-01", "apr_percent": "5"}),
        ),
    )
    .await;
    let created = tool_json(&envelope);
    assert_eq!(created["principal_derived_from_plan"], true);
    assert!(created["summary"].as_str().unwrap().contains("Coche"));
}

#[tokio::test]
async fn budget_tools_move_projection_and_validate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    let cat = app.create_category(&owner, "expense", "Ocio").await;

    app.warm_default_view(&owner.cookie, &key).await;
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
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    // #150: "Fondo" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.
    let rule_id = app.sink_rule_id(&owner.cookie).await;

    // Capar el único sink lo destruiría → mismo error tipado que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_allocation_rule",
            json!({"id": rule_id, "cap_kind": "amount", "cap_value": "5000"}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(
        body["message"].as_str().unwrap().contains("remainder_required"),
        "{body}"
    );

    // INVERTIDO en 4.12.1 (#176): deshabilitar el ÚNICO sumidero era legal (el sobrante caía a
    // surplus_cash); con la caja muerta lo dejaría sin destino, así que ahora es el MISMO 400
    // que caparlo. La salida legal sigue siendo mover la regla de activo (target_asset_id).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_allocation_rule",
            json!({"id": rule_id, "enabled": false}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert!(
        body["message"].as_str().unwrap().contains("remainder_required"),
        "{body}"
    );
}

/// REGRESIÓN (auditoría MCP §5) — `cap_value` sin `cap_kind` ya no se evapora con un 200.
///
/// El repro literal del issue: `{rule_id, enabled: true, cap_value: "99999"}` devolvía 200 con
/// `antes == despues`. La guardia de «al menos un campo» enumeraba a mano `amount`/`cap_kind`/
/// `clear_cap`/`enabled` y no nombraba `cap_value`, y el mapeo del cap solo lo leía si venía
/// `cap_kind`: con `enabled` presente la llamada pasaba la guardia y el tope se perdía por el
/// camino. Un agente que tradujera «ponle un tope de 99.999 € a la cartera» recibía un éxito y le
/// decía al usuario «hecho», sin que nada hubiera cambiado.
#[tokio::test]
async fn allocation_rule_update_never_drops_a_half_cap_silently() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    // #150: "Fondo" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.
    let rule_id = app.sink_rule_id(&owner.cookie).await;

    // Las dos medias parejas dan el MISMO error. Antes solo lo daba una de las dos.
    for half in [
        json!({"id": rule_id, "enabled": true, "cap_value": "99999"}),
        json!({"id": rule_id, "enabled": true, "cap_kind": "amount"}),
        json!({"id": rule_id, "cap_value": "99999"}),
        json!({"id": rule_id, "cap_kind": "amount"}),
    ] {
        let envelope = mcp_post(
            &app,
            &token,
            tool_call("update_allocation_rule", half.clone()),
        )
        .await;
        let body = tool_error(&envelope, "bad_request");
        assert_eq!(body["code"], "cap_pair_incomplete", "{half}: {body}");
    }

    // Poner y quitar el tope a la vez tampoco se resuelve por ti.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_allocation_rule",
            json!({"id": rule_id, "cap_kind": "amount", "cap_value": "1", "clear_cap": true}),
        ),
    )
    .await;
    assert_eq!(tool_error(&envelope, "bad_request")["code"], "cap_set_and_clear");

    // Y un cuerpo sin nada que actualizar da `patch_empty` — la guardia vive ahora en la core, así
    // que HTTP y MCP responden lo mismo.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_allocation_rule", json!({"id": rule_id})),
    )
    .await;
    assert_eq!(tool_error(&envelope, "bad_request")["code"], "patch_empty");

    let http_empty = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{rule_id}"),
            json!({}),
            &owner.cookie,
        )
        .await;
    assert_eq!(http_empty.status, http::StatusCode::BAD_REQUEST, "{http_empty:?}");
    assert_eq!(http_empty.json()["code"], "patch_empty");
}

/// REGRESIÓN (auditoría MCP §11b) — el `summary` de un flujo planificado habla el idioma del wire.
///
/// `PlanningFlowDirection` solo tenía `Debug`, y un `{:?}` en el `format!` publicaba el
/// identificador de Rust: las escrituras devolvían `"… (Outflow)"` —inglés y capitalizado— mientras
/// las lecturas devolvían `"direction":"outflow"`. Dos formas del mismo valor en el mismo catálogo.
#[tokio::test]
async fn planning_flow_summary_uses_the_wire_form_of_the_direction() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // El sentido lo da el scope de la categoría, no un parámetro: una categoría de gasto produce
    // un `outflow`.
    let cat = app.create_category(&owner, "expense", "Coche").await;
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_planning_flow",
                json!({"title": "Coche", "category_id": cat, "expected_amount": "123.45"}),
            ),
        )
        .await,
    );
    let summary = out["summary"].as_str().expect("summary");
    assert!(summary.contains("(outflow)"), "{summary}");
    assert!(!summary.contains("Outflow"), "el Debug de Rust no debe salir al wire: {summary}");

    // Y coincide con lo que devuelve la lectura para la misma fila.
    // Fase 5: los listados van envueltos con el eco de la vista aplicada.
    let flows = tool_json(
        &mcp_post(&app, &token, tool_call("list_planning_flows", json!({"view": "household"}))).await,
    );
    assert_eq!(flows["view"], "household", "{flows}");
    assert_eq!(flows["planning_flows"][0]["direction"], "outflow", "{flows}");
}

/// Cuarteto de `update_categorization_rule`: core compartida, cache NONE, errores de dominio con
/// el código del wire, y las dos puertas de escritura.
///
/// Cierra el hueco #4 del registro de paridad: desde 3.8.0 un agente podía CREAR una regla y
/// aplicarla retroactivamente a cientos de movimientos, pero no corregirla ni retirarla. La
/// asimetría empujaba a acumular reglas nuevas encima de las malas, que es lo que se ve en los
/// la práctica ya enseñaba: una misma floristería repartida entre tres categorías.
#[tokio::test]
async fn update_categorization_rule_shares_core_and_rejects_ambiguous_tristate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    // Modo B: las transacciones SÍ son inputs del engine, así que si esta tool invalidara la cache
    // se notaría. En modo A no invalida nada nunca y el test no valdría de nada.
    set_mode(&app, &owner.cookie, "transactions_avg").await;

    let rule_id = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({"match_kind": "substring", "pattern": "FLORISTERIA", "source": "n26",
                   "assign_kind": "expense", "assign_category_id": compras}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Una segunda regla, para provocar la colisión de (source, pattern) más abajo.
    app.post_json_with_cookie(
        "/v1/transactions/rules",
        json!({"match_kind": "substring", "pattern": "AMAZON", "source": "n26",
               "assign_kind": "expense"}),
        &owner.cookie,
    )
    .await;

    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    app.warm_default_view(&owner.cookie, &key).await;

    // 1. Escritura por la tool + tri-estado: `clear_source` la hace agnóstica del banco.
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_categorization_rule",
                json!({"id": rule_id, "pattern": "FLORISTERIA LA GLORIETA",
                       "clear_source": true, "clear_assign_category": true}),
            ),
        )
        .await,
    );
    assert_eq!(out["pattern"], "FLORISTERIA LA GLORIETA", "{out}");
    assert!(out["source"].is_null(), "clear_source debe dejarla agnóstica: {out}");
    assert!(out["assign_category_id"].is_null(), "{out}");

    // 2. Indistinguible vía HTTP: es la misma core.
    let rows = app
        .get_with_cookie("/v1/transactions/rules", &owner.cookie)
        .await
        .json();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(rule_id))
        .expect("la regla sigue ahí");
    assert_eq!(row["pattern"], "FLORISTERIA LA GLORIETA", "{row}");
    assert!(row["source"].is_null(), "{row}");

    // 3. Contrato de cache: NONE. Editar una regla no recategoriza nada, así que el conjunto de
    //    transacciones no se mueve y la proyección no puede cambiar — ni siquiera en modo B.
    assert!(
        app.cache_contains(&key).await,
        "editar una regla NUNCA invalida la cache de proyección"
    );

    // 4. Errores de dominio, con el código del wire.
    for (body, code) in [
        (json!({"id": rule_id}), "rule_patch_empty"),
        (
            json!({"id": rule_id, "source": "n26", "clear_source": true}),
            "rule_patch_conflict",
        ),
        (
            json!({"id": rule_id, "assign_kind": "expense", "clear_assign_kind": true}),
            "rule_patch_conflict",
        ),
        (json!({"id": rule_id, "match_kind": "regex"}), "rule_match_kind_invalid"),
    ] {
        let envelope = mcp_post(
            &app,
            &token,
            tool_call("update_categorization_rule", body.clone()),
        )
        .await;
        let err = tool_error(&envelope, "bad_request");
        assert_eq!(err["code"], code, "{body}: {err}");
    }
    // Colisión (source, pattern) con la otra regla → conflict, igual que HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_categorization_rule",
            json!({"id": rule_id, "pattern": "AMAZON", "source": "n26"}),
        ),
    )
    .await;
    tool_error(&envelope, "conflict");
    // Una regla de otro (o inexistente) es 404, nunca 403.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_categorization_rule",
            json!({"id": uuid::Uuid::new_v4().to_string(), "pattern": "X"}),
        ),
    )
    .await;
    tool_error(&envelope, "not_found");

    // 5. Toggle vivo y rol.
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
            "update_categorization_rule",
            json!({"id": rule_id, "pattern": "OTRA"}),
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
}

/// `delete_categorization_rule`: preview con la huella real, confirm que borra, y la invariante que
/// más importa — **los movimientos conservan su categoría**.
///
/// Sin esa aserción el preview sería peligroso: un LLM que lea «40 movimientos» dirá al usuario «se
/// descategorizarán 40 movimientos», que es falso. Por eso la respuesta lleva la nota, y por eso la
/// cifra que se publica primero es `ya_conformes` y no `cambiarian`: una regla ya aplicada tiene
/// `cambiarian: 0` y aun así gobierna decenas de filas.
#[tokio::test]
async fn delete_categorization_rule_previews_then_deletes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    set_mode(&app, &owner.cookie, "transactions_avg").await;

    // Tres movimientos que la regla YA gobierna (categoría correcta): `ya_conformes = 3`,
    // `cambiarian = 0`.
    for i in 0..3 {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({"op_date": format!("2026-06-{:02}", 10 + i),
                       "concept": format!("TIENDA {i}"), "amount": "-10", "kind": "expense",
                       "category_id": compras}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let rule_id = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({"match_kind": "substring", "pattern": "TIENDA",
                   "assign_kind": "expense", "assign_category_id": compras}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    app.warm_default_view(&owner.cookie, &key).await;

    // 1. Sin confirm: preview, no borra, y la huella cuadra con lo sembrado.
    let preview = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("delete_categorization_rule", json!({"id": rule_id})),
        )
        .await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["confirm_required"], true, "{preview}");
    assert_eq!(preview["effects"]["entity"]["id"], json!(rule_id), "{preview}");
    assert_eq!(preview["effects"]["side_effects"]["already_correct"], 3, "{preview}");
    assert_eq!(
        preview["effects"]["side_effects"]["would_match"], 0,
        "una regla ya aplicada no cambiaría nada — por eso `already_correct` es la cifra útil: \
         {preview}"
    );
    assert_eq!(app.count_rows("categorization_rules").await, 1, "el preview no borra");
    assert!(app.cache_contains(&key).await, "el preview no invalida");

    // 2. Con confirm: borra… y los movimientos CONSERVAN su categoría.
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "delete_categorization_rule",
                json!({"id": rule_id, "confirm": true}),
            ),
        )
        .await,
    );
    assert_eq!(out["deleted"], true, "{out}");
    assert_eq!(app.count_rows("categorization_rules").await, 0);
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(
            t["category_id"],
            json!(compras),
            "borrar la regla NO descategoriza: {t}"
        );
    }

    // 3. Cache NONE también al borrar (modo B, donde sí se notaría).
    assert!(
        app.cache_contains(&key).await,
        "borrar una regla NUNCA invalida la cache de proyección"
    );

    // 4. Idempotencia observable: repetir con confirm da not_found.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_categorization_rule",
            json!({"id": rule_id, "confirm": true}),
        ),
    )
    .await;
    tool_error(&envelope, "not_found");
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
    assert_eq!(preview["effects"]["entity"]["id"].as_str().unwrap(), rule_id);
    assert_eq!(
        preview["effects"]["side_effects"]["materialized_instances_deleted"], 0,
        "borrar la plantilla no toca las instancias: {preview}"
    );
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

    // 5.0.0: `fire_settings` ya NO lleva swr_pct, fire_number_mode, fire_number_manual_amount ni
    // horizon_lifespan_age — esos cuatro ejes son del perfil de jubilación de cada usuario (D13).
    // Lo que queda aquí es lo COMPARTIDO por el hogar.
    //
    // Tramos fiscales personalizados por HTTP (objeto completo, como hace la SPA).
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {
                "taxes_enabled": true,
                "tax_brackets": [{"up_to": "10000", "pct": "10"}, {"up_to": null, "pct": "25"}],
                "savings_source": "budget",
                "taxable_gain_ratio": "1"
            }}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    // Preview (sin confirm): before/after validado, nada persiste.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_fire_settings", json!({"taxable_gain_ratio": "0.5"})),
    )
    .await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["preview"], true);
    assert_eq!(preview["effects"]["entity"]["before"]["taxable_gain_ratio"], "1");
    assert_eq!(preview["effects"]["entity"]["after"]["taxable_gain_ratio"], "0.5");
    assert_eq!(preview["effects"]["side_effects"]["scope"], "installation");
    let stored = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(
        stored.json()["installation"]["fire_settings"]["taxable_gain_ratio"],
        "1"
    );

    // Confirm: cambia SOLO g — los tax_brackets personalizados sobreviven (el bug del
    // #[serde(default)] que un PATCH parcial por HTTP sí dispararía).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_fire_settings",
            json!({"taxable_gain_ratio": "0.5", "confirm": true}),
        ),
    )
    .await;
    let applied = tool_json(&envelope);
    assert_eq!(applied["applied"], true);
    let stored = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    let fs = stored.json()["installation"]["fire_settings"].clone();
    assert_eq!(fs["taxable_gain_ratio"], "0.5");
    assert!(
        fs.get("swr_pct").is_none(),
        "5.0.0: el SWR ya no vive en fire_settings: {fs}"
    );
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
        tool_call(
            "update_fire_settings",
            json!({"taxable_gain_ratio": "2", "confirm": true}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");

    // Member (escritor) NO puede: el gate es Owner, no role_can_write.
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let member_token = create_token_for(&app, &member.cookie).await;
    let envelope = mcp_post(
        &app,
        &member_token,
        tool_call(
            "update_fire_settings",
            json!({"taxable_gain_ratio": "0.7", "confirm": true}),
        ),
    )
    .await;
    tool_error(&envelope, "forbidden");

    // Cambiar savings_source por MCP invalida la proyección (FULL).
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    app.warm_default_view(&owner.cookie, &key).await;
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
    assert_eq!(preview["effects"]["entity"]["concept"], "aporte");
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
    assert_eq!(
        preview["effects"]["side_effects"]["transactions_unlinked"], 1,
        "{preview}"
    );
    assert_eq!(app.count_rows("assets").await, 1);
    let envelope =
        preview_then_confirm(&app, &token, "delete_asset", json!({"id": asset_id})).await;
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
    assert_eq!(preview["effects"]["side_effects"]["items_deleted"], 1, "{preview}");
    let envelope =
        preview_then_confirm(&app, &token, "delete_snapshot", json!({"id": snap_id})).await;
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
    // 4.15.0: el confirm exige categoría en toda decisión income/expense.
    let cat = app.create_category(&owner, "expense", "Compras").await;
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({"source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                   "decisions": [{"kind": "expense", "category_id": cat},
                                 {"kind": "expense", "category_id": cat}],
                   "learn_rules": false}),
            &owner.cookie,
        )
        .await;
    assert!(c.status.is_success(), "{c:?}");
    assert_eq!(app.count_rows("transactions").await, 2);

    // Fase 5: la tool pagina y ecoa la vista → los lotes van bajo `imports`.
    let batches = tool_json(
        &mcp_post(&app, &token, tool_call("list_transaction_imports", json!({}))).await,
    );
    let import_id = batches["imports"][0]["id"].as_str().unwrap().to_string();

    let envelope = mcp_post(&app, &token, tool_call("delete_import", json!({"id": import_id}))).await;
    let preview = tool_json(&envelope);
    assert_eq!(
        preview["effects"]["side_effects"]["transactions_deleted"], 2,
        "{preview}"
    );
    assert_eq!(app.count_rows("transactions").await, 2, "el preview no borra");

    let envelope =
        preview_then_confirm(&app, &token, "delete_import", json!({"id": import_id})).await;
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

    // Sin confirm es preview y no ejecuta nada (y no emite token: es reversible).
    let preview =
        tool_json(&mcp_post(&app, &token, tool_call("reconcile_transfers", json!({}))).await);
    assert_eq!(preview["preview"], true, "{preview}");
    assert!(preview["confirm_token"].is_null(), "reversible: sin token: {preview}");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("reconcile_transfers", json!({"confirm": true})),
    )
    .await;
    let body = tool_json(&envelope);
    assert_eq!(body["pairs_created"].as_u64(), Some(0), "punto fijo vía MCP: {body}");

    // Desconciliar por MCP (modo B para verificar la invalidación COND).
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    app.warm_default_view(&owner.cookie, &key).await;
    let envelope = preview_then_confirm(
        &app,
        &token,
        "unreconcile_transfer",
        json!({"transaction_id": a["id"]}),
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
    // guard no alcanza a esta ruta (auditoría MCP §3).
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
    let key = app.default_view_key(iid, owner.user_id);

    // 1. PREVIEW: no escribe y no invalida.
    app.warm_default_view(&owner.cookie, &key).await;
    let preview = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "apply_categorization_rule",
                json!({"id": rule_id, "apply_to_existing": "all"}),
            ),
        )
        .await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["effects"]["side_effects"]["would_match"], 2, "{preview}");
    assert_eq!(
        preview["effects"]["side_effects"]["would_change_kind"], 2,
        "{preview}"
    );
    assert_eq!(
        preview["effects"]["side_effects"]["moves_projection_in_modes_b_and_c"], true,
        "el aviso de proyección debe salir ANTES de ejecutar: {preview}"
    );
    assert_eq!(
        preview["effects"]["side_effects"]["sample"].as_array().unwrap().len(),
        2
    );
    assert!(app.cache_contains(&key).await, "el preview no debe invalidar");
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["kind"], "income", "el preview no debe escribir: {t}");
    }

    // 2. CONFIRM: escribe, y el resultado es indistinguible vía HTTP. Desde la Fase 3 exige el
    //    confirm_token del preview — el paso 1 lo emitió.
    let ct = preview["confirm_token"].as_str().expect("token del preview");
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "apply_categorization_rule",
                json!({"id": rule_id, "apply_to_existing": "all", "confirm": true,
                       "confirm_token": ct}),
            ),
        )
        .await,
    );
    assert_eq!(out["updated"], 2, "{out}");
    assert_eq!(out["summary"].as_array().unwrap().len(), 2, "{out}");
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
            json!({"id": rule_id, "apply_to_existing": "all", "confirm": true}),
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
            json!({"id": rule_id, "apply_to_existing": "all", "confirm": true}),
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
    let key = app.default_view_key(iid, owner.user_id);
    app.warm_default_view(&owner.cookie, &key).await;

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
    assert_eq!(out["summary"].as_array().unwrap().len(), 3, "{out}");
    assert_eq!(out["summary_truncated"], false, "{out}");

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

// ---------------------------------------------------------------------------
// Fase 0 del plan de mejora del MCP (issue #81) — la red que faltaba alrededor
// del gate de escritura.
//
// Hasta aquí, la invariante «N tools de escritura == N llamadas a
// `require_mcp_write`» solo vivía en un `grep` dentro de un `.md`
// (`futurefin-mcp-parity` §5). Una tool nueva que se olvidara del gate pasaba
// TODA la CI en verde, y con ella un `viewer` podía escribir — o se podía
// escribir con el kill-switch `mcp_write_enabled` apagado. Los dos tests que
// siguen convierten ese grep en código ejecutable, por dos vías
// deliberadamente distintas:
//
//   1. `every_write_tool_rejects_a_viewer_and_the_disabled_toggle` — de
//      comportamiento, guiado por `tools/list`: prueba lo que un cliente MCP
//      vería de verdad.
//   2. `every_write_tool_in_the_source_calls_require_mcp_write` — estructural,
//      sobre el fuente, sin base de datos: es el único que detecta el olvido en
//      una tool futura ANTES de que alguien escriba su fixture.
// ---------------------------------------------------------------------------

/// Resultado de una llamada MCP, clasificado. Un envelope JSON-RPC puede volver de tres
/// maneras y las tres importan aquí: éxito (`result` sin `isError`), tool-error (`result`
/// con `isError: true` y el cuerpo de error de la API dentro del content de texto) y error
/// de protocolo (`error` en la raíz — es lo que devuelve rmcp cuando los argumentos ni
/// siquiera deserializan contra el `inputSchema`).
#[derive(Debug)]
enum Outcome {
    Success,
    ToolError { code: String, message: String },
    RpcError,
}

fn classify(envelope: &serde_json::Value) -> Outcome {
    if envelope.get("error").is_some() {
        return Outcome::RpcError;
    }
    let result = &envelope["result"];
    if result["isError"] != true {
        return Outcome::Success;
    }
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(body) => Outcome::ToolError {
            code: body["error"].as_str().unwrap_or("").to_string(),
            message: body["message"].as_str().unwrap_or("").to_string(),
        },
        // Un tool-error que no es JSON de la API (p.ej. un error de deserialización de rmcp
        // servido como content de texto) cuenta como rechazo de validación, no como éxito.
        Err(_) => Outcome::ToolError {
            code: String::new(),
            message: text.to_string(),
        },
    }
}

/// Argumentos **sintácticamente válidos** para cada tool de escritura: UUIDs con forma de
/// UUID (que no existen: da igual, el gate corre ANTES de que la core busque la fila),
/// fechas ISO, importes decimales. Su único trabajo es atravesar el parseo de parámetros
/// para que la llamada llegue de verdad a `require_mcp_write`.
///
/// **Por qué existe esta tabla** (la trampa que haría falso el test sin ella). Dos medidas
/// tomadas el 2026-08-28 sobre `src/mcp/server.rs`, y las dos apuntan al mismo sitio:
///
///   * **Estructural**: en **36 de las 41** tools de escritura el parseo de parámetros corre
///     ANTES del gate — el patrón `let run = || { … parse … }; match run() { Err(e) =>
///     return to_tool_outcome(e) }`. Sólo cinco (`capture_snapshot`, `materialize_recurring`,
///     `reconcile_transfers` y, desde la Fase 6, `confirm_transfer_match` —su `match_id` es una
///     cadena opaca que no se parsea— y `update_installation_settings` —sus tres ejes son
///     `Option<String>`—) llaman a `require_mcp_write` en la primera línea del bloque
///     asíncrono. (`unreconcile_transfer` pasó a parsear primero en la Fase 3: su preview
///     necesita el UUID para cargar las dos patas del par.)
///   * **Observable**: **35 de las 41** declaran algún parámetro `required`, así que con
///     `{}` mueren en la deserialización de rmcp y **nunca ejecutan el gate**. Con
///     argumentos vacíos sólo lo alcanzan `capture_snapshot`, `materialize_recurring`,
///     `reconcile_transfers`, `update_fire_settings`, `update_retirement_profile` y
///     `update_installation_settings` (las tres últimas: todos sus parámetros son opcionales).
///
/// Es decir: un test que barriera las 41 con `{}` y aceptara «cualquier error» daría verde
/// aunque el gate no existiera en 35 de ellas. De ahí la tabla.
///
/// Decisión (issue #81, punto 1): se implementan **las dos vías**, y la tabla es
/// **exhaustiva** — una tool de escritura nueva sin fila aquí hace fallar el test con
/// instrucciones. La vía laxa (aceptar validación) queda sólo como red de la fase 1, que
/// prueba una propiedad más débil pero de mantenimiento cero: *ninguna* escritura con
/// argumentos vacíos puede terminar en éxito para un viewer.
fn write_probe(name: &str) -> Option<serde_json::Value> {
    // UUID v4 sintácticamente válido que no existe en ninguna instalación.
    const ID: &str = "00000000-0000-4000-8000-000000000001";
    Some(match name {
        // Gate primero: estas tres ya llegan al gate con `{}`.
        "capture_snapshot" => json!({}),
        "materialize_recurring" => json!({}),
        "reconcile_transfers" => json!({}),
        // Parseo primero desde la Fase 3 (el preview carga las dos patas del par).
        "unreconcile_transfer" => json!({"transaction_id": ID}),
        // Parseo primero: necesitan argumentos con forma válida.
        "create_transaction" => {
            json!({"op_date": "2026-07-01", "concept": "probe", "amount": "-1.00", "kind": "expense"})
        }
        "update_transaction" => json!({"id": ID}),
        "update_transactions" => json!({"ids": [ID], "notes": "probe"}),
        "delete_transaction" => json!({"id": ID}),
        "create_planning_flow" => {
            json!({"title": "probe", "category_id": ID, "expected_amount": "1.00"})
        }
        "update_planning_flow" => json!({"id": ID}),
        "delete_planning_flow" => json!({"id": ID}),
        "create_category" => json!({"scope": "expense", "name": "probe"}),
        "create_categorization_rule" => json!({"pattern": "PROBE", "assign_kind": "expense"}),
        "update_categorization_rule" => json!({"id": ID}),
        "delete_categorization_rule" => json!({"id": ID}),
        "apply_categorization_rule" => json!({"id": ID}),
        "create_asset" => json!({"name": "probe", "category_id": ID, "current_value": "1.00"}),
        "update_asset" => json!({"asset_id": ID}),
        "update_asset_value" => json!({"asset_id": ID, "current_value": "1.00"}),
        "delete_asset" => json!({"id": ID}),
        "create_liability" => {
            json!({"label": "probe", "category_id": ID, "expense_category_id": ID, "principal": "1.00"})
        }
        "update_liability" => json!({"liability_id": ID}),
        "delete_liability" => json!({"id": ID}),
        "create_budget_entry" => json!({"category_id": ID, "amount": "1.00"}),
        "update_budget_entry" => json!({"id": ID}),
        "delete_budget_entry" => json!({"id": ID}),
        "update_allocation_rule" => json!({"id": ID, "enabled": true}),
        "delete_recurring_rule" => json!({"id": ID}),
        "delete_snapshot" => json!({"id": ID}),
        "delete_import" => json!({"id": ID}),
        "update_fire_settings" => json!({"taxes_enabled": true}),
        // 5.0.0: el plan de jubilación es del usuario del token, así que el gate es por ROL
        // (`require_mcp_write`), no owner-only.
        "update_retirement_profile" => json!({"swr_pct": "3.5"}),
        // Fase 6 (issue #87).
        "create_batch" => json!({
            "transactions": [
                {"op_date": "2026-07-01", "concept": "probe", "amount": "-1.00", "kind": "expense"}
            ]
        }),
        "create_snapshot" => json!({"kind": "asset", "snapshot_date": "2026-07-01"}),
        "update_snapshot" => json!({"id": ID}),
        "create_allocation_rule" => {
            json!({"target_asset_id": ID, "kind": "fixed", "amount": "10.00"})
        }
        "delete_allocation_rule" => json!({"id": ID}),
        "update_category" => json!({"id": ID, "name": "probe"}),
        "delete_category" => json!({"id": ID}),
        // Gate primero: no parsea nada antes (el `match_id` es una cadena opaca del servidor).
        "confirm_transfer_match" => json!({"match_id": "0123456789abcdef01234567"}),
        // Gate primero: los tres ejes son `Option<String>` y no se parsean.
        "update_installation_settings" => json!({"base_currency": "EUR"}),
        _ => return None,
    })
}

/// Nombres de las tools de escritura tal y como las publica el propio servidor
/// (`annotations.readOnlyHint == false`). Guiar el test por el catálogo y no por una lista
/// escrita a mano es lo que hace que una tool nueva entre sola en la batería.
fn write_tool_names(catalog: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = catalog["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array: {catalog}"))
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == false)
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

/// Las dos puertas de escritura, sobre **todas** las tools de escritura del catálogo:
/// rol (`viewer` → `forbidden`) y kill-switch (`mcp_write_enabled = false` →
/// `mcp_write_disabled: …`).
///
/// Fase 1 (laxa, mantenimiento cero): con argumentos vacíos, ninguna escritura puede
/// terminar en éxito para un viewer. Fase 2 (con dientes): con la tabla `write_probe` de
/// argumentos válidos, el error tiene que ser EXACTAMENTE el del gate — no vale un error de
/// validación disfrazado.
#[tokio::test]
async fn every_write_tool_rejects_a_viewer_and_the_disabled_toggle() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app
        .register_and_approve_member(&owner, "vera", "viewer")
        .await;
    let viewer_token = create_token_for(&app, &viewer.cookie).await;
    let owner_token = create_token(&app, &owner).await;

    let catalog = mcp_post(
        &app,
        &owner_token,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {"_meta": request_meta()}
        }),
    )
    .await;
    let writes = write_tool_names(&catalog);
    assert_eq!(
        writes.len(),
        41,
        "contador de futurefin-mcp-parity §5: 41 tools de escritura (31 hasta la Fase 5; la \
         Fase 6 añade create_batch, create_snapshot, update_snapshot, create_allocation_rule, \
         delete_allocation_rule, update_category, delete_category, confirm_transfer_match y \
         update_installation_settings; 5.0.0 añade update_retirement_profile). Si has añadido o \
         retirado una, actualiza el contador \
         AQUÍ, en la skill mcp-parity, en .claude/mcp-catalog.md y en .claude/backend-structure.md a la vez: {writes:?}"
    );

    // --- Fase 1: barrido laxo con argumentos vacíos -------------------------
    // Prueba débil pero universal: da igual lo que devuelva (forbidden, validación, error de
    // protocolo), lo que NO puede hacer es funcionar.
    for name in &writes {
        let envelope = mcp_post(&app, &viewer_token, tool_call(name, json!({}))).await;
        match classify(&envelope) {
            Outcome::Success => panic!(
                "la tool de escritura {name} ha tenido ÉXITO con un token de rol `viewer` y \
                 argumentos vacíos: {envelope}"
            ),
            Outcome::ToolError { .. } | Outcome::RpcError => {}
        }
    }

    // --- Fase 2: barrido con dientes ----------------------------------------
    let missing: Vec<&String> = writes.iter().filter(|n| write_probe(n).is_none()).collect();
    assert!(
        missing.is_empty(),
        "tools de escritura sin fila en `write_probe`: {missing:?}. Añade unos argumentos \
         sintácticamente válidos (UUIDs inexistentes valen) para que el test pueda comprobar \
         que la llamada llega al gate en vez de morir en el parseo."
    );

    for name in &writes {
        let args = write_probe(name).unwrap();
        let envelope = mcp_post(&app, &viewer_token, tool_call(name, args.clone())).await;
        match classify(&envelope) {
            Outcome::ToolError { code, .. } if code == "forbidden" => {}
            other => panic!(
                "la tool {name} debe responder `forbidden` a un viewer y respondió {other:?}. \
                 Si es un error de validación, el problema es la fila de `write_probe`; si es \
                 un éxito, es que a la tool le falta `require_mcp_write`. Envelope: {envelope}"
            ),
        }
    }

    // Toggle apagado, con un token que SÍ podría escribir (owner).
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"mcp_write_enabled": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    for name in &writes {
        let args = write_probe(name).unwrap();
        let envelope = mcp_post(&app, &owner_token, tool_call(name, args)).await;
        match classify(&envelope) {
            Outcome::ToolError { code, message }
                if code == "bad_request" && message.starts_with("mcp_write_disabled") => {}
            other => panic!(
                "la tool {name} debe responder `mcp_write_disabled` con el kill-switch apagado \
                 y respondió {other:?}. Envelope: {envelope}"
            ),
        }
    }

    // Y con el toggle apagado nada se ha escrito: el hogar sigue vacío de datos de prueba.
    assert_eq!(app.count_rows("transactions").await, 0);
    assert_eq!(app.count_rows("assets").await, 0);
    assert_eq!(app.count_rows("liabilities").await, 0);
    assert_eq!(app.count_rows("history_snapshots").await, 0);
}

// ---------------------------------------------------------------------------
// Aserción ESTRUCTURAL — sin base de datos.
// ---------------------------------------------------------------------------

/// Quita las líneas de comentario del fuente. **No es cosmética**: el comentario de sección
/// que separa las lecturas de las escrituras cita literalmente `require_mcp_write`, y vive en
/// el mismo trozo que la ÚLTIMA tool de lectura — sin esto, `list_snapshots` parecería llamar
/// al gate. La prosa de este repo habla de su propio código; el parser tiene que ignorarla.
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Un bloque `#[tool(…)] async fn …` del fuente del servidor MCP.
struct ToolBlock {
    name: String,
    read_only: bool,
    body: String,
}

/// Trocea `src/mcp/server.rs` por `#[tool(`. Es un parser deliberadamente crudo: no
/// entiende Rust, sólo la forma que el fichero tiene hoy. A cambio es exacto para lo único
/// que le pedimos —¿aparece `require_mcp_write` dentro del cuerpo de esta tool?— y no
/// necesita ni base de datos ni arrancar el router.
fn tool_blocks() -> Vec<ToolBlock> {
    const SRC: &str = include_str!("../src/mcp/server.rs");
    let mut out = Vec::new();
    for chunk in SRC.split("#[tool(").skip(1) {
        let split_at = chunk
            .find("async fn ")
            .unwrap_or_else(|| panic!("bloque #[tool( sin `async fn`:\n{}", &chunk[..200.min(chunk.len())]));
        let (attr, body) = chunk.split_at(split_at);
        let name = attr
            .split_once("name = \"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| panic!("bloque #[tool( sin `name = \"…\"`:\n{attr}"));
        let read_only = match (
            attr.contains("read_only_hint = true"),
            attr.contains("read_only_hint = false"),
        ) {
            (true, false) => true,
            (false, true) => false,
            _ => panic!("la tool {name} debe declarar `read_only_hint` exactamente una vez"),
        };
        out.push(ToolBlock {
            name,
            read_only,
            body: strip_line_comments(body),
        });
    }
    out
}

/// **La aserción que no depende de la base de datos ni de que alguien escriba un fixture**:
/// toda tool declarada de escritura (`read_only_hint = false`) contiene una llamada a
/// `require_mcp_write` en su cuerpo, y esa llamada se identifica con SU PROPIO nombre.
///
/// Es cruda —lee el fuente como texto— pero es exacta, cuesta milisegundos y es lo único
/// que detecta el olvido en una tool futura: el test de comportamiento sólo recorre lo que
/// el catálogo publica y sólo tiene dientes donde alguien haya escrito su fila de
/// `write_probe`; éste no necesita ninguna de las dos cosas.
///
/// Fija además los contadores de `futurefin-mcp-parity` §5, para que cualquier cambio del
/// catálogo tenga que pasar por aquí y por la skill a la vez.
#[test]
fn every_write_tool_in_the_source_calls_require_mcp_write() {
    const SRC: &str = include_str!("../src/mcp/server.rs");
    let blocks = tool_blocks();

    // Contadores de futurefin-mcp-parity §5 (71 / 30 / 41 / 41, a 2026-09-03, WP6b de 5.0.0:
    // `get_projection_bands` — la primera tool nueva que NO es de escritura desde WP4).
    let read_only = blocks.iter().filter(|b| b.read_only).count();
    let writes = blocks.iter().filter(|b| !b.read_only).count();
    assert_eq!(blocks.len(), 71, "total de tools (§5 de futurefin-mcp-parity)");
    assert_eq!(read_only, 30, "tools de lectura + simulate (§5)");
    assert_eq!(writes, 41, "tools de escritura (§5)");
    assert_eq!(
        read_only + writes,
        blocks.len(),
        "toda tool es de lectura o de escritura"
    );
    assert_eq!(
        SRC.matches("require_mcp_write(&self.state.pool").count(),
        41,
        "el nº de llamadas al gate debe ser EXACTAMENTE el nº de escrituras: una escritura \
         sin gate es un fallo de seguridad (viewer escribiendo, o escritura con el \
         kill-switch apagado); una llamada de más es una lectura que ya no lo es"
    );
    assert_eq!(
        SRC.matches("p.confirm.unwrap_or(false)").count(),
        18,
        "tools con preview/confirm (§5). Toda destructiva nueva debería sumar aquí. Subió de 11 \
         a 14 en la Fase 3 (issue #84): `materialize_recurring`, `reconcile_transfers` y \
         `unreconcile_transfer` eran destructivas SIN preview — dos de ellas irreversibles —, así \
         que la regla «sin confirm en el esquema ⇒ no destructiva» era falsa en tres sitios. Y a \
         17 en la Fase 6: `delete_allocation_rule`, `delete_category` y \
         `update_installation_settings`. Las otras seis escrituras de la Fase 6 NO llevan preview \
         a propósito: cuatro son altas (`create_*`), `update_snapshot` edita en sitio, y \
         `confirm_transfer_match` ya tiene su preview en OTRA tool — `suggest_transfer_matches` \
         es literalmente la fase 1, y el `match_id` que emite es lo que acota el espacio de \
         acciones alcanzables"
    );
    // Las 8 que además exigen el token de un solo uso del preview (Fase 3). El `confirm`
    // booleano lo escribe el propio modelo, así que por sí solo nunca fue un control: sólo el
    // token demuestra que hubo un preview, y va ligado a la huella de los efectos.
    assert_eq!(
        SRC.matches("p.confirm_token.as_deref()").count(),
        8,
        "tools con confirmación en dos fases: las de cascada de tamaño no acotado \
         (delete_import, delete_asset, delete_liability, apply_categorization_rule, \
         materialize_recurring) y las puertas de un solo sentido (unreconcile_transfer, \
         delete_snapshot, y desde la Fase 6 delete_allocation_rule: borrar la regla redirige el \
         sobrante mensual y recrearla no restaura su prioridad). Los borrados de UNA fila cuyo \
         contenido entero viaja en el preview NO llevan token a propósito: encarecerlos a dos \
         viajes convierte la ceremonia en ruido"
    );
    // Y la auditoría: cada gate abre una fila, y `settled` es el ÚNICO sitio donde se cierra.
    assert_eq!(
        SRC.matches("settled(&self.state.pool, audit").count(),
        41,
        "toda escritura cierra su fila de auditoría con `settled`; sin él la fila se queda en \
         `attempted` y el log calla el desenlace de justo las llamadas que fallaron"
    );

    for block in &blocks {
        if block.read_only {
            assert!(
                !block.body.contains("require_mcp_write"),
                "la tool {} se declara `read_only_hint = true` pero llama al gate de \
                 escritura: una de las dos cosas es mentira",
                block.name
            );
            continue;
        }
        let at = block.body.find("require_mcp_write").unwrap_or_else(|| {
            panic!(
                "la tool {} declara `read_only_hint = false` y NO llama a `require_mcp_write`: \
                 un `viewer` puede ejecutarla, y el kill-switch `mcp_write_enabled` no la \
                 corta. Añade el gate como primera línea del bloque asíncrono (patrón de \
                 `capture_snapshot`).",
                block.name
            )
        });
        // El tercer argumento del gate es el nombre de la tool y alimenta la traza de
        // auditoría: copiar-pegar el bloque de otra tool y olvidarse de cambiarlo dejaría el
        // log señalando a la tool equivocada, que es peor que no tener log.
        let call = &block.body[at..];
        // Hasta el `;` de la sentencia: acota la búsqueda a ESTA llamada aunque venga
        // envuelta en varias líneas, y evita que el nombre casara por accidente con una
        // aparición posterior en el mismo cuerpo.
        let end = call.find(';').unwrap_or(call.len());
        assert!(
            call[..end].contains(&format!("\"{}\"", block.name)),
            "la llamada a require_mcp_write de la tool {} no se identifica con su propio \
             nombre (la traza de auditoría apuntaría a otra tool): {}",
            block.name,
            &call[..end.min(200)]
        );
    }
}

/// `delete_liability` (Fase 0, issue #81): hasta ahora la tool sólo existía como cadena
/// dentro del vector del catálogo congelado — **ningún test la invocaba**. Es destructiva,
/// tiene preview/confirm. Hasta la Fase 2 su `effects` **no tenía la misma forma** que el de
/// `delete_asset`: aquí `transactions_unlinked` colgaba directamente de `effects` (es un
/// escalar), mientras que en `delete_asset` vivía dentro de `effects.unlinked` junto a las
/// reglas de reparto borradas. Un cliente que asumiera la forma del vecino leería `null` y
/// le diría al usuario que no se desvincula nada.
///
/// Incluye la invariante que de verdad importa al confirmar: el movimiento vinculado
/// **sobrevive** desvinculado (SET NULL), no se borra con el pasivo.
#[tokio::test]
async fn delete_liability_previews_effects_then_deletes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_liab = app.create_category(&owner, "liability", "Préstamos").await;
    let cat_exp = app.create_category(&owner, "expense", "Cuotas").await;

    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({
                "category_id": cat_liab,
                "expense_category_id": cat_exp,
                "label": "Coche",
                "principal": "9000",
                "payment_amount": "300",
                "payment_frequency": "monthly",
                "payment_end_date": "2090-01-01",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");
    let liab_id = liab.json()["id"].as_str().unwrap().to_string();

    // Dos cuotas pagadas y vinculadas al pasivo: el preview debe contarlas.
    for day in ["2026-06-05", "2026-07-05"] {
        let t = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({"op_date": day, "concept": "cuota coche", "amount": "-300.00",
                       "kind": "expense", "category_id": cat_exp, "linked_liability_id": liab_id}),
                &owner.cookie,
            )
            .await;
        assert_eq!(t.status, http::StatusCode::CREATED, "{t:?}");
    }

    // Preview: NO borra y describe los efectos con su forma propia.
    let envelope = mcp_post(&app, &token, tool_call("delete_liability", json!({"id": liab_id}))).await;
    let preview = tool_json(&envelope);
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["confirm_required"], true, "{preview}");
    assert_eq!(preview["action"], "delete_liability", "{preview}");
    assert_eq!(preview["effects"]["entity"]["label"], "Coche", "{preview}");
    assert_eq!(preview["effects"]["entity"]["id"], liab_id, "{preview}");
    // Fase 2: misma forma que delete_asset — `{entity, side_effects}`, sin `unlinked`.
    assert_eq!(
        preview["effects"]["side_effects"]["transactions_unlinked"], 2,
        "{preview}"
    );
    assert!(
        preview["effects"]["unlinked"].is_null(),
        "la clave `unlinked` desapareció en la Fase 2: {preview}"
    );
    assert_eq!(app.count_rows("liabilities").await, 1, "el preview no borra");
    assert_eq!(app.count_rows("transactions").await, 2);

    // Confirm: borra el pasivo; los movimientos sobreviven desvinculados.
    let envelope =
        preview_then_confirm(&app, &token, "delete_liability", json!({"id": liab_id})).await;
    let done = tool_json(&envelope);
    assert_eq!(done["deleted"], true, "{done}");
    assert_eq!(done["id"], liab_id, "{done}");
    assert_eq!(app.count_rows("liabilities").await, 0);
    assert_eq!(app.count_rows("transactions").await, 2, "los movimientos sobreviven");
    let unlinked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM transactions WHERE linked_liability_id IS NULL",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(unlinked, 2, "desvinculadas, no borradas");

    // Y el pasivo ya no está: repetir el borrado es not_found, no un segundo éxito.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_liability", json!({"id": liab_id, "confirm": true})),
    )
    .await;
    tool_error(&envelope, "not_found");
}

/// `delete_planning_flow` **sin `confirm`** (Fase 0, issue #81): la única llamada que existía
/// a esta tool iba directa con `confirm: true`, así que el camino del preview no se ejecutaba
/// nunca — podía devolver cualquier cosa, o borrar, sin que ningún test se enterara.
#[tokio::test]
async fn delete_planning_flow_preview_does_not_delete() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_exp = app.create_category(&owner, "expense", "Viajes").await;

    let flow = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_planning_flow",
                json!({"title": "Viaje a Oslo", "category_id": cat_exp,
                       "expected_amount": "600", "due_date": "2090-06-01"}),
            ),
        )
        .await,
    );
    let flow_id = flow["id"].as_str().unwrap().to_string();

    let preview = tool_json(
        &mcp_post(&app, &token, tool_call("delete_planning_flow", json!({"id": flow_id}))).await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["confirm_required"], true, "{preview}");
    assert_eq!(preview["action"], "delete_planning_flow", "{preview}");
    // El preview devuelve el flujo ENTERO (la respuesta del listado), no un resumen: es lo que
    // permite a un cliente enseñar título e importe antes de pedir confirmación.
    assert_eq!(preview["effects"]["entity"]["title"], "Viaje a Oslo", "{preview}");
    assert_eq!(preview["effects"]["entity"]["id"], flow_id, "{preview}");
    assert_eq!(app.count_rows("planning_flows").await, 1, "el preview no borra");

    // Repetir el preview es inocuo (no es un borrado a medias).
    let again = tool_json(
        &mcp_post(&app, &token, tool_call("delete_planning_flow", json!({"id": flow_id}))).await,
    );
    assert_eq!(again, preview, "el preview es estable entre llamadas");
    assert_eq!(app.count_rows("planning_flows").await, 1);
}

/// Fase 2 (issue #83) — **los 11 previews tienen UNA forma, no seis**.
///
/// Antes de esta fase cada preview inventaba su `effects`: `{transaction}`, `{flow}`,
/// `{entry}`, `{rule, nota}`, `{asset, unlinked}`, `{liability, transactions_unlinked}`,
/// `{snapshot{…, items_deleted}}`, `{transactions_deleted, import}`, contadores planos, el
/// `before/after` pelado de `update_fire_settings`… y `delete_categorization_rule` los ponía en
/// **español** (`regla`, `huella.cambiarian`, `nota`), único caso del catálogo.
///
/// El peor no era la variedad, era una clave concreta: `delete_asset` escondía
/// `allocation_remainder_rules_deleted` —la única cifra IRREVERSIBLE del borrado, la que su
/// propia descripción destaca en mayúsculas— dentro de una clave llamada `unlinked`, que es la
/// palabra que describe justo lo contrario (los movimientos, que solo se desvinculan).
///
/// La forma es `{"entity": …, "side_effects": …}`: qué se toca, y qué cambia además. Este test
/// recorre los ONCE y no deja pasar ni una clave suelta ni una clave en español.
#[tokio::test]
async fn every_preview_shares_the_entity_side_effects_shape() {
    /// Claves en español que llegó a publicar algún preview. No pueden volver: la norma del
    /// repo es «UI en español, identificadores en inglés», y los VALORES de prosa (`note`)
    /// siguen en español.
    const SPANISH_KEYS: &[&str] = &[
        "regla",
        "huella",
        "nota",
        "cambiarian",
        "ya_conformes",
        "no_asigna_nada",
        "tapa_a_otra_regla",
        "pierde_frente_a_otra_regla",
        "descartados_por_source",
        // Publicada hasta la Fase 2 por los ONCE payloads de confirmación (`{id, resumen}`),
        // no por los previews. Renombrada a `summary` en el mismo cambio: era la última clave
        // en español del wire MCP.
        "resumen",
    ];

    fn assert_shape(action: &str, preview: &serde_json::Value) {
        assert_eq!(preview["preview"], true, "{action}: {preview}");
        assert_eq!(preview["confirm_required"], true, "{action}: {preview}");
        assert_eq!(preview["action"], action, "{action}: {preview}");
        let effects = preview["effects"]
            .as_object()
            .unwrap_or_else(|| panic!("{action}: effects debe ser un objeto: {preview}"));
        let mut keys: Vec<&str> = effects.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["entity", "side_effects"],
            "{action}: `effects` solo tiene `entity` y `side_effects`: {preview}"
        );
        assert!(
            effects["entity"].is_object(),
            "{action}: `entity` describe la fila tocada: {preview}"
        );
        assert!(
            effects["side_effects"].is_object(),
            "{action}: `side_effects` es un objeto (vacío = «no arrastra nada»): {preview}"
        );

        // Ninguna clave en español, a ninguna profundidad.
        fn keys_of(v: &serde_json::Value, out: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(map) => {
                    for (k, sub) in map {
                        out.push(k.clone());
                        keys_of(sub, out);
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(|i| keys_of(i, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        keys_of(&preview["effects"], &mut all);
        for k in &all {
            assert!(
                !SPANISH_KEYS.contains(&k.as_str()),
                "{action}: la clave `{k}` está en español: {preview}"
            );
            assert!(
                k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{action}: la clave `{k}` no es snake_case ASCII: {preview}"
            );
        }
    }

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let cat_lia = app.create_category(&owner, "liability", "Préstamos").await;

    let preview_of = |args: serde_json::Value, tool: &'static str| {
        let app = &app;
        let token = &token;
        async move { tool_json(&mcp_post(app, token, tool_call(tool, args)).await) }
    };

    // --- Seed: un activo con movimiento vinculado, un pasivo, un presupuesto, un próximo,
    //     un snapshot, una plantilla recurrente, una regla y un lote de import.
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({"category_id": cat_lia, "expense_category_id": cat_exp,
                   "label": "Coche", "principal": "5000"}),
            &owner.cookie,
        )
        .await;
    let liab_id = liab.json()["id"].as_str().unwrap().to_string();
    let txn = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_transaction",
                json!({"op_date": "2026-07-01", "concept": "aporte", "amount": "-200.00",
                       "kind": "savings", "linked_asset_id": asset_id, "recurring": true}),
            ),
        )
        .await,
    );
    let txn_id = txn["id"].as_str().unwrap().to_string();
    let entry = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("create_budget_entry", json!({"category_id": cat_exp, "amount": "100"})),
        )
        .await,
    );
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
    let snaps = tool_json(
        &mcp_post(&app, &token, tool_call("capture_snapshot", json!({"kinds": ["asset"]}))).await,
    );
    let snap_id = snaps["snapshots"][0]["id"].as_str().unwrap().to_string();
    let rule = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_categorization_rule",
                json!({"pattern": "SUPER", "assign_kind": "expense",
                       "assign_category_id": cat_exp}),
            ),
        )
        .await,
    );
    let rule_id = rule["id"].as_str().unwrap().to_string();
    let recurring = tool_json(
        &mcp_post(&app, &token, tool_call("list_recurring_rules", json!({}))).await,
    );
    let recurring_id = recurring[0]["id"].as_str().unwrap().to_string();

    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               01/06/2026;01/06/2026;SUPER;-10,00;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({"source": "myinvestor", "file_b64": b64}),
            &owner.cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    // 4.15.0: el confirm exige categoría en toda decisión income/expense.
    let import_cat = app.create_category(&owner, "expense", "Compras").await;
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({"source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                   "decisions": [{"kind": "expense", "category_id": import_cat}],
                   "learn_rules": false}),
            &owner.cookie,
        )
        .await;
    assert!(c.status.is_success(), "{c:?}");
    // Fase 5: la tool pagina y ecoa la vista → los lotes van bajo `imports`.
    let batches = tool_json(
        &mcp_post(&app, &token, tool_call("list_transaction_imports", json!({}))).await,
    );
    let import_id = batches["imports"][0]["id"].as_str().unwrap().to_string();

    // Par auto-conciliado (importes opuestos a un día): el preview de `unreconcile_transfer`
    // necesita una pata con contrapartida para poder enseñar las dos.
    let leg = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-07-20", "concept": "Traspaso salida", "amount": "-75",
                   "kind": "expense"}),
            &owner.cookie,
        )
        .await;
    let leg_id = leg.json()["id"].as_str().unwrap().to_string();
    let back = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-07-21", "concept": "Traspaso entrada", "amount": "75",
                   "kind": "income"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        back.json()["transfer_counterpart_id"], json!(leg_id),
        "precondición: el par queda conciliado por el pase automático"
    );

    // Una regla de cascada (el sumidero) y una categoría suelta, para los previews de la Fase 6.
    // La regla apunta a un activo APARTE: colgarla de `asset_id` cambiaría los efectos
    // colaterales que el preview de `delete_asset` comprueba al final de este test.
    let asset2 = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Colchón", "current_value": "500"}),
            &owner.cookie,
        )
        .await;
    let asset2_id = asset2.json()["id"].as_str().unwrap().to_string();
    // #150: "Fondo" (asset_id) fue el primer activo del owner → ya sembró el sumidero
    // apuntándole. Lo retargeteamos a "Colchón" (asset2) en vez de crear uno segundo: el test
    // necesita el sumidero en un activo APARTE para que el preview de `delete_asset` sobre
    // "Fondo" no lo arrastre (el comentario original de más arriba, ahora cumplido vía PATCH).
    let alloc_id = app.sink_rule_id(&owner.cookie).await;
    let retarget = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{alloc_id}"),
            json!({"target_asset_id": asset2_id}),
            &owner.cookie,
        )
        .await;
    assert_eq!(retarget.status, http::StatusCode::OK, "{retarget:?}");
    let spare_cat = app.create_category(&owner, "expense", "Ocio").await;

    // --- Los dieciocho previews. NINGUNO lleva `confirm`, así que nada se escribe.
    let cases: Vec<(&'static str, serde_json::Value)> = vec![
        ("delete_asset", json!({"id": asset_id})),
        ("delete_liability", json!({"id": liab_id})),
        ("delete_transaction", json!({"id": txn_id})),
        ("delete_budget_entry", json!({"id": entry["id"]})),
        ("delete_planning_flow", json!({"id": flow["id"]})),
        ("delete_snapshot", json!({"id": snap_id})),
        ("delete_import", json!({"id": import_id})),
        ("delete_categorization_rule", json!({"id": rule_id})),
        ("delete_recurring_rule", json!({"id": recurring_id})),
        ("apply_categorization_rule", json!({"id": rule_id})),
        ("update_fire_settings", json!({"taxes_enabled": true})),
        // Fase 3 (issue #84): las tres destructivas que NO tenían preview y ahora lo tienen.
        ("materialize_recurring", json!({})),
        ("reconcile_transfers", json!({})),
        ("unreconcile_transfer", json!({"transaction_id": leg_id})),
        // Fase 6 (issue #87).
        ("delete_allocation_rule", json!({"id": alloc_id})),
        ("delete_category", json!({"id": spare_cat})),
        ("update_installation_settings", json!({"base_currency": "EUR"})),
        // 5.0.0 (issue #207): mismo criterio que `update_fire_settings`, pero por perfil.
        ("update_retirement_profile", json!({"swr_pct": "3.5"})),
    ];
    assert_eq!(cases.len(), 18, "los previews del catálogo (§5 de futurefin-mcp-parity)");
    for (tool, args) in cases {
        let preview = preview_of(args, tool).await;
        assert_shape(tool, &preview);
    }

    // Y la cifra que motivó la unificación está donde debe: fuera de una clave llamada
    // `unlinked`, al mismo nivel que el resto de efectos colaterales del borrado.
    let asset_preview = preview_of(json!({"id": asset_id}), "delete_asset").await;
    let side = &asset_preview["effects"]["side_effects"];
    assert_eq!(side["transactions_unlinked"], 1, "{asset_preview}");
    assert_eq!(side["allocation_rules_deleted"], 0, "{asset_preview}");
    assert_eq!(side["allocation_remainder_rules_deleted"], 0, "{asset_preview}");
}

/// Fase 2 (issue #83) — **un parámetro mal escrito falla; ya no se descarta en silencio**.
///
/// Hermano de comportamiento de `every_input_schema_forbids_unknown_properties`
/// (`mcp_http.rs`), que solo mira el esquema publicado. Aquí se comprueba lo que de verdad
/// pasa al llamar, porque rmcp **no valida contra el `inputSchema`**: quien rechaza es
/// `#[serde(deny_unknown_fields)]` al deserializar, y sin él el esquema podría decir
/// `additionalProperties: false` mientras el servidor sigue tragando.
///
/// Los tres casos son los medidos en la auditoría, y ninguno fallaba antes:
///   * `delete_asset {id, confirmed: true}` — typo por `confirm`. Devolvía un **preview**, que
///     un modelo lee como «hecho»: cree haber borrado y el activo sigue ahí.
///   * `update_budget_entry {id, ammount: "250"}` — devolvía 200 sin cambiar el importe.
///   * `list_transactions {search: "…"}` — devolvía la primera página **sin filtrar**, que es
///     la peor de las tres: los datos parecen la respuesta a la pregunta.
#[tokio::test]
async fn a_misspelled_parameter_is_rejected_instead_of_silently_dropped() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let entry = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("create_budget_entry", json!({"category_id": cat_exp, "amount": "100"})),
        )
        .await,
    );

    for (tool, args, field, why) in [
        (
            "delete_asset",
            json!({"id": asset_id, "confirmed": true}),
            "confirmed",
            "`confirmed` no existe (es `confirm`): antes devolvía un preview, que un modelo lee \
             como «borrado hecho»",
        ),
        (
            "update_budget_entry",
            json!({"id": entry["id"], "ammount": "250"}),
            "ammount",
            "`ammount` no existe: antes devolvía 200 sin cambiar el importe",
        ),
        (
            "list_transactions",
            json!({"search": "mercadona"}),
            "search",
            "`search` no existe (es `concept_contains`): antes devolvía la página SIN filtrar",
        ),
    ] {
        let envelope = mcp_post(&app, &token, tool_call(tool, args)).await;
        // rmcp sirve el fallo de deserialización como tool-error de texto (no como error de
        // protocolo), y el mensaje NOMBRA el campo desconocido y enumera los válidos: es
        // exactamente lo que un modelo necesita para corregirse solo.
        match classify(&envelope) {
            Outcome::ToolError { message, .. } => {
                assert!(
                    message.contains("unknown field") && message.contains(field),
                    "{tool}: el error debe nombrar `{field}`, y dice {message:?}"
                );
            }
            other => panic!("{tool}: {why} — y ha devuelto {other:?}: {envelope}"),
        }
    }

    // Nada se ha escrito ni borrado por el camino.
    assert_eq!(app.count_rows("assets").await, 1);
    let budget = tool_json(&mcp_post(&app, &token, tool_call("get_budget", json!({}))).await);
    assert_eq!(budget["entries"][0]["amount"], "100.0000", "{budget}");
}

/// Fase 2 (issue #83) — **los errores propios de MCP llevan código estable y dicen el formato**.
///
/// Los tres helpers de parseo de `mcp/server.rs` (`parse_decimal_param`, `parse_uuid_param`,
/// `parse_date_param`) y cuatro guardias sueltas construían su mensaje SIN el prefijo
/// `snake_code: `, así que `derive_error_code` caía a la clase HTTP y el cliente recibía
/// `bad_request` — el código genérico que la SPA traduce por «Los datos enviados no son
/// válidos». Un `code` genérico es exactamente lo que `error_codes_parity` existe para impedir,
/// pero ese test extrae los códigos del FUENTE: si nadie escribe el literal, no hay nada que
/// extraer y el fixture no protesta.
///
/// El de decimal comprueba además el CONTENIDO: la UI es española, el usuario dicta «once con
/// ochenta y tres» y el modelo escribe `"11,83"`. Un mensaje que solo dijera «must be a decimal
/// string» deja al modelo eligiendo a ciegas entre cambiar la coma por un punto y quitar el
/// separador — y `"1183"` se acepta sin ruido, con dos órdenes de magnitud de más.
#[tokio::test]
async fn mcp_only_errors_carry_a_stable_code() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_exp = app.create_category(&owner, "expense", "Comida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let entry = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("create_budget_entry", json!({"category_id": cat_exp, "amount": "100"})),
        )
        .await,
    );
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

    // Importe con coma decimal: el caso que motiva el mensaje.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_asset_value", json!({"asset_id": asset_id, "current_value": "11,83"})),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert_eq!(body["code"], "decimal_invalid", "{body}");
    let msg = body["message"].as_str().unwrap();
    assert!(
        msg.contains("current_value") && msg.contains("1234.56") && msg.contains("1.234,56"),
        "el mensaje debe enseñar el formato bueno Y el malo: {msg}"
    );

    for (tool, args, code, needle) in [
        (
            "update_asset_value",
            json!({"asset_id": "el fondo", "current_value": "10"}),
            "uuid_invalid",
            "8-4-4-4-12",
        ),
        (
            "create_transaction",
            json!({"op_date": "01/03/2026", "concept": "x", "amount": "-10", "kind": "expense"}),
            "date_invalid",
            "YYYY-MM-DD",
        ),
        (
            "update_asset",
            json!({"asset_id": asset_id, "purchase_price": "10",
                   "clear_purchase_price": true}),
            "purchase_price_set_and_clear",
            "mutually exclusive",
        ),
        (
            "update_asset_value",
            json!({"asset_id": asset_id}),
            "patch_empty",
            "expected_annual_return_percent",
        ),
        (
            "update_budget_entry",
            json!({"id": entry["id"], "expense_end_date": "2027-01-01",
                   "clear_expense_end_date": true}),
            "expense_end_set_and_clear",
            "mutually exclusive",
        ),
        (
            "update_planning_flow",
            json!({"id": flow["id"], "due_date": "2027-01-01", "clear_due_date": true}),
            "due_date_set_and_clear",
            "mutually exclusive",
        ),
        (
            "list_transactions",
            json!({"limit": 501}),
            "limit_out_of_range",
            "between 1 and 500",
        ),
    ] {
        let envelope = mcp_post(&app, &token, tool_call(tool, args)).await;
        let body = tool_error(&envelope, "bad_request");
        assert_eq!(body["code"], code, "{tool}: {body}");
        assert!(
            body["message"].as_str().unwrap().contains(needle),
            "{tool}: el mensaje debe contener {needle:?}: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// Fase 6 (issue #87) — las decisiones que hacen seguras las nueve escrituras nuevas.
// ---------------------------------------------------------------------------

/// `create_allocation_rule` **no puede crear el sumidero**, y sí una regla capada.
///
/// La asimetría que justifica el `SinkPolicy::Forbidden`: crear el sumidero donde no había
/// redirige TODO el sobrante de golpe y **no se deshace por el mismo canal** — desde 4.12.1
/// (#176) el sumidero es INDESTRUCTIBLE con activos vivos: ni borrarlo, ni deshabilitarlo, ni
/// degradarlo (la salida es moverlo de activo). Un formulario que enseña la cascada entera hace
/// evidente ese estado; una conversación, no.
#[tokio::test]
async fn create_allocation_rule_refuses_the_sink_and_shares_the_core() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Indexado", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();

    // El sumidero: `remainder` SIN tope. Rechazado con su código propio, no con un 500 ni con
    // un éxito silencioso.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_allocation_rule",
            json!({"target_asset_id": asset_id, "kind": "remainder"}),
        ),
    )
    .await;
    match classify(&envelope) {
        Outcome::ToolError { message, .. } => assert!(
            message.starts_with("sink_creation_not_allowed"),
            "el rechazo del sumidero debe llevar su código: {message}"
        ),
        other => panic!("crear el sumidero desde MCP debe fallar, y devolvió {other:?}"),
    }

    // Un `remainder` CON tope sí: deja de ser el sumidero.
    app.warm_default_view(&owner.cookie, &key).await;
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_allocation_rule",
                json!({"target_asset_id": asset_id, "kind": "remainder",
                       "cap_kind": "amount", "cap_value": "5000"}),
            ),
        )
        .await,
    );
    let rule_id = out["id"].as_str().expect("id de la regla creada").to_string();
    assert!(out["impact"].is_object(), "la cascada mueve la proyección: {out}");
    app.assert_invalidated(&key, "create_allocation_rule").await;

    // Core compartida: la fila es indistinguible por HTTP.
    let via_http = app.get_with_cookie("/v1/allocation-rules", &owner.cookie).await;
    let rules = via_http.json();
    let row = rules
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(rule_id))
        .unwrap_or_else(|| panic!("la regla creada por MCP no aparece por HTTP: {rules}"));
    assert_eq!(row["cap_kind"], "amount", "{row}");
    assert_eq!(row["cap_value"], "5000.0000", "{row}");

    // Y el preview del borrado dice A DÓNDE deja de ir el dinero, no solo que hay una fila menos.
    let preview = tool_json(
        &mcp_post(&app, &token, tool_call("delete_allocation_rule", json!({"id": rule_id}))).await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert!(
        preview["effects"]["entity"]
            .as_object()
            .unwrap()
            .contains_key("amount_resolved_this_month"),
        "el preview debe traer lo que la regla encamina este mes: {preview}"
    );
    assert!(
        preview["confirm_token"].is_string(),
        "delete_allocation_rule exige el token de dos fases: {preview}"
    );
}

/// `delete_category`: el preview **obliga a nombrar el destino** del remap.
#[tokio::test]
async fn delete_category_preview_demands_a_remap_target() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Comida").await;
    let other = app.create_category(&owner, "expense", "Ocio").await;
    let txn = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_transaction",
                json!({"op_date": "2026-07-01", "concept": "cena", "amount": "-20.00",
                       "kind": "expense", "category_id": cat}),
            ),
        )
        .await,
    );
    let txn_id = txn["id"].as_str().unwrap().to_string();

    // El hogar arranca con un catálogo por defecto, así que los contadores van en relativo.
    let before_delete = app.count_rows("categories").await;

    let preview = tool_json(
        &mcp_post(&app, &token, tool_call("delete_category", json!({"id": cat}))).await,
    );
    let side = &preview["effects"]["side_effects"];
    assert_eq!(side["remap_to_required"], true, "{preview}");
    assert_eq!(side["references"]["transactions"], 1, "{preview}");
    assert!(side["remap_to_given"].is_null(), "{preview}");

    // Confirmar SIN destino es el 400 que el preview anunciaba, no un borrado silencioso.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_category", json!({"id": cat, "confirm": true})),
    )
    .await;
    match classify(&envelope) {
        Outcome::ToolError { message, .. } => assert!(
            message.starts_with("category_in_use"),
            "confirmar sin remap_to debe dar category_in_use: {message}"
        ),
        other => panic!("esperaba category_in_use y llegó {other:?}"),
    }
    assert_eq!(
        app.count_rows("categories").await,
        before_delete,
        "nada se ha borrado"
    );

    // Con destino: la categoría desaparece y el movimiento SIGUE, reasignado.
    let done = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "delete_category",
                json!({"id": cat, "remap_to": other, "confirm": true}),
            ),
        )
        .await,
    );
    assert_eq!(done["deleted"], true, "{done}");
    assert_eq!(
        app.count_rows("categories").await,
        before_delete - 1,
        "exactamente una categoría menos"
    );
    assert_eq!(app.count_rows("transactions").await, 1, "el movimiento sobrevive");
    let moved = tool_json(&mcp_post(&app, &token, tool_call("list_transactions", json!({}))).await);
    let row = &moved["transactions"][0];
    assert_eq!(row["id"], json!(txn_id), "{moved}");
    assert_eq!(row["category_id"], json!(other), "remapeado, no huérfano: {moved}");
}

/// `confirm_transfer_match`: el argumento es una PROPUESTA del servidor, no dos UUID.
#[tokio::test]
async fn confirm_transfer_match_only_accepts_a_server_issued_match_id() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Par a 10 días: fuera de la ventana del pase automático (5), así que llega sin conciliar.
    let out = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-07-01", "concept": "Traspaso salida", "amount": "-300",
                   "kind": "expense"}),
            &owner.cookie,
        )
        .await;
    let back = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-07-11", "concept": "Traspaso entrada", "amount": "300",
                   "kind": "income"}),
            &owner.cookie,
        )
        .await;
    assert!(
        back.json()["transfer_counterpart_id"].is_null(),
        "precondición: a 10 días el pase automático NO los empareja: {:?}", back.json()
    );
    let out_id = out.json()["id"].as_str().unwrap().to_string();

    // Un `match_id` inventado con la forma correcta no resuelve: el espacio de acciones
    // alcanzables es exactamente el de los pares que el servidor propondría.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "confirm_transfer_match",
            json!({"match_id": "0123456789abcdef01234567"}),
        ),
    )
    .await;
    match classify(&envelope) {
        Outcome::ToolError { message, .. } => assert!(
            message.starts_with("transfer_match_not_found"),
            "un match_id inventado debe dar transfer_match_not_found: {message}"
        ),
        other => panic!("esperaba transfer_match_not_found y llegó {other:?}"),
    }

    let suggestions = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("suggest_transfer_matches", json!({"window_days": 15})),
        )
        .await,
    );
    assert_eq!(suggestions["suggestion_count"], 1, "{suggestions}");
    let match_id = suggestions["suggestions"][0]["match_id"]
        .as_str()
        .expect("match_id")
        .to_string();
    assert_eq!(
        suggestions["suggestions"][0]["within_auto_window"], false,
        "a 10 días queda fuera de la ventana del pase automático: {suggestions}"
    );

    let done = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("confirm_transfer_match", json!({"match_id": match_id.clone()})),
        )
        .await,
    );
    assert_eq!(done["transaction"]["id"], json!(out_id), "{done}");
    assert!(
        !done["transaction"]["transfer_counterpart_id"].is_null(),
        "el par queda conciliado: {done}"
    );

    // Idempotente: reconfirmar el MISMO par devuelve el par, no un 404 sobre trabajo ya hecho.
    let again = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("confirm_transfer_match", json!({"match_id": match_id})),
        )
        .await,
    );
    assert_eq!(again["transaction"]["id"], done["transaction"]["id"], "{again}");

    // Y el par conciliado sale de los agregados de flujo, que es de lo que iba todo esto.
    let agg = tool_json(
        &mcp_post(&app, &token, tool_call("aggregate_transactions", json!({}))).await,
    );
    assert_eq!(agg["transaction_count"], 0, "{agg}");
    assert_eq!(agg["reconciled_excluded_count"], 2, "{agg}");
}

/// `create_batch`: todo-o-nada, y la clave de idempotencia reproduce EL MISMO lote.
#[tokio::test]
async fn create_batch_is_all_or_nothing_and_replays_by_key() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "expense", "Comida").await;

    // Un ítem inválido (categoría inexistente) y no se crea NINGUNO.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_batch",
            json!({"transactions": [
                {"op_date": "2026-07-01", "concept": "ok", "amount": "-10.00", "kind": "expense",
                 "category_id": cat},
                {"op_date": "2026-07-02", "concept": "malo", "amount": "-11.00", "kind": "expense",
                 "category_id": "00000000-0000-4000-8000-000000000001"}
            ]}),
        ),
    )
    .await;
    assert!(
        !matches!(classify(&envelope), Outcome::Success),
        "un ítem inválido debe tumbar el lote entero: {envelope}"
    );
    assert_eq!(app.count_rows("transactions").await, 0, "todo o nada");

    let args = json!({"transactions": [
        {"op_date": "2026-07-01", "concept": "uno", "amount": "-10.00", "kind": "expense",
         "category_id": cat},
        {"op_date": "2026-07-02", "concept": "dos", "amount": "-20.00", "kind": "expense",
         "category_id": cat}
    ], "idempotency_key": "lote-de-la-semana"});
    let first = tool_json(&mcp_post(&app, &token, tool_call("create_batch", args.clone())).await);
    assert_eq!(first["transaction_count"], 2, "{first}");
    assert_eq!(first["summary"].as_array().unwrap().len(), 2, "{first}");
    assert_eq!(app.count_rows("transactions").await, 2);

    // Reenvío del MISMO lote: los mismos ids, sin crear nada.
    let replay = tool_json(&mcp_post(&app, &token, tool_call("create_batch", args)).await);
    assert_eq!(replay["ids"], first["ids"], "la réplica devuelve los ids originales");
    assert_eq!(app.count_rows("transactions").await, 2, "la réplica no crea filas");
}

/// Snapshots por MCP: `kind` inmutable, reemplazo total de ítems y **cache intacta** (D12).
#[tokio::test]
async fn snapshot_tools_backfill_the_past_without_touching_the_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);

    app.warm_default_view(&owner.cookie, &key).await;
    let snap = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_snapshot",
                json!({"kind": "asset", "snapshot_date": "2023-01-31",
                       "items": [{"label": "Fondo", "value": "40000"}]}),
            ),
        )
        .await,
    );
    let snap_id = snap["id"].as_str().unwrap().to_string();
    assert_eq!(snap["affects_projection"], false, "{snap}");
    assert!(
        app.cache_contains(&key).await,
        "contrato D12: un snapshot NO es input del engine, la cache debe sobrevivir"
    );

    // Repetir (usuario, kind, día) es 409, no un segundo snapshot del mismo día.
    let dup = mcp_post(
        &app,
        &token,
        tool_call(
            "create_snapshot",
            json!({"kind": "asset", "snapshot_date": "2023-01-31"}),
        ),
    )
    .await;
    match classify(&dup) {
        Outcome::ToolError { code, .. } => assert_eq!(code, "conflict", "{dup}"),
        other => panic!("esperaba 409 y llegó {other:?}"),
    }

    // `items` omitido conserva; presente reemplaza. Y `kind` ni siquiera se puede pedir.
    let moved = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_snapshot",
                json!({"id": snap_id, "snapshot_date": "2023-02-28"}),
            ),
        )
        .await,
    );
    assert!(moved["summary"].as_str().unwrap().contains("1 ítems"), "{moved}");
    let replaced = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_snapshot",
                json!({"id": snap_id, "items": [
                    {"label": "Fondo", "value": "41000"},
                    {"label": "Cuenta", "value": "3000"}
                ]}),
            ),
        )
        .await,
    );
    assert!(replaced["summary"].as_str().unwrap().contains("2 ítems"), "{replaced}");
    assert!(
        app.cache_contains(&key).await,
        "contrato D12 también en la edición"
    );

    let bad = mcp_post(
        &app,
        &token,
        tool_call("update_snapshot", json!({"id": snap_id, "kind": "liability"})),
    )
    .await;
    assert!(
        matches!(classify(&bad), Outcome::RpcError | Outcome::ToolError { .. }),
        "`kind` no está en el esquema de update_snapshot: {bad}"
    );
}

/// `update_installation_settings`: allowlist estricta, owner-only y preview reversible.
///
/// La ausencia que importa: **`mcp_write_enabled` no está y no puede estar**. Un kill-switch que
/// la propia superficie que corta puede reencender es decorativo.
#[tokio::test]
async fn installation_settings_tool_cannot_reach_the_write_kill_switch() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app
        .register_and_approve_member(&owner, "mario", "member")
        .await;
    let token = create_token(&app, &owner).await;
    let member_token = create_token_for(&app, &member.cookie).await;

    for forbidden in ["mcp_write_enabled", "onboarding_completed"] {
        let envelope = mcp_post(
            &app,
            &token,
            tool_call(
                "update_installation_settings",
                json!({forbidden: true, "confirm": true}),
            ),
        )
        .await;
        assert!(
            !matches!(classify(&envelope), Outcome::Success),
            "`{forbidden}` no puede llegar a esta tool: {envelope}"
        );
    }

    // Preview: valida y devuelve before/after sin persistir.
    let preview = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("update_installation_settings", json!({"show_age_mode": "ages"})),
        )
        .await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["effects"]["entity"]["before"]["show_age_mode"], "dates", "{preview}");
    assert_eq!(preview["effects"]["entity"]["after"]["show_age_mode"], "ages", "{preview}");
    let live = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(
        live.json()["installation"]["show_age_mode"], "dates",
        "el preview no persiste"
    );

    // Un `member` con permiso de escritura sigue sin poder: la comprobación es owner-only y
    // vive DENTRO de la core, no en esta superficie.
    let envelope = mcp_post(
        &app,
        &member_token,
        tool_call(
            "update_installation_settings",
            json!({"show_age_mode": "ages", "confirm": true}),
        ),
    )
    .await;
    match classify(&envelope) {
        Outcome::ToolError { code, .. } => assert_eq!(code, "forbidden", "{envelope}"),
        other => panic!("un member no puede tocar los ajustes y llegó {other:?}"),
    }

    // Y el owner sí, con su bloque `impact`.
    let done = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_installation_settings",
                json!({"show_age_mode": "ages", "confirm": true}),
            ),
        )
        .await,
    );
    assert_eq!(done["applied"], true, "{done}");
    assert!(done["impact"].is_object(), "mueve la proyección: {done}");
    let live = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(live.json()["installation"]["show_age_mode"], "ages");
}

/// La puerta del sumidero era saltable **en dos pasos**: crear un `remainder` CON tope (legítimo
/// desde MCP) y quitárselo después con `cap: null`. `SinkPolicy` solo llegaba a la core de
/// creación, así que la descripción de la tool prometía algo que el catálogo no cumplía.
///
/// El arreglo es la misma doctrina que el invariante del módulo: la puerta mira el **estado
/// resultante**, no la operación. Este test recorre el camino completo, porque una guardia solo
/// en el `create` deja verde cualquier test que solo pruebe el `create`.
#[tokio::test]
async fn the_sink_cannot_be_forged_by_editing_a_capped_remainder() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Indexado", "current_value": "1000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();

    // Paso 1: un `remainder` CON tope sí se puede crear desde MCP — no es el sumidero.
    let created = mcp_post(
        &app,
        &token,
        tool_call(
            "create_allocation_rule",
            json!({
                "target_asset_id": asset_id,
                "kind": "remainder",
                "cap_kind": "amount",
                "cap_value": "5000",
            }),
        ),
    )
    .await;
    assert!(
        matches!(classify(&created), Outcome::Success),
        "un remainder CON tope debe poder crearse: {created:?}"
    );
    let rule_id = tool_json(&created)["id"]
        .as_str()
        .expect("id de la regla")
        .to_string();

    // Paso 2: quitarle el tope lo convertiría en el sumidero. Es el agujero que había.
    let patched = mcp_post(
        &app,
        &token,
        tool_call(
            "update_allocation_rule",
            json!({"id": rule_id, "clear_cap": true}),
        ),
    )
    .await;
    match classify(&patched) {
        Outcome::ToolError { message, .. } => assert!(
            message.starts_with("sink_creation_not_allowed"),
            "editar hasta el sumidero debe rechazarse con el mismo código que crearlo: {message}"
        ),
        other => panic!("el sumidero se fabricó editando — el agujero sigue abierto: {other:?}"),
    }

    // Y la regla sobrevive intacta: un rechazo no puede dejarla a medio editar.
    let rules = app.get_with_cookie("/v1/allocation-rules", &owner.cookie).await;
    let still_capped = rules.json().as_array().expect("array de reglas").iter().any(|r| {
        r["id"].as_str() == Some(rule_id.as_str()) && !r["cap_kind"].is_null()
    });
    assert!(still_capped, "la regla debe conservar su tope tras el rechazo");
}

/// #148 — cuarteto de la ventana recurrente sobre create/update_planning_flow. La invalidación
/// FULL y el toggle `mcp_write_enabled` de estas dos tools ya están cubiertos por los barridos
/// genéricos de este fichero (líneas «create_planning_flow: FULL» y el sweep del toggle): aquí
/// van el core compartido, el error de dominio con el mismo código de wire y los tri-estados
/// nuevos.
#[tokio::test]
async fn planning_flow_recurring_window_via_mcp() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "income", "Alquileres").await;

    // (1) Core compartido: la fila creada por MCP es indistinguible de la del POST HTTP.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_planning_flow",
            json!({"title": "Alquiler", "category_id": cat, "expected_amount": "800",
                   "amount_basis": "per_month", "window_start_date": "2026-09-01",
                   "window_end_date": "2029-08-31"}),
        ),
    )
    .await;
    let flow = tool_json(&envelope);
    assert!(flow["summary"].as_str().unwrap().contains("€/mes"), "{flow}");
    let flow_id = flow["id"].as_str().unwrap().to_string();
    let rows = app.get_with_cookie("/v1/planning/flows", &owner.cookie).await.json();
    assert_eq!(rows[0]["amount_basis"], "per_month", "{rows}");
    assert_eq!(rows[0]["window_start_date"], "2026-09-01", "{rows}");
    assert_eq!(rows[0]["window_end_date"], "2029-08-31", "{rows}");

    // (3) Error de dominio compartido: mismo código de wire por las dos superficies.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_planning_flow",
            json!({"title": "Mal", "category_id": cat, "expected_amount": "10",
                   "amount_basis": "per_month", "window_start_date": "2027-01-01",
                   "window_end_date": "2026-01-01"}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");
    let http = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            json!({"category_id": cat, "title": "Mal", "expected_amount": "10",
                   "amount_basis": "per_month", "window_start_date": "2027-01-01",
                   "window_end_date": "2026-01-01"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(http.status, ::http::StatusCode::BAD_REQUEST, "{http:?}");
    let msg = http.json()["message"].as_str().unwrap_or_default().to_string();
    assert!(msg.starts_with("window_end_before_start"), "{msg}");

    // clear_window_end: la ventana pasa a sin fin (tri-estado), y set+clear a la vez es 400.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_planning_flow", json!({"id": flow_id, "clear_window_end": true})),
    )
    .await;
    let updated = tool_json(&envelope);
    assert!(updated["summary"].as_str().unwrap().contains("sin fin"), "{updated}");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_planning_flow",
            json!({"id": flow_id, "window_end_date": "2030-01-01", "clear_window_end": true}),
        ),
    )
    .await;
    tool_error(&envelope, "bad_request");

    // Vuelta a one_off: nada se auto-borra — sin limpiar la ventana el estado resultante es
    // incoherente y se rechaza; con clear_window_start el cambio entra.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_planning_flow", json!({"id": flow_id, "amount_basis": "one_off"})),
    )
    .await;
    tool_error(&envelope, "bad_request");
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_planning_flow",
            json!({"id": flow_id, "amount_basis": "one_off", "clear_window_start": true}),
        ),
    )
    .await;
    let _ = tool_json(&envelope);
    let rows = app.get_with_cookie("/v1/planning/flows", &owner.cookie).await.json();
    assert_eq!(rows[0]["amount_basis"], "one_off", "{rows}");
    assert!(rows[0].get("window_start_date").is_none(), "{rows}");
    assert!(rows[0].get("window_end_date").is_none(), "{rows}");
}

/// `update_category` con `is_fallback` (4.15.0): el cuarteto de una tool de escritura sobre el
/// eje nuevo — core compartida, indistinguible vía HTTP, contrato de cache NONE y el error de
/// dominio con el MISMO `code` que el wire HTTP. El gate del rol/toggle lo cubre la batería
/// genérica (`write_tool_names`), que ya incluye `update_category`.
///
/// Por qué merece test propio y no una línea en el de renombrar: `is_fallback: true` no edita un
/// campo, hace un SWAP — desmarca la categoría por defecto anterior del scope y marca ésta. Si el
/// swap se rompiera a medias, el índice único parcial dejaría la instalación con CERO categorías
/// por defecto en ese scope, y entonces todo movimiento income/expense sin categoría empezaría a
/// fallar con `fallback_category_missing` en las dos superficies a la vez.
#[tokio::test]
async fn update_category_designates_the_scope_fallback_and_shares_the_core() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    // Modo B: las transacciones son inputs del motor, así que una invalidación indebida se vería.
    set_mode(&app, &owner.cookie, "transactions_avg").await;

    let compras = app.create_category(&owner, "expense", "Compras online").await;
    let fondos = app.create_category(&owner, "asset", "Fondos").await;

    let fallback_before = expense_fallback_id(&app, &owner.cookie).await;
    assert_ne!(fallback_before, compras, "precondición: la semilla trae otra por defecto");

    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);
    app.warm_default_view(&owner.cookie, &key).await;

    // 1. La tool designa la nueva por defecto.
    let out = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("update_category", json!({"id": compras, "is_fallback": true})),
        )
        .await,
    );
    assert_eq!(out["id"], compras.as_str(), "{out}");

    // 2. Indistinguible vía HTTP, y el SWAP es exactamente eso: una y solo una por scope.
    let rows = app.get_with_cookie("/v1/categories", &owner.cookie).await.json();
    let expense_fallbacks: Vec<&serde_json::Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["scope"] == "expense" && c["is_fallback"] == json!(true))
        .collect();
    assert_eq!(expense_fallbacks.len(), 1, "una por scope, ni cero ni dos: {rows}");
    assert_eq!(expense_fallbacks[0]["id"], compras.as_str(), "{rows}");
    let previous = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(fallback_before))
        .expect("la anterior sigue existiendo, solo desmarcada");
    assert_eq!(previous["is_fallback"], json!(false), "{previous}");

    // 3. Cache: NONE. Designar la categoría por defecto no mueve ni una transacción ni un importe,
    //    así que la proyección no puede cambiar — ni siquiera en modo B.
    assert!(
        app.cache_contains(&key).await,
        "designar la categoría por defecto NUNCA invalida la cache de proyección"
    );

    // 4. Errores de dominio, con el MISMO `code` por las dos superficies (misma core).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_category", json!({"id": compras, "is_fallback": false})),
    )
    .await;
    assert_eq!(
        tool_error(&envelope, "bad_request")["code"],
        "fallback_cannot_be_unset",
        "desmarcar dejaría el scope sin destino por defecto: hay que designar OTRA"
    );
    let http = app
        .patch_json_with_cookie(
            &format!("/v1/categories/{compras}"),
            json!({"is_fallback": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(http.status, http::StatusCode::BAD_REQUEST, "{http:?}");
    assert_eq!(http.json()["code"], "fallback_cannot_be_unset", "{http:?}");

    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_category", json!({"id": fondos, "is_fallback": true})),
    )
    .await;
    assert_eq!(
        tool_error(&envelope, "bad_request")["code"],
        "fallback_scope_invalid",
        "solo income/expense tienen categoría por defecto"
    );
}

/// Id de la categoría por defecto de gasto de la instalación. Falla alto si no hay exactamente
/// una: el invariante de 4.15.0 es que siempre haya una y solo una por scope.
async fn expense_fallback_id(app: &TestApp, cookie: &str) -> String {
    let rows = app.get_with_cookie("/v1/categories", cookie).await.json();
    let found: Vec<String> = rows
        .as_array()
        .expect("categories list")
        .iter()
        .filter(|c| c["scope"] == "expense" && c["is_fallback"] == json!(true))
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(found.len(), 1, "una categoría por defecto de gasto, y solo una: {rows}");
    found.into_iter().next().unwrap()
}

// ---------------------------------------------------------------------------
// 5.0.0 — D21 por MCP y el perfil de jubilación por usuario (D13)
// ---------------------------------------------------------------------------

/// **D21 llega gratis al MCP porque las tools comparten las cores.** Este test es la evidencia:
/// el token de un miembro no puede mover el activo de otro por `update_asset`, `update_asset_value`
/// ni `delete_asset`, y el error es el MISMO `not_row_owner` del HTTP.
///
/// Sin él, la puerta se podría añadir solo al handler HTTP y el MCP quedaría abierto — que es
/// exactamente la forma del dual-branch drift que ya mordió dos veces (Fase 2 y Fase 6).
#[tokio::test]
async fn mcp_writes_cannot_touch_another_members_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let member_token = create_token_for(&app, &member.cookie).await;

    // Un activo del OWNER.
    let cat = app.create_category(&owner, "asset", "Indexados").await;
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat, "name": "MSCI World", "current_value": "10000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let asset_id = created.json()["id"].as_str().expect("asset id").to_string();

    for call in [
        tool_call("update_asset", json!({"asset_id": asset_id, "name": "secuestrado"})),
        tool_call(
            "update_asset_value",
            json!({"asset_id": asset_id, "current_value": "1"}),
        ),
        tool_call("delete_asset", json!({"id": asset_id, "confirm": false})),
    ] {
        let envelope = mcp_post(&app, &member_token, call.clone()).await;
        let body = tool_error(&envelope, "forbidden");
        assert_eq!(
            body["code"], "not_row_owner",
            "{call} debía dar el código de fila ajena y dio: {body}"
        );
    }

    // Y el activo sigue igual.
    let listed = app.get_with_cookie("/v1/assets", &owner.cookie).await;
    let rows = listed.json();
    let row = rows
        .as_array()
        .expect("assets array")
        .iter()
        .find(|a| a["id"] == asset_id.as_str())
        .expect("sigue ahí");
    assert_eq!(row["name"], "MSCI World", "{row}");
    assert_eq!(row["current_value"], "10000.0000", "{row}");
}

/// Las dos tools nuevas del perfil: preview/confirm, merge campo a campo y — lo que las separa de
/// `update_fire_settings` — **auth por ROL, no owner-only**. El perfil es dato del usuario del
/// token, así que un `viewer` edita el suyo y nadie el de otro (no hay parámetro para pedirlo).
#[tokio::test]
async fn retirement_profile_tools_are_personal_and_preview_before_writing() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;

    // Lectura: los defaults de 4.15.x.
    let got = tool_json(&mcp_post(&app, &token, tool_call("get_retirement_profile", json!({}))).await);
    assert_eq!(got["profile"]["strategy"], "asap", "{got}");
    assert_eq!(got["profile"]["swr_pct"], "3.5", "{got}");
    assert_eq!(got["birth_date"], "1990-01-01", "{got}");

    // Preview: valida y enseña before/after, sin persistir.
    let preview = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_retirement_profile",
                json!({"strategy": "retire_at_age", "target_retirement_age": 57, "swr_pct": "3.0"}),
            ),
        )
        .await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["effects"]["entity"]["before"]["strategy"], "asap");
    assert_eq!(preview["effects"]["entity"]["after"]["strategy"], "retire_at_age");
    // El radio es UNA persona, no la instalación: es lo que lo distingue de update_fire_settings.
    assert_eq!(preview["effects"]["side_effects"]["scope"], "user", "{preview}");
    let still = tool_json(&mcp_post(&app, &token, tool_call("get_retirement_profile", json!({}))).await);
    assert_eq!(still["profile"]["strategy"], "asap", "el preview no persiste: {still}");

    // Confirm: escribe, y el merge NO resetea lo que no nombra.
    let applied = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "update_retirement_profile",
                json!({"strategy": "retire_at_age", "target_retirement_age": 57, "swr_pct": "3.0", "confirm": true}),
            ),
        )
        .await,
    );
    assert_eq!(applied["applied"], true, "{applied}");
    let applied2 = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("update_retirement_profile", json!({"cash_buffer_months": 12, "confirm": true})),
        )
        .await,
    );
    let after = &applied2["outcome"]["after"];
    assert_eq!(after["cash_buffer_months"], 12, "{applied2}");
    assert_eq!(after["target_retirement_age"], 57, "el merge no resetea: {applied2}");
    assert_eq!(after["swr_pct"], "3.0", "{applied2}");

    // Las mismas cotas que el PATCH HTTP, con el mismo código.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("update_retirement_profile", json!({"swr_pct": "9", "confirm": true})),
    )
    .await;
    tool_error(&envelope, "bad_request");

    // `clear_*` y su valor a la vez es una intención contradictoria, no un ganador implícito.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_retirement_profile",
            json!({"cash_buffer_months": 6, "clear_cash_buffer_months": true}),
        ),
    )
    .await;
    let body = tool_error(&envelope, "bad_request");
    assert_eq!(body["code"], "field_set_and_clear", "{body}");

    // Un VIEWER edita el SUYO: el gate es por rol (require_mcp_write), no owner-only… salvo que
    // `viewer` no es rol de escritura, así que se comprueba con un `member`, que sí lo es, y que
    // con `update_fire_settings` recibiría un 403.
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let member_token = create_token_for(&app, &member.cookie).await;
    let mine = tool_json(
        &mcp_post(
            &app,
            &member_token,
            tool_call(
                "update_retirement_profile",
                json!({"strategy": "coast", "target_retirement_age": 50, "confirm": true}),
            ),
        )
        .await,
    );
    assert_eq!(mine["applied"], true, "un member configura SU jubilación: {mine}");
    // Y no ha tocado la del owner.
    let owners = tool_json(&mcp_post(&app, &token, tool_call("get_retirement_profile", json!({}))).await);
    assert_eq!(owners["profile"]["strategy"], "retire_at_age", "{owners}");
}

/// **`update_asset` puede BORRAR la rentabilidad esperada y la volatilidad** (5.0.0, WP5-2).
///
/// Un JSON Schema de tool no puede expresar «omitir vs null», así que el tri-estado del PATCH
/// viaja por `clear_*` — el mismo molde que `clear_purchase_price` desde 4.x. Sin esto, un modelo
/// que escribió una volatilidad por error solo podía deshacerlo borrando y recreando el activo:
/// la tool tenía la capacidad de romper un estado que no tenía la capacidad de reparar.
#[tokio::test]
async fn update_asset_clears_the_return_and_the_volatility() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat = app.create_category(&owner, "asset", "Indexados").await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_asset",
            json!({"name": "RV global", "category_id": cat, "current_value": "10000",
                   "expected_annual_return_percent": "6", "annual_volatility_percent": "16"}),
        ),
    )
    .await;
    let asset_id = tool_json(&envelope)["id"].as_str().unwrap().to_string();

    async fn row(app: &TestApp, cookie: &str, id: &str) -> serde_json::Value {
        let rows = app.get_with_cookie("/v1/assets", cookie).await.json();
        rows.as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .cloned()
            .expect("el activo sigue ahí")
    }

    // Los dos `clear_*` en la misma llamada: el activo vuelve a determinista y sin rentabilidad.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_asset",
            json!({"asset_id": asset_id, "clear_expected_annual_return_percent": true,
                   "clear_annual_volatility_percent": true}),
        ),
    )
    .await;
    assert!(tool_json(&envelope)["summary"].is_string(), "{envelope}");
    let a = row(&app, &owner.cookie, &asset_id).await;
    assert!(a["expected_annual_return_percent"].is_null(), "{a}");
    assert!(a["annual_volatility_percent"].is_null(), "{a}");

    // Valor y `clear_*` a la vez es una intención contradictoria: 400 con el código compartido
    // con el perfil de jubilación (`field_set_and_clear`), no una elección a ciegas.
    for body in [
        json!({"asset_id": asset_id, "expected_annual_return_percent": "5",
               "clear_expected_annual_return_percent": true}),
        json!({"asset_id": asset_id, "annual_volatility_percent": "15",
               "clear_annual_volatility_percent": true}),
    ] {
        let envelope = mcp_post(&app, &token, tool_call("update_asset", body.clone())).await;
        let err = tool_error(&envelope, "bad_request");
        assert_eq!(err["code"], "field_set_and_clear", "{body} → {err}");
    }

    // Y el camino normal (poner un valor) sigue funcionando después de haberlos borrado.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "update_asset",
            json!({"asset_id": asset_id, "annual_volatility_percent": "18"}),
        ),
    )
    .await;
    assert!(tool_json(&envelope)["summary"].is_string(), "{envelope}");
    let a = row(&app, &owner.cookie, &asset_id).await;
    assert_eq!(a["annual_volatility_percent"], "18.0000", "{a}");
    assert!(a["expected_annual_return_percent"].is_null(), "borrada sigue borrada: {a}");
}
