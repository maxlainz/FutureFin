//! Fase 3, issue #84 — lo que hace que una escritura MCP sea *contable* y *deliberada*:
//!
//!   * **El ciclo de auditoría completo.** La Fase 3 dejó la fila naciendo en `attempted`; aquí se
//!     comprueba que `mcp/server.rs` la CIERRA: `ok` con los ids realmente mutados, `failed` con el
//!     código estable del error, y `ok` con la lista vacía en un preview — que es lo que separa en
//!     el log un borrado consumado de un sondeo.
//!   * **La confirmación en dos fases de verdad.** `confirm: true` es un booleano que escribe el
//!     propio modelo: sobre una fila jamás previsualizada la borraba al instante. El
//!     `confirm_token` sólo lo emite el preview, vale una vez, caduca, es del usuario que
//!     previsualizó y va ligado a la HUELLA DE LOS EFECTOS — si el mundo se mueve entre las dos
//!     llamadas, la confirmación se rechaza en vez de destruir algo distinto de lo que se enseñó.
//!   * **Las tres destructivas que no tenían barrera**: `materialize_recurring` (poda en toda la
//!     instalación), `unreconcile_transfer` (irreversible por diseño) y `reconcile_transfers`.
//!   * **El bloque `impact`**: una escritura que mueve el motor cuenta su propia consecuencia.

mod common;

use common::{LoggedInOwner, TestApp};
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

/// Cuerpo de una llamada correcta, ya parseado.
fn ok_json(envelope: &serde_json::Value) -> serde_json::Value {
    assert_ne!(
        envelope["result"]["isError"], true,
        "se esperaba una llamada correcta: {envelope}"
    );
    let text = envelope["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("sin contenido de texto: {envelope}"));
    serde_json::from_str(text).unwrap_or_else(|_| panic!("cuerpo no JSON: {text}"))
}

/// Código estable de un tool-error.
fn error_code(envelope: &serde_json::Value) -> String {
    assert_eq!(
        envelope["result"]["isError"], true,
        "se esperaba un error: {envelope}"
    );
    let text = envelope["result"]["content"][0]["text"].as_str().unwrap_or("");
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|b| b["code"].as_str().map(str::to_string))
        .unwrap_or_default()
}

async fn create_token(app: &TestApp, cookie: &str) -> String {
    let created = app
        .post_json_with_cookie("/v1/api-tokens", json!({"label": "fase3"}), cookie)
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

#[derive(Debug, sqlx::FromRow)]
struct AuditRow {
    tool: String,
    outcome: String,
    error_code: Option<String>,
    target_ids: Vec<Uuid>,
    settled: bool,
}

async fn audit_rows(app: &TestApp) -> Vec<AuditRow> {
    sqlx::query_as::<_, AuditRow>(
        r#"SELECT tool, outcome, error_code, target_ids, (settled_at IS NOT NULL) AS settled
           FROM mcp_write_audit ORDER BY at, tool"#,
    )
    .fetch_all(&app.pool)
    .await
    .expect("leer mcp_write_audit")
}

/// Preview → token → confirmación. Devuelve `(preview, envelope de la confirmación)`.
async fn preview_then_confirm(
    app: &TestApp,
    bearer: &str,
    name: &str,
    args: serde_json::Value,
) -> (serde_json::Value, serde_json::Value) {
    let preview = ok_json(&mcp_post(app, bearer, tool_call(name, args.clone())).await);
    assert_eq!(preview["preview"], true, "{name}: {preview}");
    let ct = preview["confirm_token"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} debe emitir confirm_token: {preview}"))
        .to_string();
    let mut confirmed = args;
    confirmed["confirm"] = json!(true);
    confirmed["confirm_token"] = json!(ct);
    let envelope = mcp_post(app, bearer, tool_call(name, confirmed)).await;
    (preview, envelope)
}

/// Un pasivo con plan de pago mensual y dos movimientos vinculados.
async fn seed_liability(app: &TestApp, owner: &LoggedInOwner) -> String {
    let cat_lia = app.create_category(owner, "liability", "Préstamos").await;
    let cat_exp = app.create_category(owner, "expense", "Cuotas").await;
    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({"category_id": cat_lia, "expense_category_id": cat_exp, "label": "Hipoteca",
                   "principal": "120000", "payment_amount": "700", "payment_frequency": "monthly",
                   "payment_end_date": "2040-01-01"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");
    liab.json()["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// TAREA 1 — el ciclo completo de la auditoría
// ---------------------------------------------------------------------------

/// `attempted` → `ok`, con los ids REALMENTE mutados. Y las otras dos salidas del contrato:
/// `failed` con el código estable, y `ok` con la lista vacía cuando la llamada fue un preview.
#[tokio::test]
async fn el_ciclo_de_auditoria_se_cierra_con_su_desenlace_real() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;

    // 1. Escritura correcta → `ok` con el id de la fila creada.
    let created = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_transaction",
                json!({"op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
                       "kind": "expense", "category_id": cat}),
            ),
        )
        .await,
    );
    let txn_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();

    // 2. Escritura que falla en la core (categoría inexistente) → `failed` + código, sin targets.
    let bogus = "00000000-0000-4000-8000-000000000001";
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-06-11", "concept": "Otro", "amount": "-1.00",
                   "kind": "expense", "category_id": bogus}),
        ),
    )
    .await;
    assert_eq!(envelope["result"]["isError"], true, "{envelope}");

    // 3. Preview → `ok` SIN targets: fue bien y no tocó ninguna fila.
    let preview = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call("delete_transaction", json!({"id": txn_id.to_string()})),
        )
        .await,
    );
    assert_eq!(preview["preview"], true, "{preview}");

    let rows = audit_rows(&app).await;
    assert_eq!(rows.len(), 3, "tres pasos por el gate, tres filas: {rows:?}");
    for r in &rows {
        assert!(
            r.settled,
            "ninguna fila puede quedarse en `attempted`: si el llamante propaga el error antes \
             del settle, el log calla el desenlace de justo las llamadas que fallaron — {r:?}"
        );
        assert_ne!(r.outcome, "attempted", "{r:?}");
    }

    let create_ok = &rows[0];
    assert_eq!(create_ok.tool, "create_transaction");
    assert_eq!(create_ok.outcome, "ok");
    assert_eq!(create_ok.target_ids, vec![txn_id], "el id realmente creado");
    assert!(create_ok.error_code.is_none());

    let create_failed = &rows[1];
    assert_eq!(create_failed.outcome, "failed", "{create_failed:?}");
    assert!(
        create_failed.error_code.is_some(),
        "un `failed` sin código no dice nada: {create_failed:?}"
    );
    assert!(create_failed.target_ids.is_empty(), "no mutó nada");

    let previewed = &rows[2];
    assert_eq!(previewed.tool, "delete_transaction");
    assert_eq!(
        previewed.outcome, "ok",
        "un preview es una llamada correcta, no un error: {previewed:?}"
    );
    assert!(
        previewed.target_ids.is_empty(),
        "`ok` + sin targets = «fue bien y no tocó nada»: es lo que distingue un sondeo de un \
         borrado consumado — {previewed:?}"
    );
    // Y, efectivamente, el movimiento sigue vivo.
    assert_eq!(app.count_rows("transactions").await, 1);
}

/// El borrado consumado deja `ok` **con** el id: el log distingue el sondeo del hecho.
#[tokio::test]
async fn un_borrado_consumado_registra_el_id_que_borro() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;
    let created = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_transaction",
                json!({"op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
                       "kind": "expense", "category_id": cat}),
            ),
        )
        .await,
    );
    let txn_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_transaction",
            json!({"id": txn_id.to_string(), "confirm": true}),
        ),
    )
    .await;
    assert_eq!(ok_json(&envelope)["deleted"], true, "{envelope}");

    let rows = audit_rows(&app).await;
    let deleted = rows.iter().find(|r| r.tool == "delete_transaction").unwrap();
    assert_eq!(deleted.outcome, "ok");
    assert_eq!(deleted.target_ids, vec![txn_id]);
    assert_eq!(app.count_rows("transactions").await, 0);
}

// ---------------------------------------------------------------------------
// TAREA 2 — las tres destructivas que no pedían confirmación
// ---------------------------------------------------------------------------

/// `materialize_recurring` PODA instancias de toda la instalación y su firma no admitía ningún
/// parámetro: `confirm` era literalmente inexpresable, y su descripción delegaba en la buena
/// voluntad del modelo. Ahora exige confirmación en dos fases.
#[tokio::test]
async fn materialize_recurring_ya_no_poda_sin_barrera() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;

    // Sin confirm: preview, y NO ejecuta.
    let preview = ok_json(
        &mcp_post(&app, &token, tool_call("materialize_recurring", json!({}))).await,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["action"], "materialize_recurring", "{preview}");
    assert_eq!(
        preview["effects"]["entity"]["scope"], "installation",
        "el preview tiene que decir que el ámbito NO es el usuario del token: {preview}"
    );
    // El preview honesto de esta tool: declara lo que no puede saber en vez de estimarlo.
    assert!(
        preview["effects"]["side_effects"]["would_prune"].is_null()
            && preview["effects"]["side_effects"]["would_materialize"].is_null(),
        "{preview}"
    );
    assert!(
        preview["effects"]["side_effects"]["counts_unavailable_reason"].is_string(),
        "un `null` sin motivo se lee como «cero»: {preview}"
    );
    assert!(preview["confirm_token"].is_string(), "{preview}");

    // Con confirm pero SIN token: se rechaza.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("materialize_recurring", json!({"confirm": true})),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_required", "{envelope}");

    // Con las dos cosas: ejecuta.
    let (_, envelope) = preview_then_confirm(&app, &token, "materialize_recurring", json!({})).await;
    let out = ok_json(&envelope);
    assert_eq!(out["rules_processed"], 0, "{out}");
    assert_eq!(out["pruned"], 0, "{out}");
}

/// `reconcile_transfers` es un pase sobre la base entera y no tenía preview. Ahora lo tiene —
/// pero sin token: se deshace con `unreconcile_transfer`, así que exigir dos viajes sería
/// ceremonia sin daño que prevenir.
#[tokio::test]
async fn reconcile_transfers_previsualiza_pero_no_pide_token() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;

    let preview =
        ok_json(&mcp_post(&app, &token, tool_call("reconcile_transfers", json!({}))).await);
    assert_eq!(preview["preview"], true, "{preview}");
    assert!(
        preview["confirm_token"].is_null(),
        "reversible ⇒ sin token: {preview}"
    );
    assert_eq!(
        preview["effects"]["side_effects"]["reversible_with"], "unreconcile_transfer",
        "{preview}"
    );

    let out = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call("reconcile_transfers", json!({"confirm": true})),
        )
        .await,
    );
    assert_eq!(out["pairs_created"], 0, "{out}");
}

/// `unreconcile_transfer` es irreversible por diseño (persiste un rechazo anti-resurrección) y el
/// cliente sólo tiene el id de UNA pata: confirmar era confirmar a ciegas cuál era el par. El
/// preview enseña las dos.
#[tokio::test]
async fn unreconcile_ensena_las_dos_patas_antes_de_romper_el_par() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-06-10", "concept": "Salida", "amount": "-120",
                   "kind": "expense"}),
            &owner.cookie,
        )
        .await
        .json();
    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-06-11", "concept": "Entrada", "amount": "120",
                   "kind": "income"}),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(b["transfer_counterpart_id"], a["id"], "precondición: conciliadas");

    let preview = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call("unreconcile_transfer", json!({"transaction_id": a["id"]})),
        )
        .await,
    );
    assert_eq!(
        preview["effects"]["entity"]["transaction"]["id"], a["id"],
        "{preview}"
    );
    assert_eq!(
        preview["effects"]["entity"]["counterpart"]["id"], b["id"],
        "la pata que el cliente NO nombró es justo la que hay que enseñar: {preview}"
    );
    assert_eq!(
        preview["effects"]["side_effects"]["reversible_from_chat"], false,
        "{preview}"
    );

    // El preview no rompió nada.
    let still = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await
        .json();
    for t in still.as_array().unwrap() {
        assert!(t["transfer_counterpart_id"].is_string(), "{t}");
    }

    let (_, envelope) = preview_then_confirm(
        &app,
        &token,
        "unreconcile_transfer",
        json!({"transaction_id": a["id"]}),
    )
    .await;
    let out = ok_json(&envelope);
    assert!(out["transaction"]["transfer_counterpart_id"].is_null(), "{out}");
    assert!(out["counterpart"]["transfer_counterpart_id"].is_null(), "{out}");

    // Y la auditoría registra LAS DOS patas.
    let rows = audit_rows(&app).await;
    let done = rows
        .iter()
        .find(|r| r.tool == "unreconcile_transfer" && r.outcome == "ok" && !r.target_ids.is_empty())
        .expect("la confirmación deja su fila cerrada con ids");
    assert_eq!(done.target_ids.len(), 2, "{done:?}");
}

// ---------------------------------------------------------------------------
// TAREA 3 — el preview dejó de ser saltable
// ---------------------------------------------------------------------------

/// **El agujero que cierra la fase**: `confirm: true` en la PRIMERA llamada, sobre una fila jamás
/// previsualizada. Antes borraba; ahora se rechaza con un código que le dice al modelo qué hacer.
#[tokio::test]
async fn confirmar_a_ciegas_ya_no_borra_nada() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let liab_id = seed_liability(&app, &owner).await;

    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_liability", json!({"id": liab_id, "confirm": true})),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_required", "{envelope}");
    assert_eq!(app.count_rows("liabilities").await, 1, "no se borró nada");

    // Un token inventado tampoco vale.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_liability",
            json!({"id": liab_id, "confirm": true, "confirm_token": "ffpv_inventado"}),
        ),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_invalid", "{envelope}");
    assert_eq!(app.count_rows("liabilities").await, 1);
}

/// Un solo uso: el token muere al consumirse. Reintentar el mismo borrado con él no vale — ni
/// aunque el objetivo aún existiera.
#[tokio::test]
async fn el_token_del_preview_vale_exactamente_una_vez() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let mut ids = Vec::new();
    for name in ["Fondo A", "Fondo B"] {
        let r = app
            .post_json_with_cookie(
                "/v1/assets",
                json!({"category_id": cat_ast, "name": name, "current_value": "1000"}),
                &owner.cookie,
            )
            .await;
        ids.push(r.json()["id"].as_str().unwrap().to_string());
    }
    let (a1, a2) = (ids[0].clone(), ids[1].clone());

    let preview = ok_json(&mcp_post(&app, &token, tool_call("delete_asset", json!({"id": a1}))).await);
    let ct = preview["confirm_token"].as_str().unwrap().to_string();

    // Uso 1: borra.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_asset",
            json!({"id": a1, "confirm": true, "confirm_token": ct}),
        ),
    )
    .await;
    assert_eq!(ok_json(&envelope)["deleted"], true, "{envelope}");
    assert_eq!(app.count_rows("assets").await, 1);

    // Uso 2 del MISMO token, ahora contra el otro activo: rechazado, y el activo sigue vivo.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_asset",
            json!({"id": a2, "confirm": true, "confirm_token": ct}),
        ),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_invalid", "{envelope}");
    assert_eq!(app.count_rows("assets").await, 1, "el segundo activo sobrevive");
}

/// El token está ligado al OBJETIVO y a la TOOL, no sólo al usuario: uno emitido para el activo A
/// no confirma el borrado del activo B (aunque esté fresco y sin usar).
#[tokio::test]
async fn un_token_no_confirma_otro_objetivo_ni_otra_tool() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let a1 = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo A", "current_value": "1000"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let a2 = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo B", "current_value": "2000"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let preview = ok_json(&mcp_post(&app, &token, tool_call("delete_asset", json!({"id": a1}))).await);
    let ct = preview["confirm_token"].as_str().unwrap().to_string();

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_asset",
            json!({"id": a2, "confirm": true, "confirm_token": ct}),
        ),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_invalid", "{envelope}");
    assert_eq!(app.count_rows("assets").await, 2, "nada se borró");
}

/// **Lo que un `confirm` booleano no podía ni ver**: los efectos cambian ENTRE el preview y la
/// confirmación. El token va ligado a su huella, así que la confirmación se rechaza en vez de
/// destruir algo distinto de lo que se enseñó.
#[tokio::test]
async fn si_los_efectos_cambian_entre_el_preview_y_el_confirm_el_token_deja_de_valer() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let liab_id = seed_liability(&app, &owner).await;
    let cat_exp = app.create_category(&owner, "expense", "Otros").await;

    let preview = ok_json(
        &mcp_post(&app, &token, tool_call("delete_liability", json!({"id": liab_id}))).await,
    );
    assert_eq!(
        preview["effects"]["side_effects"]["transactions_unlinked"], 0,
        "{preview}"
    );
    let ct = preview["confirm_token"].as_str().unwrap().to_string();

    // El mundo se mueve: aparece un movimiento vinculado al pasivo. El borrado ya NO es el que
    // se enseñó (ahora desvincula una fila).
    let t = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({"op_date": "2026-06-10", "concept": "cuota", "amount": "-700.00",
                   "kind": "expense", "category_id": cat_exp, "linked_liability_id": liab_id}),
            &owner.cookie,
        )
        .await;
    assert_eq!(t.status, http::StatusCode::CREATED, "{t:?}");

    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_liability",
            json!({"id": liab_id, "confirm": true, "confirm_token": ct}),
        ),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_stale", "{envelope}");
    assert_eq!(app.count_rows("liabilities").await, 1, "no se borró");

    // Volver a previsualizar sí funciona, y ahora el preview dice la verdad nueva.
    let (preview2, envelope) =
        preview_then_confirm(&app, &token, "delete_liability", json!({"id": liab_id})).await;
    assert_eq!(
        preview2["effects"]["side_effects"]["transactions_unlinked"], 1,
        "{preview2}"
    );
    assert_eq!(ok_json(&envelope)["deleted"], true, "{envelope}");
}

/// El token es del usuario que previsualizó. El de otro miembro no confirma tu borrado — con
/// ámbito de instalación sería una vía para que una sesión ajena consumiera tu salvaguarda.
#[tokio::test]
async fn el_token_de_otro_miembro_no_confirma_lo_tuyo() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app
        .register_and_approve_member(&owner, "bob", "member")
        .await;
    let owner_token = create_token(&app, &owner.cookie).await;
    let member_token = create_token(&app, &member.cookie).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let asset_id = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_ast, "name": "Fondo", "current_value": "1000"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let preview = ok_json(
        &mcp_post(&app, &member_token, tool_call("delete_asset", json!({"id": asset_id}))).await,
    );
    let ct = preview["confirm_token"].as_str().unwrap().to_string();

    let envelope = mcp_post(
        &app,
        &owner_token,
        tool_call(
            "delete_asset",
            json!({"id": asset_id, "confirm": true, "confirm_token": ct}),
        ),
    )
    .await;
    assert_eq!(error_code(&envelope), "confirm_token_invalid", "{envelope}");
    assert_eq!(app.count_rows("assets").await, 1);
}

/// Los borrados de UNA fila cuyo contenido entero viaja en el preview siguen con `confirm` a
/// secas: la decisión de alcance del token es deliberada y esto la fija.
#[tokio::test]
async fn los_borrados_de_una_fila_no_pagan_el_segundo_viaje() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat_exp = app.create_category(&owner, "expense", "Viajes").await;

    let flow = ok_json(
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
    let flow_id = flow["id"].as_str().unwrap().to_string();

    let preview = ok_json(
        &mcp_post(&app, &token, tool_call("delete_planning_flow", json!({"id": flow_id}))).await,
    );
    assert!(
        preview["confirm_token"].is_null(),
        "una fila que cabe entera en el preview no necesita token: {preview}"
    );
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_planning_flow",
            json!({"id": flow_id, "confirm": true}),
        ),
    )
    .await;
    assert_eq!(ok_json(&envelope)["deleted"], true, "{envelope}");
    assert_eq!(app.count_rows("planning_flows").await, 0);
}

// ---------------------------------------------------------------------------
// TAREA 4 — `impact`: la escritura cuenta su propia consecuencia
// ---------------------------------------------------------------------------

fn num(v: &serde_json::Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un decimal como string, llegó {v:?}"))
        .parse()
        .expect("decimal")
}

/// Un `create_liability` movía cuatro cifras de `get_summary` y no mencionaba ninguna. Ahora las
/// cuenta él mismo: antes, después y delta, sin que el agente tenga que re-consultar nada.
#[tokio::test]
async fn crear_un_pasivo_cuenta_lo_que_le_hace_al_patrimonio() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let cat_lia = app.create_category(&owner, "liability", "Préstamos").await;
    let cat_exp = app.create_category(&owner, "expense", "Cuotas").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({"category_id": cat_ast, "name": "Fondo", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;

    let out = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_liability",
                json!({"label": "Coche", "category_id": cat_lia,
                       "expense_category_id": cat_exp, "principal": "5000"}),
            ),
        )
        .await,
    );
    let impact = &out["impact"];
    assert!(impact.is_object(), "sin bloque impact: {out}");
    assert_eq!(num(&impact["net_worth"]["before"]), 10000.0, "{impact}");
    assert_eq!(num(&impact["net_worth"]["after"]), 5000.0, "{impact}");
    assert_eq!(num(&impact["net_worth"]["delta"]), -5000.0, "{impact}");
    assert_eq!(
        num(&impact["debt_to_assets_ratio"]["after"]), 0.5,
        "el ratio deuda/activos es una de las cuatro: {impact}"
    );
    // Y NO trae la fecha de jubilación: eso es una simulación completa, y meterla aquí pondría
    // una proyección de hasta 840 meses en cada escritura.
    assert!(impact["jubilacion_month_index"].is_null(), "{impact}");
    assert!(impact["note"].is_string(), "{impact}");
}

/// El `impact` va donde la escritura mueve el motor SIEMPRE (invalidación FULL), no en las de
/// ámbito condicional: `create_transaction` es la escritura más frecuente del catálogo y sólo
/// mueve el promedio en los modos B/C, así que pagar dos `summary` por movimiento apuntado sería
/// caro justo donde más se llama.
#[tokio::test]
async fn las_escrituras_de_alta_frecuencia_no_pagan_el_impacto() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;

    let out = ok_json(
        &mcp_post(
            &app,
            &token,
            tool_call(
                "create_transaction",
                json!({"op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
                       "kind": "expense", "category_id": cat}),
            ),
        )
        .await,
    );
    assert!(out["impact"].is_null(), "{out}");
    assert!(out["id"].is_string(), "{out}");
}

// ---------------------------------------------------------------------------
// TAREA 5 — lo que las otras dos mitades construyeron, ya expuesto por MCP
// ---------------------------------------------------------------------------

/// La clave de idempotencia del alta manual, ahora alcanzable desde el chat: mismo cuerpo ⇒ el
/// movimiento original; cuerpo distinto ⇒ conflicto, gana el primero.
#[tokio::test]
async fn create_transaction_expone_la_clave_de_idempotencia() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;
    let body = json!({"op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
                      "kind": "expense", "category_id": cat, "idempotency_key": "k-1"});

    let first = ok_json(&mcp_post(&app, &token, tool_call("create_transaction", body.clone())).await);
    let second = ok_json(&mcp_post(&app, &token, tool_call("create_transaction", body)).await);
    assert_eq!(first["id"], second["id"], "el reintento devuelve el original");
    assert_eq!(app.count_rows("transactions").await, 1, "y no crea otro");

    // Misma clave, cuerpo distinto: conflicto explícito.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "create_transaction",
            json!({"op_date": "2026-06-10", "concept": "Otra cosa", "amount": "-99.00",
                   "kind": "expense", "category_id": cat, "idempotency_key": "k-1"}),
        ),
    )
    .await;
    assert_eq!(error_code(&envelope), "idempotency_key_conflict", "{envelope}");
    assert_eq!(app.count_rows("transactions").await, 1);
}

/// El preview de `delete_liability` contaba los movimientos que quedan sueltos y CALLABA la cuota
/// que desaparece del presupuesto — cientos de euros al mes en una hipoteca.
#[tokio::test]
async fn el_preview_del_pasivo_ensena_la_cuota_que_desaparece_del_presupuesto() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let liab_id = seed_liability(&app, &owner).await;

    let preview = ok_json(
        &mcp_post(&app, &token, tool_call("delete_liability", json!({"id": liab_id}))).await,
    );
    let removed = &preview["effects"]["side_effects"]["budget_entry_removed"];
    assert!(removed.is_object(), "el efecto que faltaba: {preview}");
    assert_eq!(removed["label"], "Hipoteca", "{preview}");
    assert_eq!(num(&removed["monthly_amount"]), 700.0, "{preview}");
    assert_eq!(
        num(&removed["expense_monthly_before"]) - num(&removed["expense_monthly_after"]),
        700.0,
        "el gasto presupuestado baja exactamente la cuota: {preview}"
    );
}

/// Confundir una cuota derivada con una partida de presupuesto ya no es un 404 sobre un id que el
/// propio servidor acaba de publicar: es un error tipado que dice a dónde ir. Y el PREVIEW ya lo
/// da — antes prometía un borrado que la confirmación iba a rechazar.
#[tokio::test]
async fn borrar_una_cuota_derivada_como_partida_remite_al_pasivo() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner.cookie).await;
    let liab_id = seed_liability(&app, &owner).await;

    for args in [
        json!({"id": liab_id}),
        json!({"id": liab_id, "confirm": true}),
    ] {
        let envelope = mcp_post(&app, &token, tool_call("delete_budget_entry", args)).await;
        assert_eq!(
            error_code(&envelope),
            "budget_entry_is_liability_derived",
            "{envelope}"
        );
    }
    assert_eq!(app.count_rows("liabilities").await, 1);
}
