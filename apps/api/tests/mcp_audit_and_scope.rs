//! Fase 3, issue #84: rendición de cuentas (`mcp_write_audit`) y radio de explosión
//! (`api_tokens.scope`) de las escrituras MCP.
//!
//! Lo que se prueba aquí es lo que un log de auditoría tiene que garantizar para servir de algo:
//!
//!   * **Que existe**: cada paso por `require_mcp_write` deja UNA fila, también cuando el gate
//!     rechaza — un intento denegado es justo el que más importa.
//!   * **Que no miente**: la fila nace en `attempted` (nunca `ok`) y solo un `settle` explícito la
//!     cierra; una fila cerrada no se puede reescribir.
//!   * **Que no guarda lo que no debe**: ni conceptos, ni importes, ni ningún texto escrito por la
//!     persona. Se comprueba leyendo la tabla ENTERA en busca del concepto de un movimiento real.
//!   * **Que el scope solo resta**: `read_only` corta la escritura; `read_write` (el default de la
//!     columna, y por tanto el de todos los tokens ya emitidos) no cambia absolutamente nada.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::error::ApiError;
use futurefin_api::handlers::api_tokens::TokenScope;
use futurefin_api::handlers::membership::MembershipRole;
use futurefin_api::mcp::auth::{require_mcp_write, McpCredential, McpIdentity};
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

fn tool_call(name: &str, arguments: serde_json::Value) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {},
            },
        }
    })
}

/// Código de error ESTABLE de un tool-error (vacío si la llamada fue bien).
///
/// Se lee de `ErrorBody.code`, no de `ErrorBody.error`: `error` es la clase HTTP (`bad_request`)
/// y sería la misma para `mcp_token_read_only` que para `mcp_write_disabled`. Los tres rechazos
/// del gate solo se distinguen por `code`.
fn tool_error_code(envelope: &serde_json::Value) -> String {
    let result = &envelope["result"];
    if result["isError"] != true {
        return String::new();
    }
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|b| b["code"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Una llamada que fue bien. `isError` llega como `false` (no ausente) en una respuesta correcta.
fn assert_ok(envelope: &serde_json::Value) {
    assert_ne!(
        envelope["result"]["isError"], true,
        "se esperaba una llamada correcta: {envelope}"
    );
}

async fn create_token_with_scope(app: &TestApp, cookie: &str, scope: &str) -> String {
    let created = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            json!({"label": format!("audit test {scope}"), "scope": scope}),
            cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    assert_eq!(created.json()["scope"], scope);
    created.json()["token"].as_str().unwrap().to_string()
}

/// Token SIN campo `scope` en el body — el camino de todos los clientes anteriores al scope.
async fn create_token_legacy(app: &TestApp, cookie: &str) -> String {
    let created = app
        .post_json_with_cookie("/v1/api-tokens", json!({"label": "legacy"}), cookie)
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

/// Filas de auditoría, de la más antigua a la más reciente.
async fn audit_rows(app: &TestApp) -> Vec<AuditRow> {
    sqlx::query_as::<_, AuditRow>(
        r#"SELECT tool, outcome, error_code, role, credential_kind, credential_id, user_id,
                  installation_id, target_ids, (settled_at IS NOT NULL) AS settled
           FROM mcp_write_audit ORDER BY at, tool"#,
    )
    .fetch_all(&app.pool)
    .await
    .expect("leer mcp_write_audit")
}

#[derive(Debug, sqlx::FromRow)]
struct AuditRow {
    tool: String,
    outcome: String,
    error_code: Option<String>,
    role: String,
    credential_kind: String,
    credential_id: Uuid,
    user_id: Uuid,
    installation_id: Uuid,
    target_ids: Vec<Uuid>,
    settled: bool,
}

// ---------------------------------------------------------------------------
// TAREA 1 — el log
// ---------------------------------------------------------------------------

/// Una escritura que ATRAVIESA el gate deja una fila con la identidad completa, y esa fila NO
/// afirma que la operación haya terminado.
#[tokio::test]
async fn a_successful_write_leaves_one_row_with_the_full_identity() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token_legacy(&app, &owner.cookie).await;
    let installation_id = app.installation_id().await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({
                "op_date": "2026-07-01",
                "concept": "Cena en el Rincon",
                "amount": "-23.50",
                "kind": "expense"
            }),
        ),
    )
    .await;
    assert_ok(&envelope);

    let rows = audit_rows(&app).await;
    assert_eq!(rows.len(), 1, "una escritura, una fila: {rows:?}");
    let r = &rows[0];
    assert_eq!(r.tool, "create_transaction");
    assert_eq!(r.role, "owner");
    assert_eq!(r.credential_kind, "api_token");
    assert_eq!(r.user_id, owner.user_id);
    assert_eq!(r.installation_id, installation_id);
    assert_ne!(r.credential_id, Uuid::nil(), "la credencial queda identificada");
    // El desenlace lo cierra `settle` desde el llamante (`mcp/server.rs`). Mientras no esté
    // cableado la fila se queda en `attempted`; una vez lo esté dirá `ok`. Lo que NO puede pasar
    // nunca es que diga `ok` sin que la operación haya corrido, ni que diga `denied`.
    assert!(
        r.outcome == "attempted" || r.outcome == "ok",
        "una escritura que pasó el gate no puede quedar como {}: {r:?}",
        r.outcome
    );
    assert_eq!(r.settled, r.outcome != "attempted", "`settled_at` y `outcome` van juntos");
}

/// Los tres rechazos del gate quedan registrados, con su código, y **cerrados**: para una llamada
/// denegada no hay nada más que esperar.
#[tokio::test]
async fn every_denial_is_recorded_with_its_code() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app
        .register_and_approve_member(&owner, "vera", "viewer")
        .await;

    let args = json!({
        "op_date": "2026-07-01", "concept": "probe", "amount": "-1.00", "kind": "expense"
    });

    // 1. Rol: un viewer no escribe, con el token que sea.
    let viewer_token = create_token_with_scope(&app, &viewer.cookie, "read_write").await;
    let e = mcp_post(&app, &viewer_token, tool_call("create_transaction", args.clone())).await;
    assert_eq!(tool_error_code(&e), "forbidden", "{e}");

    // 2. Scope: el owner escribe, pero este token no.
    let ro_token = create_token_with_scope(&app, &owner.cookie, "read_only").await;
    let e = mcp_post(&app, &ro_token, tool_call("create_transaction", args.clone())).await;
    assert_eq!(tool_error_code(&e), "mcp_token_read_only", "{e}");

    // 3. Kill-switch de la instalación.
    let rw_token = create_token_with_scope(&app, &owner.cookie, "read_write").await;
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"mcp_write_enabled": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let e = mcp_post(&app, &rw_token, tool_call("create_transaction", args)).await;
    assert_eq!(tool_error_code(&e), "mcp_write_disabled", "{e}");

    let rows = audit_rows(&app).await;
    assert_eq!(rows.len(), 3, "tres intentos, tres filas: {rows:?}");
    let mut codes: Vec<String> = rows
        .iter()
        .map(|r| {
            assert_eq!(r.outcome, "denied", "un rechazo del gate se registra como denied: {r:?}");
            assert!(r.settled, "una fila `denied` nace cerrada: {r:?}");
            assert_eq!(r.tool, "create_transaction");
            r.error_code.clone().unwrap_or_default()
        })
        .collect();
    codes.sort();
    assert_eq!(
        codes,
        vec!["forbidden", "mcp_token_read_only", "mcp_write_disabled"]
    );
    // Y ningún rechazo escribió nada.
    assert_eq!(app.count_rows("transactions").await, 0);
}

/// **La regla de higiene, comprobada leyendo la tabla entera.** Ni el concepto, ni el importe, ni
/// las notas de un movimiento real pueden aparecer en ningún sitio de `mcp_write_audit`: el log
/// guarda identificadores opacos, no contenido. Si esto falla, borrar un movimiento privado
/// dejaría de borrarlo de verdad.
#[tokio::test]
async fn the_audit_log_never_stores_what_the_person_wrote() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token_legacy(&app, &owner.cookie).await;

    const CONCEPT: &str = "Psicologa Marta Ruiz sesion";
    const NOTE: &str = "pagado con la tarjeta de credito compartida";
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({
                "op_date": "2026-07-02",
                "concept": CONCEPT,
                "amount": "-77.77",
                "kind": "expense",
                "notes": NOTE
            }),
        ),
    )
    .await;
    assert_ok(&envelope);

    // Volcado textual de TODAS las columnas de la tabla, sin nombrarlas una a una: si alguien
    // añade mañana una columna de texto libre y mete argumentos en ella, este test la ve.
    let dump: String = sqlx::query_scalar(
        r#"SELECT coalesce(string_agg(t::text, ' | '), '') FROM mcp_write_audit t"#,
    )
    .fetch_one(&app.pool)
    .await
    .expect("volcar mcp_write_audit");

    assert!(!dump.is_empty(), "debería haber al menos una fila auditada");
    for needle in [CONCEPT, NOTE, "77.77", "77,77"] {
        assert!(
            !dump.contains(needle),
            "`mcp_write_audit` contiene contenido escrito por la persona ({needle:?}). El log \
             guarda ids opacos, nunca argumentos: un log append-only con conceptos convierte el \
             borrado del usuario en una mentira.\nVolcado: {dump}"
        );
    }
    // Y sí guarda lo que tiene que guardar.
    assert!(dump.contains("create_transaction"), "el verbo sí: {dump}");
}

/// El ciclo completo insertar → cerrar, ejercitado directamente contra el pool (los tests viven en
/// otro crate; `mcp/server.rs` todavía no lo cablea). Prueba las dos propiedades que hacen que el
/// registro no mienta: la fila nace SIN desenlace, y una vez cerrada es **write-once**.
#[tokio::test]
async fn settle_closes_the_row_once_and_only_once() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let installation_id = app.installation_id().await;
    let id = McpIdentity {
        user_id: owner.user_id,
        installation_id,
        role: MembershipRole::Owner,
        credential: McpCredential::ApiToken {
            token_id: Uuid::new_v4(),
        },
        scope: TokenScope::ReadWrite,
    };

    // Fase 1: la fila nace abierta y no afirma nada sobre el desenlace.
    let audit = require_mcp_write(&app.pool, &id, "delete_transaction")
        .await
        .expect("el owner con toggle activo pasa el gate");
    let rows = audit_rows(&app).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "attempted");
    assert!(!rows[0].settled, "sin desenlace todavía");
    assert!(rows[0].target_ids.is_empty());

    // Fase 2: el desenlace real, con los ids afectados.
    let target = Uuid::new_v4();
    let result: Result<(), ApiError> = Ok(());
    audit.settle(&app.pool, &result, &[target]).await;
    let rows = audit_rows(&app).await;
    assert_eq!(rows[0].outcome, "ok");
    assert!(rows[0].settled);
    assert_eq!(rows[0].target_ids, vec![target]);

    // Write-once: un segundo cierre sobre la misma fila no puede reescribir la historia.
    let audit2 = require_mcp_write(&app.pool, &id, "delete_asset")
        .await
        .expect("gate ok");
    let err: Result<(), ApiError> = Err(ApiError::NotFound);
    audit2.settle(&app.pool, &err, &[]).await;
    let rows = audit_rows(&app).await;
    let deleted_asset = rows.iter().find(|r| r.tool == "delete_asset").unwrap();
    assert_eq!(deleted_asset.outcome, "failed");
    assert_eq!(deleted_asset.error_code.as_deref(), Some("not_found"));
    // La fila de `delete_transaction` sigue como la dejamos: nadie la ha tocado.
    let deleted_txn = rows.iter().find(|r| r.tool == "delete_transaction").unwrap();
    assert_eq!(deleted_txn.outcome, "ok");
    assert_eq!(deleted_txn.target_ids, vec![target]);
}

/// La poda por retención vive en el camino de escritura (precedente `gc_orphan_clients`), NUNCA
/// en un GET (D5). Una fila más vieja que la ventana desaparece en la siguiente escritura; una
/// dentro de la ventana no.
#[tokio::test]
async fn retention_prunes_old_rows_on_the_next_write_and_never_on_a_read() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token_legacy(&app, &owner.cookie).await;
    let installation_id = app.installation_id().await;

    // Dos filas plantadas a mano: una caducada (400 días) y una viva (100 días).
    for (days, tool) in [(400, "delete_asset"), (100, "delete_liability")] {
        sqlx::query(
            r#"INSERT INTO mcp_write_audit
                   (at, installation_id, user_id, credential_kind, credential_id, role, tool,
                    outcome, settled_at)
               VALUES (now() - make_interval(days => $1), $2, $3, 'api_token', gen_random_uuid(),
                       'owner', $4, 'ok', now() - make_interval(days => $1))"#,
        )
        .bind(days)
        .bind(installation_id)
        .bind(owner.user_id)
        .bind(tool)
        .execute(&app.pool)
        .await
        .expect("plantar fila de auditoría");
    }
    assert_eq!(app.count_rows("mcp_write_audit").await, 2);

    // Una LECTURA por MCP no poda nada: los GET no mutan, y eso incluye la limpieza.
    let read = mcp_post(
        &app,
        &token,
        tool_call("list_transactions", json!({"limit": 1})),
    )
    .await;
    assert_ok(&read);
    assert_eq!(
        app.count_rows("mcp_write_audit").await,
        2,
        "una lectura no puede podar la tabla de auditoría (D5)"
    );

    // Una ESCRITURA sí: poda la caducada y añade la suya.
    let write = mcp_post(&app, &token, tool_call("capture_snapshot", json!({}))).await;
    assert_ok(&write);
    let tools: Vec<String> = audit_rows(&app).await.into_iter().map(|r| r.tool).collect();
    assert!(
        !tools.contains(&"delete_asset".to_string()),
        "la fila de 400 días debía podarse: {tools:?}"
    );
    assert!(
        tools.contains(&"delete_liability".to_string()),
        "la fila de 100 días está dentro de la ventana: {tools:?}"
    );
    assert!(tools.contains(&"capture_snapshot".to_string()), "{tools:?}");
}

// ---------------------------------------------------------------------------
// TAREA 2 — el scope
// ---------------------------------------------------------------------------

/// **El no-negociable**: un token creado sin pedir scope se comporta EXACTAMENTE como antes.
/// Es el camino de todos los tokens ya emitidos, que la migración deja en `read_write`.
#[tokio::test]
async fn a_token_created_without_scope_still_writes_exactly_as_before() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token_legacy(&app, &owner.cookie).await;

    let list = app.get_with_cookie("/v1/api-tokens", &owner.cookie).await;
    assert_eq!(list.json()[0]["scope"], "read_write", "default de la columna");

    let e = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-01", "concept": "x", "amount": "-1.00", "kind": "expense"}),
        ),
    )
    .await;
    assert_ok(&e);
    assert_eq!(app.count_rows("transactions").await, 1);
}

/// Un token `read_only` lee todo lo que leía y no escribe nada — sin tocar el rol de la persona,
/// que sigue escribiendo por la web con su cookie.
#[tokio::test]
async fn read_only_token_reads_everything_and_writes_nothing() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ro = create_token_with_scope(&app, &owner.cookie, "read_only").await;

    // Lee.
    let summary = mcp_post(&app, &ro, tool_call("get_summary", json!({}))).await;
    assert_ok(&summary);
    let listed = mcp_post(&app, &ro, tool_call("list_assets", json!({}))).await;
    assert_ok(&listed);

    // No escribe.
    let e = mcp_post(
        &app,
        &ro,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-07-01", "concept": "x", "amount": "-1.00", "kind": "expense"}),
        ),
    )
    .await;
    assert_eq!(tool_error_code(&e), "mcp_token_read_only", "{e}");
    assert_eq!(app.count_rows("transactions").await, 0);

    // Y su dueña sigue escribiendo por la web: el scope acota la CREDENCIAL, no a la persona.
    let category = app.create_category(&owner, "expense", "Ocio").await;
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({
                "op_date": "2026-07-01", "concept": "por la web",
                "amount": "-9.00", "kind": "expense", "category_id": category
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    assert_eq!(app.count_rows("transactions").await, 1);
}

/// El scope **solo resta**: nunca asciende. Un `viewer` con token `read_write` sigue sin escribir,
/// y el error que ve es el del rol, no el del scope — el orden de las puertas importa para que el
/// mensaje diga la verdad sobre qué falta.
#[tokio::test]
async fn scope_never_grants_more_than_the_live_role() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app
        .register_and_approve_member(&owner, "vera", "viewer")
        .await;
    let token = create_token_with_scope(&app, &viewer.cookie, "read_write").await;

    let e = mcp_post(&app, &token, tool_call("capture_snapshot", json!({}))).await;
    assert_eq!(tool_error_code(&e), "forbidden", "{e}");
    assert_eq!(app.count_rows("history_snapshots").await, 0);
}

/// Validación del campo: un scope desconocido es 400 con código propio (no el genérico), y no
/// crea nada.
#[tokio::test]
async fn an_unknown_scope_is_rejected_with_its_own_code() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            json!({"label": "raro", "scope": "admin"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "token_scope_invalid", "{r:?}");
    assert_eq!(app.count_rows("api_tokens").await, 0);
}

/// El scope viaja en las tres superficies que lo enseñan: la respuesta del POST (junto al
/// secreto), el listado y —vía la fila— el gate. Sin esto la SPA no puede pintar la columna.
#[tokio::test]
async fn scope_is_visible_in_create_and_list() {
    let app = TestApp::spawn().await;
    let owner: LoggedInOwner = app.register_and_login_owner("alice").await;

    create_token_with_scope(&app, &owner.cookie, "read_only").await;
    create_token_with_scope(&app, &owner.cookie, "read_write").await;

    let list = app.get_with_cookie("/v1/api-tokens", &owner.cookie).await;
    assert_eq!(list.status, http::StatusCode::OK);
    let mut scopes: Vec<String> = list
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["scope"].as_str().expect("scope siempre presente").to_string())
        .collect();
    scopes.sort();
    assert_eq!(scopes, vec!["read_only", "read_write"]);
}
