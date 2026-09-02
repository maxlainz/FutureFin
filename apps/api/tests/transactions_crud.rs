//! Integración del CRUD de transacciones (`/v1/transactions`, `/months`, `/imports`).
//!
//! Cubre alta manual (individual + batch), savings sin categoría, mismatch de scope, PATCH con
//! campos inmutables en importadas, recompute de huella en manuales, `ON DELETE SET NULL` de los
//! links, remap de categoría (incluye transacciones), meses con datos, y borrado de lotes.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::TestApp;
use serde_json::{json, Value};

async fn create_manual(app: &TestApp, cookie: &str, body: Value) -> common::ResponseParts {
    app.post_json_with_cookie("/v1/transactions", body, cookie).await
}

/// Id de la categoría POR DEFECTO de un scope (4.15.0). Es la que el servidor pone cuando un alta
/// o un `clear_category` no nombran ninguna, y la que el preview del import sugiere cuando ninguna
/// regla casa — así que es también la que el confirm exige que la decisión traiga.
async fn fallback_category(app: &TestApp, cookie: &str, scope: &str) -> String {
    let cats = app
        .get_with_cookie(&format!("/v1/categories?scope={scope}"), cookie)
        .await
        .json();
    cats.as_array()
        .unwrap()
        .iter()
        .find(|c| c["is_fallback"] == json!(true))
        .unwrap_or_else(|| panic!("sin categoría por defecto en '{scope}'"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Importa una única transacción de gasto y devuelve su id.
async fn import_one_expense(app: &TestApp, cookie: &str) -> String {
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               15/06/2026;15/06/2026;TIENDA IMPORTADA;-9;EUR\n";
    let b64 = B64.encode(csv);
    let cat = fallback_category(app, cookie, "expense").await;
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            cookie,
        )
        .await;
    let pb = p.json();
    let sha = pb["file_sha256"].as_str().unwrap();
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({
                "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": [ { "kind": "expense", "category_id": cat } ], "learn_rules": false,
            }),
            cookie,
        )
        .await;
    assert_eq!(c.status, http::StatusCode::OK, "import: {c:?}");
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", cookie)
        .await;
    list.json()[0]["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Alta manual + listado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manual_create_and_list() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;

    let r = create_manual(
        &app,
        &owner.cookie,
        json!({
            "op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
            "kind": "expense", "category_id": cat, "notes": "efectivo"
        }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create: {r:?}");
    let b = r.json();
    assert_eq!(b["source"], "manual");
    assert!(b["import_id"].is_null(), "manual → import_id null");
    assert_eq!(b["kind"], "expense");
    assert_eq!(b["category_name"], "Ocio");
    assert_eq!(b["amount"].as_str().unwrap().parse::<f64>().unwrap(), -12.5);

    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await;
    assert_eq!(list.json().as_array().unwrap().len(), 1);
    // Otro mes → vacío.
    let empty = app
        .get_with_cookie("/v1/transactions?month=2026-05", &owner.cookie)
        .await;
    assert_eq!(empty.json().as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn batch_create() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "transactions": [
                { "op_date": "2026-06-01", "concept": "A", "amount": "-5", "kind": "expense" },
                { "op_date": "2026-06-02", "concept": "B", "amount": "1000", "kind": "income" },
                { "op_date": "2026-06-03", "concept": "C", "amount": "-200", "kind": "savings" },
            ] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "batch: {r:?}");
    assert_eq!(r.json().as_array().unwrap().len(), 3);
    assert_eq!(app.count_rows("transactions").await, 3);
}

// ---------------------------------------------------------------------------
// Validaciones de scope
// ---------------------------------------------------------------------------

#[tokio::test]
async fn savings_requires_null_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "X").await;
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Aporte", "amount": "-100",
                "kind": "savings", "category_id": cat }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("savings_no_category"));
}

#[tokio::test]
async fn expense_with_income_category_mismatch() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-100",
                "kind": "expense", "category_id": income_cat }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("category_scope_mismatch"));
}

#[tokio::test]
async fn zero_amount_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "0", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("amount_zero"));
}

// ---------------------------------------------------------------------------
// PATCH: importadas editables (huella anclada al CSV), manuales recomputan la huella
// ---------------------------------------------------------------------------

/// Total `expense_actual` (magnitud) del summary de un mes concreto.
async fn month_expense_actual(app: &TestApp, cookie: &str, year: i32, month: u32) -> f64 {
    let s = app
        .get_with_cookie(&format!("/v1/transactions/summary?year={year}&month={month}"), cookie)
        .await;
    s.json()["totals"]["expense_actual"].as_str().unwrap().parse::<f64>().unwrap()
}

#[tokio::test]
async fn patch_imported_fields_editable_fingerprint_anchored() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Importada: 15/06/2026, -9, "TIENDA IMPORTADA".
    let id = import_one_expense(&app, &owner.cookie).await;

    // Editar los 3 campos que antes eran inmutables: fecha (a otro mes), importe y concepto.
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "op_date": "2026-05-20", "amount": "-15", "concept": "TIENDA MOVIDA" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "patch importada: {patched:?}");
    let pb = patched.json();
    assert_eq!(pb["op_date"], "2026-05-20");
    assert_eq!(pb["concept"], "TIENDA MOVIDA");
    assert_eq!(pb["amount"].as_str().unwrap().parse::<f64>().unwrap(), -15.0);
    assert!(pb["import_id"].is_string(), "sigue siendo importada");

    // El summary re-agrega por la NUEVA fecha: mayo la incluye, junio ya no.
    assert_eq!(month_expense_actual(&app, &owner.cookie, 2026, 5).await, 15.0, "mayo incluye la movida");
    assert_eq!(month_expense_actual(&app, &owner.cookie, 2026, 6).await, 0.0, "junio ya no la tiene");

    // La huella queda anclada al CSV original → re-importar el MISMO archivo la omite por dedup
    // (pese a que la fila fue reubicada de fecha y editada en importe/concepto).
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               15/06/2026;15/06/2026;TIENDA IMPORTADA;-9;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            &owner.cookie,
        )
        .await;
    let pj = p.json();
    assert_eq!(pj["already_imported_count"].as_u64(), Some(1), "dedup detecta la huella anclada");
    let sha = pj["file_sha256"].as_str().unwrap().to_string();
    let cat = fallback_category(&app, &owner.cookie, "expense").await;
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({
                "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": [ { "kind": "expense", "category_id": cat } ], "learn_rules": false,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(c.status, http::StatusCode::OK, "re-import: {c:?}");
    let cb = c.json();
    assert!(cb["skipped_already_imported"].as_u64().unwrap() >= 1, "re-import omitido por dedup: {cb:?}");
    assert_eq!(cb["imported"].as_u64(), Some(0), "no se importa nada nuevo");
    // Sigue habiendo una única fila (no se duplicó).
    assert_eq!(app.count_rows("transactions").await, 1, "sin duplicar la fila anclada");
}

#[tokio::test]
async fn patch_manual_op_date_recomputes_and_allows_reuse() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Manual: 2026-06-10, "Café", -5.
    let created = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Café", "amount": "-5", "kind": "expense" }),
    )
    .await;
    let id = created.json()["id"].as_str().unwrap().to_string();

    // Cambiar la fecha → recomputa la huella (source=manual, op_date nueva) y libera la vieja.
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "op_date": "2026-06-11" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "patch op_date manual: {patched:?}");
    assert_eq!(patched.json()["op_date"], "2026-06-11");

    // La huella original (op_date 2026-06-10) quedó libre → crear un manual idéntico con la fecha
    // original no colisiona (201).
    let again = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Café", "amount": "-5", "kind": "expense" }),
    )
    .await;
    assert_eq!(again.status, http::StatusCode::CREATED, "reuse freed fingerprint: {again:?}");
}

#[tokio::test]
async fn patch_manual_amount_recomputes_and_allows_reuse() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Manual con importe -5.
    let created = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Café", "amount": "-5", "kind": "expense" }),
    )
    .await;
    let id = created.json()["id"].as_str().unwrap().to_string();

    // Cambiar el importe a -7 → recomputa huella.
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "amount": "-7" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "patch amount: {patched:?}");
    assert_eq!(patched.json()["amount"].as_str().unwrap().parse::<f64>().unwrap(), -7.0);

    // La huella original (-5) quedó libre → crear otra idéntica no colisiona.
    let again = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Café", "amount": "-5", "kind": "expense" }),
    )
    .await;
    assert_eq!(again.status, http::StatusCode::CREATED, "reuse freed fingerprint: {again:?}");
}

// ---------------------------------------------------------------------------
// Links ON DELETE SET NULL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn linked_asset_set_null_on_asset_delete() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let acat = app.create_category(&owner, "asset", "Cartera").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": acat, "name": "Fondo", "current_value": "1000" }),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();

    let txn = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Aporte", "amount": "-200",
                "kind": "savings", "linked_asset_id": asset_id }),
    )
    .await;
    assert_eq!(txn.status, http::StatusCode::CREATED, "{txn:?}");
    let txn_id = txn.json()["id"].as_str().unwrap().to_string();
    assert_eq!(txn.json()["linked_asset_id"], json!(asset_id));

    // Borrar el asset → el movimiento sobrevive con linked_asset_id NULL.
    let del = app.delete_with_cookie(&format!("/v1/assets/{asset_id}"), &owner.cookie).await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);

    let after = app
        .get_with_cookie(&format!("/v1/transactions?month=2026-06"), &owner.cookie)
        .await;
    let arr = after.json();
    let t = arr.as_array().unwrap().iter().find(|t| t["id"] == json!(txn_id)).unwrap().clone();
    assert!(t.get("linked_asset_id").is_none(), "linked_asset_id debe ser NULL tras borrar el asset");
    assert_eq!(app.count_rows("transactions").await, 1, "el movimiento sobrevive");
}

// ---------------------------------------------------------------------------
// Remap de categoría incluye transacciones
// ---------------------------------------------------------------------------

#[tokio::test]
async fn category_delete_remaps_transactions() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_a = app.create_category(&owner, "expense", "A").await;
    let cat_b = app.create_category(&owner, "expense", "B").await;

    let txn = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-10",
                "kind": "expense", "category_id": cat_a }),
    )
    .await;
    let txn_id = txn.json()["id"].as_str().unwrap().to_string();

    // Borrar A sin remap → 400 (en uso por la transacción, ON DELETE RESTRICT).
    let bad = app.delete_with_cookie(&format!("/v1/categories/{cat_a}"), &owner.cookie).await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST, "categoría en uso: {bad:?}");

    // Borrar A con remap a B → la transacción pasa a B.
    let ok = app
        .delete_with_cookie(&format!("/v1/categories/{cat_a}?remap_to={cat_b}"), &owner.cookie)
        .await;
    assert_eq!(ok.status, http::StatusCode::NO_CONTENT, "remap: {ok:?}");

    let list = app.get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie).await;
    let t = list.json();
    let found = t.as_array().unwrap().iter().find(|t| t["id"] == json!(txn_id)).unwrap().clone();
    assert_eq!(found["category_id"], json!(cat_b), "transacción remapeada a B");
}

// ---------------------------------------------------------------------------
// Meses, delete, cross-user, viewer, batches
// ---------------------------------------------------------------------------

#[tokio::test]
async fn months_endpoint_marks_complete() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    app.post_json_with_cookie(
        "/v1/transactions/batch",
        json!({ "transactions": [
            { "op_date": "2020-01-15", "concept": "Viejo", "amount": "-5", "kind": "expense" },
            { "op_date": "2020-02-15", "concept": "Viejo2", "amount": "-5", "kind": "expense" },
        ] }),
        &owner.cookie,
    )
    .await;
    let m = app.get_with_cookie("/v1/transactions/months", &owner.cookie).await;
    let arr = m.json();
    let months = arr.as_array().unwrap();
    // Tres, no dos: desde 4.4.0 el MES EN CURSO viaja siempre, aunque esté vacío. Antes salía de
    // un `GROUP BY` y un mes en curso sin movimientos simplemente no aparecía — la rama
    // `is_complete = false` no se materializaba nunca, mientras las series (`category-series`,
    // `history/cashflow`) sí le reservaban su hueco. La lista existe para orientar consultas y
    // contradecía a lo que se iba a consultar.
    assert_eq!(months.len(), 3, "mes en curso + los dos de 2020: {arr}");
    // Orden DESC, con el mes en curso el primero (es el más reciente: no hay fechas futuras).
    let hoy = chrono::Utc::now().format("%Y-%m").to_string();
    assert_eq!(months[0]["month"], serde_json::json!(hoy), "{arr}");
    assert_eq!(months[0]["is_complete"], false, "el mes en curso nunca es completo");
    assert_eq!(months[0]["txn_count"], 0, "y su cuenta a 0 es un dato real, no una ausencia");
    assert_eq!(months[1]["month"], "2020-02");
    assert_eq!(months[2]["month"], "2020-01");
    // Meses de 2020 son completos (no el mes en curso).
    assert_eq!(months[1]["is_complete"], true);
    assert_eq!(months[1]["txn_count"], 1);
}

#[tokio::test]
async fn delete_transaction_and_cross_user_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let created = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-10", "kind": "expense" }),
    )
    .await;
    let id = created.json()["id"].as_str().unwrap().to_string();

    // Bob no puede tocar el movimiento de Alice → 404.
    let bob_del = app.delete_with_cookie(&format!("/v1/transactions/{id}"), &bob.cookie).await;
    assert_eq!(bob_del.status, http::StatusCode::NOT_FOUND, "cross-user delete");

    let del = app.delete_with_cookie(&format!("/v1/transactions/{id}"), &owner.cookie).await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("transactions").await, 0);
}

#[tokio::test]
async fn viewer_cannot_create_403() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vic", "viewer").await;
    let r = create_manual(
        &app,
        &viewer.cookie,
        json!({ "op_date": "2026-06-10", "concept": "X", "amount": "-10", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_import_cascades_and_requires_confirm() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let _id = import_one_expense(&app, &owner.cookie).await;
    assert_eq!(app.count_rows("transactions").await, 1);
    assert_eq!(app.count_rows("transaction_imports").await, 1);

    let imports = app.get_with_cookie("/v1/transactions/imports", &owner.cookie).await;
    let batch = imports.json();
    let import_id = batch[0]["id"].as_str().unwrap().to_string();
    assert_eq!(batch[0]["txn_count"], 1);

    // Sin confirm → 400.
    let no_confirm = app
        .delete_with_cookie(&format!("/v1/transactions/imports/{import_id}"), &owner.cookie)
        .await;
    assert_eq!(no_confirm.status, http::StatusCode::BAD_REQUEST);
    assert!(no_confirm.json()["message"].as_str().unwrap().contains("confirm_required"));

    // Con confirm → 204 + cascade.
    let ok = app
        .delete_with_cookie(&format!("/v1/transactions/imports/{import_id}?confirm=true"), &owner.cookie)
        .await;
    assert_eq!(ok.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("transactions").await, 0, "cascade a transacciones");
    assert_eq!(app.count_rows("transaction_imports").await, 0);
}

// ---------------------------------------------------------------------------
// Conciliación (3.5.0): los listados NO filtran conciliadas
// ---------------------------------------------------------------------------

/// Un mes cuyo único contenido es un par conciliado sigue apareciendo en `/months` (con sus 2
/// filas) y en el listado — la exclusión es SOLO de agregados de flujo. Si `/months` filtrara,
/// el selector ocultaría un mes cuyas filas sí se listan → estado imposible en la UI.
#[tokio::test]
async fn list_months_counts_reconciled_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Par −80/+80 a 1 día → auto-conciliado en el alta de la segunda pata.
    let r1 = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-03-10", "concept": "Salida", "amount": "-80", "kind": "expense" }),
    )
    .await;
    assert_eq!(r1.status, http::StatusCode::CREATED);
    let r2 = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-03-11", "concept": "Entrada", "amount": "80", "kind": "income" }),
    )
    .await;
    assert!(r2.json()["transfer_counterpart_id"].is_string(), "precondición: conciliadas");

    let months = app.get_with_cookie("/v1/transactions/months", &owner.cookie).await.json();
    let march = months
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["month"] == "2026-03")
        .unwrap_or_else(|| panic!("mes solo-conciliadas ausente del selector: {months:?}"));
    assert_eq!(march["txn_count"].as_i64(), Some(2), "las dos patas cuentan en el selector");

    // Y el listado del mes las devuelve, con los campos de contrapartida presentes.
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-03", &owner.cookie)
        .await
        .json();
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2, "conciliadas visibles en el listado");
    for t in rows {
        assert!(t["transfer_counterpart_id"].is_string());
        assert_eq!(t["transfer_reconciled_source"], "auto");
        assert!(t["transfer_counterpart_concept"].is_string());
        assert!(t["transfer_counterpart_op_date"].is_string());
    }
}

// ---------------------------------------------------------------------------
// Filtros de búsqueda (3.8.0): concepto, importe con signo, rango de fechas
// ---------------------------------------------------------------------------

/// Siembra cinco movimientos con conceptos, importes y fechas diversos para ejercitar los filtros.
async fn seed_searchable(app: &TestApp, owner: &common::LoggedInOwner) {
    for (date, concept, amount, kind) in [
        ("2026-06-02", "Café   Módena", "-4.50", "expense"),
        ("2026-06-10", "WWW.AMAZON* MN34OP56", "-104.45", "expense"),
        ("2026-06-22", "AMAZON PRIME", "-8.99", "expense"),
        ("2026-07-05", "NOMINA JULIO", "2500", "income"),
        ("2026-07-19", "Descuento 50% dto_especial", "-20", "expense"),
    ] {
        let r = create_manual(
            app,
            &owner.cookie,
            json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind }),
        )
        .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{concept}: {r:?}");
    }
}

fn concepts(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| t["concept"].as_str().unwrap().to_string())
        .collect()
}

/// El filtro de concepto pliega tildes **en las dos direcciones** y es insensible a mayúsculas.
/// Es la misma semántica que el matching de reglas de categorización (`fold_diacritics_upper`),
/// replicada en SQL con `translate` — no con `upper()`, que depende de la collation del cluster.
#[tokio::test]
async fn concept_contains_is_accent_and_case_insensitive_both_ways() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_searchable(&app, &owner).await;

    // Sin tilde encuentra el concepto acentuado…
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=cafe", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()), vec!["Café   Módena"]);

    // …y con tilde encuentra lo mismo (la columna se pliega, no el patrón solo).
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=CAFÉ", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()), vec!["Café   Módena"]);

    // Búsqueda con acento distinto del almacenado: ambos pliegan a la misma letra.
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=modena", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()).len(), 1);

    // El colapso de espacios también se replica: el concepto se guarda con tres espacios.
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=cafe%20modena", &owner.cookie)
        .await;
    assert_eq!(
        concepts(&r.json()),
        vec!["Café   Módena"],
        "el espacio único del patrón debe casar con el run de espacios almacenado"
    );

    // Subcadena que aparece en dos filas, orden op_date DESC.
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=amazon", &owner.cookie)
        .await;
    assert_eq!(
        concepts(&r.json()),
        vec!["AMAZON PRIME", "WWW.AMAZON* MN34OP56"]
    );
}

/// `%` y `_` del usuario son texto literal. Sin escape, `%` devolvería el conjunto entero.
#[tokio::test]
async fn concept_contains_escapes_like_wildcards() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_searchable(&app, &owner).await;

    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=50%25%20dto", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()).len(), 1, "«50% dto» es literal: {:?}", r.json());

    // Un `%` suelto NO es «todo»: solo casa con las filas que contienen el carácter.
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=%25", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()).len(), 1, "un % literal, no un comodín: {:?}", r.json());

    // `_` tampoco es «cualquier carácter».
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=dto_especial", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()).len(), 1);
    let r = app
        .get_with_cookie("/v1/transactions?concept_contains=dtoXespecial", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()).len(), 0, "«_» no debe casar con cualquier carácter");
}

/// Los importes se comparan **con signo**: es la fuente de error más probable para un cliente.
#[tokio::test]
async fn amount_filters_compare_signed_values() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_searchable(&app, &owner).await;

    // Gastos de 50 € o más: amount <= -50.
    let r = app
        .get_with_cookie("/v1/transactions?max_amount=-50", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()), vec!["WWW.AMAZON* MN34OP56"]);

    // Solo entradas de dinero: amount >= 0.
    let r = app
        .get_with_cookie("/v1/transactions?min_amount=0", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()), vec!["NOMINA JULIO"]);

    // Banda cerrada.
    let r = app
        .get_with_cookie("/v1/transactions?min_amount=-25&max_amount=-5", &owner.cookie)
        .await;
    assert_eq!(concepts(&r.json()), vec!["Descuento 50% dto_especial", "AMAZON PRIME"]);

    // Banda invertida → 400 explícito, no un conjunto vacío silencioso.
    let r = app
        .get_with_cookie("/v1/transactions?min_amount=0&max_amount=-100", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
}

/// Rango de fechas **inclusivo** en los dos extremos, y excluyente con `month`.
#[tokio::test]
async fn date_range_is_inclusive_and_exclusive_with_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_searchable(&app, &owner).await;

    // El día exacto del extremo entra: 2026-06-22 es el `date_to`.
    let r = app
        .get_with_cookie(
            "/v1/transactions?date_from=2026-06-10&date_to=2026-06-22",
            &owner.cookie,
        )
        .await;
    assert_eq!(
        concepts(&r.json()),
        vec!["AMAZON PRIME", "WWW.AMAZON* MN34OP56"],
        "los dos extremos son inclusivos"
    );

    // Cruzar meses es justo lo que `month` no permite.
    let r = app
        .get_with_cookie(
            "/v1/transactions?date_from=2026-06-22&date_to=2026-07-05",
            &owner.cookie,
        )
        .await;
    assert_eq!(concepts(&r.json()), vec!["NOMINA JULIO", "AMAZON PRIME"]);

    // `month` + rango a la vez → 400: dos formas de decir lo mismo, sin ganador implícito.
    let r = app
        .get_with_cookie(
            "/v1/transactions?month=2026-06&date_from=2026-06-01",
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");

    // Rango invertido → 400.
    let r = app
        .get_with_cookie(
            "/v1/transactions?date_from=2026-07-01&date_to=2026-06-01",
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
}

/// Todos los ejes a la vez. Es el test que caza un cruce entre el orden de emisión de los
/// placeholders y el orden de los binds del macro `bind_filters!`: con un solo filtro cada query
/// parece correcta, y solo al combinarlos se ve que los valores se aplican a la columna equivocada.
#[tokio::test]
async fn all_filters_combined_agree_with_each_axis() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Compras").await;
    seed_searchable(&app, &owner).await;

    // Una fila más, categorizada, que cumple TODOS los criterios.
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-15", "concept": "AMAZON MARKETPLACE", "amount": "-60",
                "kind": "expense", "category_id": cat }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let url = format!(
        "/v1/transactions?concept_contains=amazon&kind=expense&category_id={cat}\
         &min_amount=-100&max_amount=-10&date_from=2026-06-01&date_to=2026-06-30"
    );
    let r = app.get_with_cookie(&url, &owner.cookie).await;
    assert_eq!(
        concepts(&r.json()),
        vec!["AMAZON MARKETPLACE"],
        "seis ejes simultáneos deben intersecar, no cruzarse: {:?}",
        r.json()
    );

    // Y cada eje por separado, para que el fallo del combinado no sea ambiguo.
    for (axis, expected_len) in [
        ("concept_contains=amazon", 3),
        ("kind=expense", 5),
        ("min_amount=-100&max_amount=-10", 2),
        ("date_from=2026-06-01&date_to=2026-06-30", 4),
    ] {
        let r = app
            .get_with_cookie(&format!("/v1/transactions?{axis}"), &owner.cookie)
            .await;
        assert_eq!(
            concepts(&r.json()).len(),
            expected_len,
            "eje «{axis}» solo: {:?}",
            r.json()
        );
    }
}

// ---------------------------------------------------------------------------
// Backfill de reglas de categorización (3.8.0)
// ---------------------------------------------------------------------------

async fn create_rule(app: &TestApp, cookie: &str, body: Value) -> common::ResponseParts {
    app.post_json_with_cookie("/v1/transactions/rules", body, cookie).await
}

/// El backfill usa la **precedencia completa**, no la regla suelta: una fila donde gana otra regla
/// no se toca y se reporta en `matched_by_other_rule`. Así el pasado queda como habría quedado
/// importando hoy, que es la única semántica reproducible.
#[tokio::test]
async fn apply_rule_uses_full_precedence_and_reports_losers() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let libros = app.create_category(&owner, "expense", "Libros").await;

    for (date, concept) in [
        ("2026-06-10", "WWW.AMAZON* MN34OP56"),
        ("2026-06-11", "AMAZON PRIME VIDEO"),
    ] {
        let r = create_manual(
            &app,
            &owner.cookie,
            json!({ "op_date": date, "concept": concept, "amount": "-20", "kind": "expense" }),
        )
        .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }

    // Regla ancha (substring "AMAZON") y regla más específica (substring "AMAZON PRIME").
    let ancha = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON", "assign_kind": "expense",
                "assign_category_id": compras }),
    )
    .await;
    assert_eq!(ancha.status, http::StatusCode::CREATED, "{ancha:?}");
    let ancha_id = ancha.json()["id"].as_str().unwrap().to_string();
    let especifica = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON PRIME", "assign_kind": "expense",
                "assign_category_id": libros }),
    )
    .await;
    assert_eq!(especifica.status, http::StatusCode::CREATED, "{especifica:?}");

    // Aplicar la ANCHA: solo debe tocar la fila donde gana; la otra la gana el patrón más largo.
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{ancha_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let out = r.json();
    assert_eq!(out["matched"], 1, "solo la fila donde gana la ancha: {out}");
    assert_eq!(
        out["matched_by_other_rule"], 1,
        "la fila de PRIME casa con la ancha pero la gana la específica: {out}"
    );

    let list = app
        .get_with_cookie("/v1/transactions?concept_contains=amazon", &owner.cookie)
        .await;
    let rows = list.json();
    let by_concept = |c: &str| -> Value {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|t| t["concept"].as_str().unwrap() == c)
            .unwrap()
            .clone()
    };
    assert_eq!(by_concept("WWW.AMAZON* MN34OP56")["category_id"], json!(compras));
    // La fila de la regla perdedora NO debe tocarse. Desde 4.15.0 «no tocada» ya no es «sin
    // categoría»: nació en la POR DEFECTO al darse de alta sin ninguna, y ahí sigue.
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    assert_eq!(
        by_concept("AMAZON PRIME VIDEO")["category_id"],
        json!(otros_gastos),
        "la fila de la regla perdedora NO debe tocarse"
    );
}

/// Una regla de un banco concreto no toca movimientos de otro origen — misma semántica que en el
/// import. Sin `skipped_by_source`, un `matched: 0` se leería como «no hay nada que hacer».
#[tokio::test]
async fn apply_rule_respects_source_and_reports_it() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Compras").await;

    // Movimiento MANUAL (source = "manual").
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "TIENDA X", "amount": "-20", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // Regla específica de MyInvestor: casa por texto, pero no por origen.
    let rule = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "TIENDA", "source": "myinvestor",
                "assign_kind": "expense", "assign_category_id": cat }),
    )
    .await;
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    let out = r.json();
    assert_eq!(out["matched"], 0, "{out}");
    assert_eq!(
        out["skipped_by_source"], 1,
        "el movimiento manual casa por texto pero la regla es de myinvestor: {out}"
    );
}

/// `uncategorized` respeta lo ya clasificado; `all` reasigna. Y el preview no escribe.
///
/// **4.15.0 vacía el scope `uncategorized` sin cambiar su SQL** (`t.category_id IS NULL`): ya
/// ningún ingreso ni gasto puede quedarse sin categoría, así que el alta sin categoría de este
/// test aterriza en la POR DEFECTO y el scope no la alcanza. No es un fallo del filtro: es que la
/// pregunta que respondía —«¿qué me falta por categorizar?»— dejó de tener respuestas. Lo que el
/// test sigue fijando es lo mismo de siempre: `uncategorized` NO pisa lo ya clasificado y `all` sí.
#[tokio::test]
async fn apply_rule_scopes_and_preview_does_not_write() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cajon = app.create_category(&owner, "expense", "Other").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;

    // Una fila sin categoría explícita (→ cae en la POR DEFECTO) y otra en la categoría cajón.
    let a = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "AMAZON UNO", "amount": "-20", "kind": "expense" }),
    )
    .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
    let b = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-11", "concept": "AMAZON DOS", "amount": "-30",
                "kind": "expense", "category_id": cajon }),
    )
    .await;
    assert_eq!(b.status, http::StatusCode::CREATED, "{b:?}");

    let rule = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON", "assign_kind": "expense",
                "assign_category_id": compras }),
    )
    .await;
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    // Sin confirm por HTTP → 400 explícito (la SPA ya enseña el impacto antes de llamar).
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");

    // `uncategorized`: cero filas — ninguna tiene `category_id IS NULL` desde 4.15.0.
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "uncategorized", "confirm": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.json()["matched"], 0, "{:?}", r.json());
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let rows = app
        .get_with_cookie("/v1/transactions?concept_contains=amazon", &owner.cookie)
        .await
        .json();
    let row_by = |c: &str| -> Value {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|t| t["concept"] == c)
            .unwrap()
            .clone()
    };
    assert_eq!(row_by("AMAZON DOS")["category_id"], json!(cajon), "«all» no se ha ejecutado todavía");
    assert_eq!(
        row_by("AMAZON UNO")["category_id"],
        json!(otros_gastos),
        "y la que no eligió categoría sigue en la por defecto, intacta"
    );

    // `all`: ahora sí reasigna la de la categoría cajón.
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    let out = r.json();
    assert_eq!(out["matched"], 2, "{out}");
    assert_eq!(out["by_current_category"][0]["category_name"], "Other", "{out}");
    let rows = app
        .get_with_cookie("/v1/transactions?concept_contains=amazon", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["category_id"], json!(compras), "todo en Compras: {t}");
    }

    // Idempotente: repetir no cambia nada y lo reporta como ya correcto.
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    let out = r.json();
    assert_eq!(out["matched"], 0, "{out}");
    assert_eq!(out["already_correct"], 2, "{out}");
}

/// Una regla de otro usuario es 404, no 403: no se filtra su existencia.
#[tokio::test]
async fn apply_rule_cross_user_is_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let cat = app.create_category(&owner, "expense", "Compras").await;

    let rule = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "AMAZON", "assign_kind": "expense",
                "assign_category_id": cat }),
    )
    .await;
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &bob.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NOT_FOUND, "{r:?}");
}

// ---------------------------------------------------------------------------
// PATCH /v1/transactions/batch — reclasificación en lote (3.8.0)
// ---------------------------------------------------------------------------

/// Todo o nada: un id ajeno en medio del lote deja CERO filas tocadas, y el 404 nombra al culpable.
/// Un resultado parcial obligaría al llamante a reconciliar estado, que es justo lo que un lote
/// viene a evitar.
#[tokio::test]
async fn batch_patch_is_all_or_nothing_and_names_the_culprit() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let cat = app.create_category(&owner, "expense", "Compras").await;

    let mut ids = Vec::new();
    for i in 0..5 {
        let r = create_manual(
            &app,
            &owner.cookie,
            json!({ "op_date": format!("2026-06-{:02}", 10 + i), "concept": format!("COMPRA {i}"),
                    "amount": "-10", "kind": "expense" }),
        )
        .await;
        ids.push(r.json()["id"].as_str().unwrap().to_string());
    }
    // Un movimiento de Bob: existe, pero no es de Alice.
    let ajeno = create_manual(
        &app,
        &bob.cookie,
        json!({ "op_date": "2026-06-20", "concept": "DE BOB", "amount": "-99", "kind": "expense" }),
    )
    .await;
    let ajeno_id = ajeno.json()["id"].as_str().unwrap().to_string();

    // El id ajeno va en MEDIO del lote: si la implementación escribiera y luego fallara, las
    // primeras filas ya estarían modificadas.
    let mut mixed = ids.clone();
    mixed.insert(2, ajeno_id.clone());
    let r = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": mixed, "category_id": cat }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NOT_FOUND, "{r:?}");
    let msg = r.json()["message"].as_str().unwrap().to_string();
    assert!(msg.contains(&ajeno_id), "el 404 debe nombrar el id culpable: {msg}");

    // CERO filas tocadas: siguen en la categoría POR DEFECTO con la que nacieron (4.15.0), que es
    // el estado previo al lote — antes esa comprobación era «siguen sin categoría».
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let rows = app
        .get_with_cookie("/v1/transactions?view=mine", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(
            t["category_id"],
            json!(otros_gastos),
            "ninguna fila debía tocarse tras el 404: {t}"
        );
    }

    // Y el lote correcto sí aplica, en una sola llamada.
    let r = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": ids, "category_id": cat, "notes": "revisado" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let out = r.json();
    assert_eq!(out["updated"], 5, "{out}");
    assert_eq!(out["resumen"].as_array().unwrap().len(), 5, "{out}");
    assert_eq!(out["resumen_truncated"], false, "{out}");
    let rows = app
        .get_with_cookie("/v1/transactions?view=mine", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["category_id"], json!(cat), "{t}");
        assert_eq!(t["notes"], "revisado", "{t}");
    }
}

/// El lote es equivalente a N PATCH individuales para los campos que admite, y rechaza los que no.
#[tokio::test]
async fn batch_patch_matches_individual_patches_and_rejects_rewrites() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Compras").await;

    // Positivas y `income`: el test las reclasifica a `expense`, que es el caso de una devolución
    // (llega en positivo, el signo la marca como ingreso, y pasarla a gasto la netea contra el mes).
    // Desde 4.0.0 el alta exige que el signo cuadre con el kind, así que un `income` negativo ya no
    // se puede crear; reclasificar sí sigue pudiendo dejar un `expense` positivo — y tiene que
    // poder, o el lote y el PATCH individual dejarían de ser equivalentes.
    let a = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "UNO", "amount": "10", "kind": "income" }),
    )
    .await;
    let b = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-11", "concept": "DOS", "amount": "20", "kind": "income" }),
    )
    .await;
    let a_id = a.json()["id"].as_str().unwrap().to_string();
    let b_id = b.json()["id"].as_str().unwrap().to_string();

    // A por la vía individual, B por el lote: el resultado debe ser indistinguible.
    let indiv = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{a_id}"),
            json!({ "kind": "expense", "category_id": cat }),
            &owner.cookie,
        )
        .await;
    assert_eq!(indiv.status, http::StatusCode::OK, "{indiv:?}");
    let lote = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": [b_id], "kind": "expense", "category_id": cat }),
            &owner.cookie,
        )
        .await;
    assert_eq!(lote.status, http::StatusCode::OK, "{lote:?}");

    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    for t in rows.as_array().unwrap() {
        assert_eq!(t["kind"], "expense", "{t}");
        assert_eq!(t["category_id"], json!(cat), "{t}");
    }

    // Campos de reescritura: no existen en el body del lote, así que serde los ignora y el lote
    // queda «sin nada que actualizar» → 400 explícito, nunca un cambio parcial silencioso.
    let r = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": [a_id], "amount": "-999", "op_date": "2020-01-01" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    let rows = app
        .get_with_cookie("/v1/transactions", &owner.cookie)
        .await
        .json();
    let a_row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(a_id))
        .unwrap();
    assert_eq!(a_row["amount"], "10.0000", "el importe no se toca por lote: {a_row}");
    assert_eq!(a_row["op_date"], "2026-06-10", "{a_row}");

    // Validaciones de exclusión mutua y de lote vacío.
    for body in [
        json!({ "ids": [a_id], "category_id": cat, "clear_category": true }),
        json!({ "ids": [a_id], "notes": "x", "clear_notes": true }),
        json!({ "ids": [], "kind": "expense" }),
    ] {
        let r = app
            .patch_json_with_cookie("/v1/transactions/batch", body.clone(), &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{body}: {r:?}");
    }
}

// ---------------------------------------------------------------------------
// REGRESIÓN — signo↔kind y fecha futura (auditoría MCP §3, auditoría MCP §11a; 4.0.0)
// ---------------------------------------------------------------------------

/// El alta manual exige que el signo cuadre con el `kind`.
///
/// Hasta 4.0.0 la invariante la aplicaba **un componente de React**
/// (`ManualCashEntryModal`), no el servidor: `{"amount":"23.50","kind":"expense"}` devolvía 201 y
/// dejaba un gasto positivo. Como el lado gasto se agrega como `-Σ`, ese mes publicaba un **gasto
/// total negativo**, y en los modos B/C entraba en el promedio real que alimenta la proyección:
/// la tasa de ahorro subía y la fecha FIRE se adelantaba, sin que nada lo señalara. Es el error
/// que comete un LLM al traducir «apunta 23,50 € de cena».
#[tokio::test]
async fn manual_create_rejects_amount_sign_that_contradicts_kind() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    for (amount, kind) in [("23.50", "expense"), ("100", "savings"), ("-50", "income")] {
        let r = create_manual(
            &app,
            &owner.cookie,
            json!({ "op_date": "2026-06-10", "concept": "X", "amount": amount, "kind": kind }),
        )
        .await;
        assert_eq!(
            r.status,
            http::StatusCode::BAD_REQUEST,
            "{amount} como {kind} debería rechazarse: {r:?}"
        );
        assert_eq!(r.json()["code"], "amount_sign_mismatch", "{amount}/{kind}");
    }

    // Los tres signos correctos siguen entrando.
    for (amount, kind) in [("-23.50", "expense"), ("-100", "savings"), ("50", "income")] {
        let r = create_manual(
            &app,
            &owner.cookie,
            json!({ "op_date": "2026-06-10", "concept": "OK", "amount": amount, "kind": kind }),
        )
        .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{amount}/{kind}: {r:?}");
    }
}

/// El PATCH valida el signo **cuando fija el importe**, y deja libre la reclasificación.
///
/// La distinción es lo que mantiene equivalentes el PATCH individual, el lote (que ni admite
/// `amount`) y el motor de reglas: los tres reclasifican, y ninguno puede prohibir el `expense`
/// positivo sin romper el neteo de una devolución.
#[tokio::test]
async fn patch_validates_sign_when_it_writes_the_amount_but_not_on_reclassification() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let id = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "CENA", "amount": "-23.50", "kind": "expense" }),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Fijar un importe que contradice el kind actual: rechazado.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "amount": "23.50" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "amount_sign_mismatch");

    // Cambiar importe y kind a la vez, coherentes: aceptado. Desde 4.15.0 el cambio de clase
    // arrastra la categoría —la que tenía es de otro scope y no puede quedarse—, así que el PATCH
    // la nombra: dejarla implícita sería `category_scope_mismatch`, y eso es deliberado (el
    // servidor no descarta en silencio una categoría que el usuario eligió).
    let otros_ingresos = fallback_category(&app, &owner.cookie, "income").await;
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "amount": "23.50", "kind": "income", "category_id": otros_ingresos }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    // Reclasificar sin tocar el importe: aceptado aunque quede incoherente. Es la devolución que
    // se pasa a gasto para netear contra el mes.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "kind": "expense", "category_id": otros_gastos }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "reclasificar debe seguir siendo libre: {r:?}");
    assert_eq!(r.json()["amount"], "23.5000", "el importe no se toca al reclasificar");

    // Y la variante SIN categoría es un 400 que nombra el problema, no un 200 que se come la
    // categoría vieja: cambiar de clase sin decir a qué categoría va no tiene respuesta correcta.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "kind": "income" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "category_scope_mismatch", "{}", r.json());
}

/// El importador NO aplica el guard de signo: trae el del banco, y una regla aprendida puede
/// asignarle un kind que lo contradiga. Si el import validara, un CSV con una devolución no se
/// podría confirmar — y el usuario no puede editar el CSV.
#[tokio::test]
async fn csv_import_still_accepts_the_sign_the_bank_sent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               15/06/2026;15/06/2026;DEVOLUCION TIENDA;9;EUR\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            &owner.cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    let cat = fallback_category(&app, &owner.cookie, "expense").await;
    // Importe positivo declarado como gasto: es un abono que netea contra el gasto del mes.
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({
                "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                "decisions": [ { "kind": "expense", "category_id": cat } ], "learn_rules": false,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(c.status, http::StatusCode::OK, "el import no debe validar el signo: {c:?}");
    assert_eq!(c.json()["imported"], 1, "{:?}", c.json());
}

/// Un movimiento con fecha futura no es un gasto: es un plan, y para eso está «Próximos».
///
/// Se llegó a registrar `op_date: "2099-12-31"` sin error, y `list_transaction_months` lo publicaba
/// como `{"month":"2099-12","is_complete":true}` — un mes a 73 años vista marcado como cerrado y
/// con datos.
#[tokio::test]
async fn manual_create_rejects_future_op_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2099-12-31", "concept": "FUTURO", "amount": "-10", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "op_date_in_future");

    // Hoy sí (el borde no se rechaza: el guard es `>`, no `>=`).
    let today = chrono::Utc::now().date_naive().format("%Y-%m-%d").to_string();
    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": today, "concept": "HOY", "amount": "-10", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "hoy debe entrar: {r:?}");

    // Y el PATCH tampoco deja mover una fila al futuro.
    let id = r.json()["id"].as_str().unwrap().to_string();
    let moved = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "op_date": "2099-12-31" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(moved.status, http::StatusCode::BAD_REQUEST, "{moved:?}");
    assert_eq!(moved.json()["code"], "op_date_in_future");
}

// ---------------------------------------------------------------------------
// PATCH individual: poner y borrar el mismo campo (Fase 1, issue #82)
// ---------------------------------------------------------------------------

/// `category_id` + `clear_category: true` en la MISMA llamada devolvía **200** y dejaba el
/// movimiento SIN categoría: el `clear` ganaba en silencio. Un agente que arma el patch desde una
/// plantilla creía recategorizar, los totales seguían cuadrando y la atribución mentía (y en los
/// modos B/C eso mueve el promedio que alimenta la proyección).
///
/// Se comprueban los CINCO `clear_*` del body, no solo los que tienen consecuencia contable: la
/// guardia existía ya en el camino de lote y en el de reglas, y el hueco era justo el PATCH
/// individual, que es el que comparten HTTP y la tool MCP `update_transaction`.
///
/// Los ids de asset/pasivo son UUID inventados a propósito: la guardia se evalúa ANTES que
/// `assert_asset_in_installation`, así que si alguna vez se moviera detrás, este test lo cazaría
/// (devolvería `linked_asset_not_found` en vez del código del conflicto).
#[tokio::test]
async fn patch_rejects_setting_and_clearing_the_same_field() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let libros = app.create_category(&owner, "expense", "Libros").await;

    let created = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "LIBRERIA", "amount": "-20",
                "kind": "expense", "category_id": compras }),
    )
    .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let ghost = "11111111-2222-3333-4444-555555555555";
    for (code, body) in [
        (
            "value_date_set_and_clear",
            json!({ "value_date": "2026-06-11", "clear_value_date": true }),
        ),
        (
            "category_set_and_clear",
            json!({ "category_id": libros, "clear_category": true }),
        ),
        (
            "linked_asset_set_and_clear",
            json!({ "linked_asset_id": ghost, "clear_linked_asset": true }),
        ),
        (
            "linked_liability_set_and_clear",
            json!({ "linked_liability_id": ghost, "clear_linked_liability": true }),
        ),
        (
            "notes_set_and_clear",
            json!({ "notes": "algo", "clear_notes": true }),
        ),
    ] {
        let r = app
            .patch_json_with_cookie(&format!("/v1/transactions/{id}"), body.clone(), &owner.cookie)
            .await;
        assert_eq!(
            r.status,
            http::StatusCode::BAD_REQUEST,
            "{code} debería rechazarse: {r:?}"
        );
        assert_eq!(r.json()["code"], code, "{}", r.json());
    }

    // Y NADA se ha escrito: la categoría sigue siendo la original.
    let row = app
        .get_with_cookie("/v1/transactions?concept_contains=LIBRERIA", &owner.cookie)
        .await
        .json();
    assert_eq!(row[0]["category_id"], json!(compras), "{row}");

    // El camino sano sigue funcionando: solo `clear_category` borra, solo `category_id` asigna.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "category_id": libros }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["category_id"], json!(libros), "{}", r.json());
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "clear_category": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    // 4.15.0: `clear_category` sobre un ingreso/gasto ya no deja el movimiento sin categoría —
    // lo devuelve a la POR DEFECTO de su scope. Es un cambio de significado sin cambio de forma,
    // y es el único posible: el estado que este `clear` producía dejó de ser representable.
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    assert_eq!(r.json()["category_id"], json!(otros_gastos), "{}", r.json());
}

/// El gemelo del anterior por el lado de la INVERSIÓN: ahí `clear_category` sigue significando
/// «sin categoría», porque los `savings` no llevan ninguna por diseño.
#[tokio::test]
async fn clear_category_on_savings_still_means_no_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "Aporte", "amount": "-100", "kind": "savings" }),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{id}"),
            json!({ "clear_category": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["category_id"], Value::Null, "{}", r.json());
}

// ---------------------------------------------------------------------------
// 4.15.0 — la categoría por defecto en las vías de escritura
// ---------------------------------------------------------------------------

/// Un alta sin categoría ya no crea un gasto «sin categoría»: lo crea en la POR DEFECTO de su
/// scope. El lote hace lo mismo, y cada fila cae en la de SU clase (la inversión, en ninguna).
///
/// Ese estado no es un detalle de presentación: mientras existió, el desglose por categoría de un
/// mes cuadraba en el total y mentía en la atribución, con la diferencia escondida en un hueco sin
/// nombre que nadie sumaba.
#[tokio::test]
async fn manual_create_without_category_lands_in_the_fallback() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let otros_ingresos = fallback_category(&app, &owner.cookie, "income").await;

    let r = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "SUELTO", "amount": "-12.50", "kind": "expense" }),
    )
    .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    assert_eq!(r.json()["category_id"], json!(otros_gastos), "{}", r.json());
    assert_eq!(r.json()["category_name"], "Otros gastos", "{}", r.json());

    let b = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "transactions": [
                { "op_date": "2026-06-01", "concept": "A", "amount": "-5", "kind": "expense" },
                { "op_date": "2026-06-02", "concept": "B", "amount": "1000", "kind": "income" },
                { "op_date": "2026-06-03", "concept": "C", "amount": "-200", "kind": "savings" },
            ] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(b.status, http::StatusCode::CREATED, "batch: {b:?}");
    let rows = b.json();
    assert_eq!(rows[0]["category_id"], json!(otros_gastos), "{rows}");
    assert_eq!(rows[1]["category_id"], json!(otros_ingresos), "{rows}");
    assert_eq!(rows[2]["category_id"], Value::Null, "la inversión no lleva categoría: {rows}");
}

/// El lote resuelve la categoría **por fila**, con el kind efectivo de cada una (`body.kind` si
/// viene, si no el de la fila). Un solo `clear_category` sobre un ingreso y un gasto tiene que
/// dejarlos en cajones DISTINTOS; resolver una vez para todo el lote los metería en el mismo.
#[tokio::test]
async fn batch_patch_resolves_the_fallback_per_row_kind() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let nomina = app.create_category(&owner, "income", "Nómina").await;
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let otros_ingresos = fallback_category(&app, &owner.cookie, "income").await;

    let gasto = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "GASTO", "amount": "-20",
                "kind": "expense", "category_id": compras }),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let ingreso = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-11", "concept": "INGRESO", "amount": "900",
                "kind": "income", "category_id": nomina }),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let r = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": [gasto, ingreso], "clear_category": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["updated"], 2, "{}", r.json());

    let rows = app.get_with_cookie("/v1/transactions", &owner.cookie).await.json();
    let cat_of = |c: &str| -> Value {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|t| t["concept"] == c)
            .unwrap()["category_id"]
            .clone()
    };
    assert_eq!(cat_of("GASTO"), json!(otros_gastos), "{rows}");
    assert_eq!(cat_of("INGRESO"), json!(otros_ingresos), "{rows}");
}

/// Una regla «solo kind» (sin `assign_category_id`) aplicada retroactivamente escribía `NULL` en
/// `category_id`. Desde 4.15.0 eso es un 23514 del CHECK —un 500 con cara de bug del motor de
/// reglas—, así que la regla resuelve la POR DEFECTO antes de su `UPDATE`.
#[tokio::test]
async fn apply_rule_with_kind_only_lands_rows_in_the_fallback() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;

    let t = create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "KIOSCO DE LA ESQUINA", "amount": "-3",
                "kind": "expense", "category_id": compras }),
    )
    .await;
    assert_eq!(t.status, http::StatusCode::CREATED, "{t:?}");

    let rule = create_rule(
        &app,
        &owner.cookie,
        json!({ "match_kind": "substring", "pattern": "KIOSCO", "assign_kind": "expense" }),
    )
    .await;
    assert_eq!(rule.status, http::StatusCode::CREATED, "{rule:?}");
    let rule_id = rule.json()["id"].as_str().unwrap().to_string();

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["matched"], 1, "{}", r.json());

    let rows = app
        .get_with_cookie("/v1/transactions?concept_contains=KIOSCO", &owner.cookie)
        .await
        .json();
    assert_eq!(rows[0]["category_id"], json!(otros_gastos), "{rows}");

    // Y es idempotente: la segunda pasada ve la fila YA en el destino resuelto, no «pendiente».
    let r2 = app
        .post_json_with_cookie(
            &format!("/v1/transactions/rules/{rule_id}/apply"),
            json!({ "apply_to_existing": "all", "confirm": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r2.json()["matched"], 0, "{}", r2.json());
    assert_eq!(r2.json()["already_correct"], 1, "{}", r2.json());
}

/// `?uncategorized=true` conserva su SQL (`category_id IS NULL`) y por eso cambia de conjunto: tras
/// 4.15.0 solo alcanza a las filas SIN CLASE —las que llegan restaurando un backup antiguo— y, si
/// se pide explícitamente, a la inversión. Documentarlo con un test evita que el próximo lector
/// crea que el filtro se rompió.
#[tokio::test]
async fn uncategorized_filter_now_returns_only_unclassified_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Un gasto y una inversión por la vía normal.
    create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-10", "concept": "GASTO", "amount": "-20", "kind": "expense" }),
    )
    .await;
    create_manual(
        &app,
        &owner.cookie,
        json!({ "op_date": "2026-06-11", "concept": "APORTE", "amount": "-100", "kind": "savings" }),
    )
    .await;
    // Y una fila SIN CLASE, que solo puede nacer de un restore: por SQL, como nacería.
    let iid = app.installation_id().await;
    sqlx::query(
        "INSERT INTO transactions (installation_id, owner_user_id, source, op_date, concept, \
         amount, currency, kind, category_id, fingerprint, fingerprint_ordinal) \
         VALUES ($1, $2, 'manual', DATE '2026-06-12', 'SIN CLASE', -7, 'EUR', NULL, NULL, 'fp-sc', 0)",
    )
    .bind(iid)
    .bind(owner.user_id)
    .execute(&app.pool)
    .await
    .expect("fila sin clase");

    let rows = app
        .get_with_cookie("/v1/transactions?uncategorized=true", &owner.cookie)
        .await
        .json();
    let conceptos: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["concept"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(conceptos, vec!["SIN CLASE".to_string()], "{rows}");

    // Pedir la inversión explícitamente sigue devolviéndola (la exclusión es un DEFAULT).
    let sav = app
        .get_with_cookie("/v1/transactions?uncategorized=true&kind=savings", &owner.cookie)
        .await
        .json();
    assert_eq!(sav.as_array().unwrap().len(), 1, "{sav}");
    assert_eq!(sav[0]["concept"], "APORTE", "{sav}");
}
