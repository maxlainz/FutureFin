//! Endurecimiento de las reglas de categorización (Fase 1 de la revisión adversarial, issue #82).
//!
//! Tres cosas, todas de la misma familia «el servidor contesta 200 a algo que no ha hecho»:
//!
//! 1. **Duplicados agnósticos de banco**: la descripción de `create_categorization_rule` promete
//!    «Duplicado (source, pattern) → resource conflict», y sin `source` no pasaba: la constraint
//!    UNIQUE no atrapa `NULL` porque en SQL `NULL <> NULL`. Dos llamadas idénticas creaban dos
//!    reglas y devolvían 200 las dos veces.
//! 2. **La migración que cierra ese agujero tiene que ser segura sobre una base que YA tenga
//!    duplicados**: si `CREATE UNIQUE INDEX` falla, el contenedor no arranca. Aquí se ejecuta el
//!    fichero de migración de verdad (`include_str!`) contra un estado sucio fabricado a mano.
//! 3. **El preview de `delete_categorization_rule` reventaba** con una regla sin `assign_kind`
//!    (alcanzable desde el propio catálogo con `clear_assign_kind`): borrarla a ciegas funcionaba
//!    y previsualizarla fallaba. El peor patrón posible.

mod common;

use common::TestApp;
use serde_json::{json, Value};

const PROTOCOL: &str = "2026-07-28";
const MIGRATION: &str =
    include_str!("../migrations/20260828120000_categorization_rules_unique_agnostic.sql");

async fn create_rule(app: &TestApp, cookie: &str, body: Value) -> common::ResponseParts {
    app.post_json_with_cookie("/v1/transactions/rules", body, cookie)
        .await
}

// ---------------------------------------------------------------------------
// 1. Duplicados
// ---------------------------------------------------------------------------

/// El caso reproducido en vivo: dos llamadas idénticas **sin `source`**. Un agente que reintenta
/// tras un timeout envenenaba la categorización de todos los imports futuros, y las reglas
/// contradictorias «ganan por precedencia, no por acierto».
#[tokio::test]
async fn creating_the_same_agnostic_rule_twice_is_a_conflict() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let libros = app.create_category(&owner, "expense", "Libros").await;

    let body = json!({ "match_kind": "substring", "pattern": "AMAZON",
                       "assign_kind": "expense", "assign_category_id": compras });
    let first = create_rule(&app, &owner.cookie, body.clone()).await;
    assert_eq!(first.status, http::StatusCode::CREATED, "{first:?}");

    let second = create_rule(&app, &owner.cookie, body).await;
    assert_eq!(second.status, http::StatusCode::CONFLICT, "{second:?}");
    assert_eq!(second.json()["code"], "rule_duplicate", "{}", second.json());

    // Y no basta con cambiar la asignación: el duplicado lo define `(source, pattern)`, que es lo
    // que decide el matching. Dos reglas con el mismo patrón y distinta categoría son justo las
    // «contradictorias» de las que avisa `update_categorization_rule`.
    let otra = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON",
                "assign_kind": "expense", "assign_category_id": libros }),
    )
    .await;
    assert_eq!(otra.status, http::StatusCode::CONFLICT, "{otra:?}");

    // Con `source` concreto sigue siendo 409 (la constraint ya lo cubría) y ahora con el mismo
    // código: el llamante no tiene que aprender dos contratos según haya rellenado un opcional.
    let con_source = json!({ "match_kind": "substring", "pattern": "AMAZON", "source": "myinvestor",
                             "assign_kind": "expense", "assign_category_id": compras });
    let a = create_rule(&app, &owner.cookie, con_source.clone()).await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
    let b = create_rule(&app, &owner.cookie, con_source).await;
    assert_eq!(b.status, http::StatusCode::CONFLICT, "{b:?}");
    assert_eq!(b.json()["code"], "rule_duplicate", "{}", b.json());

    // Otro patrón sigue siendo una regla nueva: la guardia no es un candado general.
    let nueva = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON PRIME",
                "assign_kind": "expense", "assign_category_id": libros }),
    )
    .await;
    assert_eq!(nueva.status, http::StatusCode::CREATED, "{nueva:?}");

    // Y el otro usuario del hogar no colisiona con las reglas de la primera (son per-user).
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let suya = create_rule(
        &app,
        &bob.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON",
                "assign_kind": "expense", "assign_category_id": compras }),
    )
    .await;
    assert_eq!(suya.status, http::StatusCode::CREATED, "{suya:?}");
}

/// El respaldo en carrera: aunque la comprobación de la aplicación se saltara (dos peticiones
/// simultáneas), el índice parcial rechaza el segundo INSERT.
#[tokio::test]
async fn the_partial_index_rejects_a_duplicate_agnostic_rule_at_the_database() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let iid = app.installation_id().await;

    let insert = r#"INSERT INTO categorization_rules
            (installation_id, owner_user_id, match_kind, pattern, source, assign_kind)
        VALUES ($1, $2, 'substring', 'CARREFOUR', NULL, 'expense')"#;
    sqlx::query(insert)
        .bind(iid)
        .bind(owner.user_id)
        .execute(&app.pool)
        .await
        .expect("la primera entra");
    let err = sqlx::query(insert)
        .bind(iid)
        .bind(owner.user_id)
        .execute(&app.pool)
        .await
        .expect_err("la segunda debe violar el índice único");
    let code = err
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.to_string()))
        .unwrap_or_default();
    assert_eq!(code, "23505", "esperaba unique_violation, salió {err}");
}

/// **La migración sobre una base sucia.** Es el escenario que puede dejar una instalación sin
/// arrancar: `CREATE UNIQUE INDEX` falla si ya hay duplicados, y una migración que falla bloquea
/// el contenedor entero. Aquí se fabrica el estado sucio (índice fuera, tres reglas agnósticas con
/// el mismo patrón) y se ejecuta el **fichero de migración real**.
///
/// Lo que se comprueba, además de que no explote:
/// - sobrevive **exactamente una** de las duplicadas, y es la de `updated_at` mayor: la que
///   `match_rule` elige hoy, así que ninguna categorización cambia;
/// - la regla que solo difiere en `match_kind` **NO se toca**: matchea otros conceptos, y borrarla
///   sí sería una pérdida de datos con consecuencia visible;
/// - las de `source` concreto tampoco (no entran en el índice parcial).
#[tokio::test]
async fn the_migration_deduplicates_before_creating_the_index() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let iid = app.installation_id().await;

    sqlx::query("DROP INDEX categorization_rules_unique_agnostic")
        .execute(&app.pool)
        .await
        .expect("el índice existe tras migrar");

    // Tres agnósticas idénticas en la clave, con `updated_at` creciente, más dos que deben
    // sobrevivir: la de `match_kind` distinto y la de `source` concreto.
    for (i, minutos) in [(1, 10), (2, 20), (3, 30)] {
        sqlx::query(
            r#"INSERT INTO categorization_rules
                   (installation_id, owner_user_id, match_kind, pattern, source, assign_kind,
                    updated_at)
               VALUES ($1, $2, 'substring', 'MERCADONA', NULL, 'expense', now() + ($3 || ' minutes')::interval)"#,
        )
        .bind(iid)
        .bind(owner.user_id)
        .bind(minutos.to_string())
        .execute(&app.pool)
        .await
        .unwrap_or_else(|e| panic!("insert duplicada {i}: {e}"));
    }
    for (match_kind, source) in [("exact", None), ("substring", Some("myinvestor"))] {
        sqlx::query(
            r#"INSERT INTO categorization_rules
                   (installation_id, owner_user_id, match_kind, pattern, source, assign_kind)
               VALUES ($1, $2, $3, 'MERCADONA', $4, 'expense')"#,
        )
        .bind(iid)
        .bind(owner.user_id)
        .bind(match_kind)
        .bind(source)
        .execute(&app.pool)
        .await
        .expect("insert superviviente");
    }

    // La migración de verdad, tal cual está en el repo.
    sqlx::raw_sql(MIGRATION)
        .execute(&app.pool)
        .await
        .expect("la migración debe sobrevivir a una base con duplicados");

    let filas: Vec<(String, Option<String>, i64)> = sqlx::query_as(
        r#"SELECT match_kind, source, EXTRACT(EPOCH FROM updated_at)::bigint
           FROM categorization_rules WHERE pattern = 'MERCADONA' ORDER BY match_kind, source"#,
    )
    .fetch_all(&app.pool)
    .await
    .expect("leer las supervivientes");
    assert_eq!(filas.len(), 3, "quedan 3 (1 deduplicada + 2 intactas): {filas:?}");

    let agnostica_substring: Vec<_> = filas
        .iter()
        .filter(|(mk, src, _)| mk == "substring" && src.is_none())
        .collect();
    assert_eq!(agnostica_substring.len(), 1, "{filas:?}");
    let max_epoch: i64 = sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM now() + interval '30 minutes')::bigint")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    // Sobrevive la más reciente (±2 s de holgura por el `now()` de cada sentencia).
    assert!(
        (agnostica_substring[0].2 - max_epoch).abs() <= 2,
        "debe sobrevivir la de updated_at mayor: {filas:?}"
    );
    assert!(
        filas.iter().any(|(mk, src, _)| mk == "exact" && src.is_none()),
        "la de match_kind distinto no se toca: {filas:?}"
    );
    assert!(
        filas.iter().any(|(_, src, _)| src.as_deref() == Some("myinvestor")),
        "la de source concreto no entra en el índice parcial: {filas:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Preview de una regla que no asigna nada
// ---------------------------------------------------------------------------

async fn mcp_post(app: &TestApp, bearer: &str, body: Value) -> Value {
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
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                last = Some(v);
            }
        }
    }
    last.unwrap_or_else(|| panic!("no JSON data frame in SSE response:\n{text}"))
}

fn tool_call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments, "_meta": {
            "io.modelcontextprotocol/protocolVersion": PROTOCOL,
            "io.modelcontextprotocol/clientCapabilities": {},
        }}
    })
}

/// Una regla sin `assign_kind` se alcanza desde el propio catálogo (`clear_assign_kind`). Con ella,
/// `delete_categorization_rule` **con `confirm: true` borraba** y **sin confirm fallaba** con
/// «rule_not_applicable: rule has no assign_kind to apply», un mensaje que en un preview de borrado
/// no quiere decir nada. Ahora el preview responde; aplicar la regla de verdad sigue siendo 400,
/// porque no hay nada que escribir.
#[tokio::test]
async fn previewing_the_deletion_of_a_rule_that_assigns_nothing_works() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let libros = app.create_category(&owner, "expense", "Libros").await;
    let token = app
        .post_json_with_cookie("/v1/api-tokens", json!({"label": "t"}), &owner.cookie)
        .await
        .json()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // Regla ancha que SÍ asigna, y regla más específica (patrón más largo ⇒ gana la precedencia)
    // que se queda sin `assign_kind`.
    let ancha = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON",
                "assign_kind": "expense", "assign_category_id": compras }),
    )
    .await;
    assert_eq!(ancha.status, http::StatusCode::CREATED, "{ancha:?}");
    let muda = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON PRIME",
                "assign_kind": "expense", "assign_category_id": libros }),
    )
    .await;
    let muda_id = muda.json()["id"].as_str().unwrap().to_string();
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/rules/{muda_id}"),
            json!({ "clear_assign_kind": true, "clear_assign_category": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["assign_kind"], Value::Null, "{}", r.json());

    let t = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-10", "concept": "AMAZON PRIME VIDEO",
                    "amount": "-20", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(t.status, http::StatusCode::CREATED, "{t:?}");

    // Preview (sin confirm) → ya no revienta.
    let envelope = mcp_post(
        &app,
        &token,
        tool_call("delete_categorization_rule", json!({ "id": muda_id })),
    )
    .await;
    assert_ne!(
        envelope["result"]["isError"], true,
        "el preview de una regla sin assign_kind no debe ser un error: {envelope}"
    );
    let body: Value =
        serde_json::from_str(envelope["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["preview"], true, "{body}");
    // Una regla que no asigna nada no cambia ni deja conforme a ningún movimiento: su huella de
    // CAMBIO es cero por definición, no por falta de trabajo.
    // Fase 2 (issue #83): la huella se publica bajo `side_effects` y con los MISMOS nombres que
    // el preview de `apply_categorization_rule` — comparten core, así que compartir vocabulario
    // deja de dar dos lecturas de los mismos números.
    assert_eq!(body["effects"]["side_effects"]["would_match"], 0, "{body}");
    assert_eq!(body["effects"]["side_effects"]["already_correct"], 0, "{body}");
    // La regla sigue ahí: el preview no borra.
    let quedan = app
        .get_with_cookie("/v1/transactions/rules", &owner.cookie)
        .await
        .json();
    assert_eq!(quedan.as_array().unwrap().len(), 2, "{quedan}");

    // Aplicarla de verdad sigue siendo 400: no hay nada que escribir.
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{muda_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "rule_not_applicable", "{}", r.json());

    // Y el confirm sí borra (era lo único que funcionaba antes; que siga funcionando).
    let envelope = mcp_post(
        &app,
        &token,
        tool_call(
            "delete_categorization_rule",
            json!({ "id": muda_id, "confirm": true }),
        ),
    )
    .await;
    assert_ne!(envelope["result"]["isError"], true, "{envelope}");
}

/// El caso normal no cambia de forma: `assigns_nothing` es `false`, `shadowed_transactions` es 0 y
/// no hay `note`. Sin esto, los tres campos nuevos podrían aparecer solo en el camino raro y nadie
/// se enteraría de que el contrato del caso común se movió.
#[tokio::test]
async fn a_normal_apply_outcome_keeps_its_shape() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let t = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-10", "concept": "AMAZON UNO",
                    "amount": "-20", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(t.status, http::StatusCode::CREATED, "{t:?}");
    let rule = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON",
                "assign_kind": "expense", "assign_category_id": compras }),
    )
    .await;
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    let out = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(out["matched"], 1, "{out}");
    assert_eq!(out["assigns_nothing"], false, "{out}");
    assert_eq!(out["shadowed_transactions"], 0, "{out}");
    assert_eq!(out["note"], Value::Null, "{out}");
}
