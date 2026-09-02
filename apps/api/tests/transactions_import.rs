//! Integración del import de CSV bancario (`/v1/transactions/import/*`).
//!
//! Cubre autodetección (MyInvestor/N26), decodificación Windows-1252, sugerencias de
//! kind/categoría/transferencia, dedup por huella + ordinales + force, reglas aprendidas,
//! viewer 403 y sha mismatch. Los CSV son fixtures SINTÉTICOS: reproducen el formato exacto
//! de cada banco, con datos inventados (ver la skill `futurefin-data-hygiene`).

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::{ResponseParts, TestApp};
use serde_json::{json, Value};

fn fixture_b64(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read fixture {name}: {e}"));
    B64.encode(bytes)
}

async fn preview(app: &TestApp, cookie: &str, source: &str, file_b64: &str) -> ResponseParts {
    app.post_json_with_cookie(
        "/v1/transactions/import/preview",
        json!({ "source": source, "file_b64": file_b64 }),
        cookie,
    )
    .await
}

/// Construye `decisions[]` paralelo a las filas del preview propagando kind/categoría sugeridos.
/// Desde 3.5.0 el default del wizard INCLUYE también las transferencias sugeridas (nada se
/// descarta en silencio; la exclusión del gasto pasa por la conciliación) — este helper refleja
/// ese default. El descarte explícito sigue existiendo (ver `learned_rules_precategorize`).
fn decisions_from_preview(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .map(|r| {
            json!({
                "kind": r["suggested_kind"],
                "category_id": r["suggested_category_id"],
            })
        })
        .collect()
}

async fn confirm(
    app: &TestApp,
    cookie: &str,
    source: &str,
    file_b64: &str,
    sha: &str,
    decisions: Vec<Value>,
    learn_rules: bool,
) -> ResponseParts {
    app.post_json_with_cookie(
        "/v1/transactions/import/confirm",
        json!({
            "source": source,
            "file_b64": file_b64,
            "file_sha256": sha,
            "decisions": decisions,
            "learn_rules": learn_rules,
        }),
        cookie,
    )
    .await
}

/// Id de la categoría POR DEFECTO de un scope (4.15.0): la que el preview sugiere cuando ninguna
/// regla casa, y la que el confirm exige que la decisión traiga.
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

fn row_by_concept<'a>(rows: &'a [Value], needle: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["concept"].as_str().unwrap_or("").contains(needle))
        .unwrap_or_else(|| panic!("no preview row containing '{needle}'"))
}

/// CSV MyInvestor de una sola fila (15/06/2026) con `concept`/`amount` a medida.
fn myinvestor_csv(concept: &str, amount: &str) -> String {
    format!(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
         15/06/2026;15/06/2026;{concept};{amount};EUR\n"
    )
}

// ---------------------------------------------------------------------------
// Autodetección + sugerencias (MyInvestor)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn myinvestor_autodetect_and_suggestions() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = fixture_b64("myinvestor_junio.csv");

    let resp = preview(&app, &owner.cookie, "auto", &b64).await;
    assert_eq!(resp.status, http::StatusCode::OK, "preview: {resp:?}");
    let body = resp.json();
    assert_eq!(body["source"], "myinvestor", "autodetección");
    let rows = body["rows"].as_array().unwrap();
    let n = rows.len() as u64;
    assert!(n > 20, "esperadas >20 filas, got {n}");
    // Primer import → todo nuevo.
    assert_eq!(body["new_count"].as_u64(), Some(n));
    assert_eq!(body["already_imported_count"].as_u64(), Some(0));

    // Nómina (positiva, sin regla) → income.
    assert_eq!(row_by_concept(rows, "Nomina Juny")["suggested_kind"], "income");
    // Cargo (negativo) → expense.
    assert_eq!(row_by_concept(rows, "VENDING")["suggested_kind"], "expense");
    // Aportación cartera → savings (heurística).
    assert_eq!(
        row_by_concept(rows, "Aportacion automatica cartera")["suggested_kind"],
        "savings"
    );
    // Transferencias internas → el HINT sigue marcándolas (informativo desde 3.5.0)…
    assert_eq!(
        row_by_concept(rows, "Transferencia desde MyInvestor")["suggested_transfer"],
        true
    );
    assert_eq!(row_by_concept(rows, "Enviada desde N26")["suggested_transfer"], true);
    assert_eq!(row_by_concept(rows, "estalvi")["suggested_transfer"], true);

    // …pero el confirm con el default nuevo las IMPORTA TODAS (nada se descarta en silencio;
    // la exclusión del gasto pasa por la conciliación).
    let decisions = decisions_from_preview(rows);
    let sha = body["file_sha256"].as_str().unwrap();
    let cresp = confirm(&app, &owner.cookie, "auto", &b64, sha, decisions, true).await;
    assert_eq!(cresp.status, http::StatusCode::OK, "confirm: {cresp:?}");
    let cbody = cresp.json();
    assert_eq!(cbody["imported"].as_u64(), Some(n), "todas las filas importadas");
    assert_eq!(cbody["discarded"].as_u64(), Some(0));
    assert!(cbody["reconciled_pairs"].is_u64(), "reconciled_pairs presente");
    assert!(cbody["import_id"].is_string(), "import_id presente");

    // Re-preview del mismo archivo → TODO ya importado (las transferencias también entraron).
    let re = preview(&app, &owner.cookie, "auto", &b64).await;
    let rbody = re.json();
    assert_eq!(
        rbody["already_imported_count"].as_u64(),
        Some(n),
        "todas las filas deben aparecer ya importadas"
    );
    // Re-confirmar sin forzar → 0 nuevos.
    let re_rows = rbody["rows"].as_array().unwrap();
    let re_decisions = decisions_from_preview(re_rows);
    let re_sha = rbody["file_sha256"].as_str().unwrap();
    let re_conf = confirm(&app, &owner.cookie, "auto", &b64, re_sha, re_decisions, false).await;
    assert_eq!(re_conf.json()["imported"].as_u64(), Some(0), "re-import: 0 nuevos");
}

// ---------------------------------------------------------------------------
// Dedup por huella: ordinales + force
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dedup_ordinals_and_force() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Dos filas idénticas (misma fecha/importe/concepto) → dos ocurrencias distintas.
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               10/06/2026;10/06/2026;CAFE DUPLICADO;-3;EUR\n\
               10/06/2026;10/06/2026;CAFE DUPLICADO;-3;EUR\n";
    let b64 = B64.encode(csv);

    let p1 = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let b1 = p1.json();
    // Sin nada en BD: ambas ocurrencias son "new".
    assert_eq!(b1["new_count"].as_u64(), Some(2));
    let sha = b1["file_sha256"].as_str().unwrap();
    let decisions = decisions_from_preview(b1["rows"].as_array().unwrap());
    let c1 = confirm(&app, &owner.cookie, "myinvestor", &b64, sha, decisions, false).await;
    assert_eq!(c1.json()["imported"].as_u64(), Some(2), "ambas ocurrencias importadas");

    // Re-preview: ahora existen 2 en BD → ambas ya importadas.
    let p2 = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let b2 = p2.json();
    assert_eq!(b2["already_imported_count"].as_u64(), Some(2));
    assert_eq!(b2["new_count"].as_u64(), Some(0));

    // Forzar la primera fila → nueva ocurrencia (ordinal siguiente).
    let sha2 = b2["file_sha256"].as_str().unwrap();
    let cat = fallback_category(&app, &owner.cookie, "expense").await;
    let decisions2 = vec![
        json!({ "force": true, "kind": "expense", "category_id": cat }),
        json!({ "discard": true, "kind": "expense", "category_id": cat }),
    ];
    let c2 = confirm(&app, &owner.cookie, "myinvestor", &b64, sha2, decisions2, false).await;
    let cb2 = c2.json();
    assert_eq!(cb2["imported"].as_u64(), Some(1), "forzada → 1 importada");
    assert_eq!(cb2["discarded"].as_u64(), Some(1));

    // Ahora hay 3 transacciones CAFE DUPLICADO.
    assert_eq!(app.count_rows("transactions").await, 3);
}

// ---------------------------------------------------------------------------
// Transferencias internas (N26): par opuesto + "Cuenta de Ahorro"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn n26_transfers_suggested() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = fixture_b64("n26_junio.csv");

    let resp = preview(&app, &owner.cookie, "auto", &b64).await;
    assert_eq!(resp.status, http::StatusCode::OK, "preview: {resp:?}");
    let body = resp.json();
    assert_eq!(body["source"], "n26");
    let rows = body["rows"].as_array().unwrap();

    // Par opuesto −26 / +26 el mismo día → ambos sugeridos transferencia.
    let cena = row_by_concept(rows, "Cena");
    assert_eq!(cena["suggested_transfer"], true, "−26 par opuesto");
    // "Cuenta de Ahorro" (partner) → transferencia.
    assert_eq!(
        row_by_concept(rows, "Cuenta de Ahorro")["suggested_transfer"],
        true
    );
    // Cross-bank +600 "Transferencia desde MyInvestor" → token de transferencia.
    assert_eq!(
        row_by_concept(rows, "Transferencia desde MyInvestor")["suggested_transfer"],
        true
    );
    assert!(body["suggested_transfer_count"].as_u64().unwrap() >= 5);

    // El importe `-26.000000000` se normaliza a 4 dp: la fila conserva -26.0000.
    let amt = cena["amount"].as_str().unwrap();
    assert_eq!(amt.parse::<f64>().unwrap(), -26.0);
}

// ---------------------------------------------------------------------------
// Reglas aprendidas
// ---------------------------------------------------------------------------

#[tokio::test]
async fn learned_rules_precategorize() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let super_cat = app.create_category(&owner, "expense", "Supermercado").await;
    let b64 = fixture_b64("n26_junio.csv");

    let p = preview(&app, &owner.cookie, "auto", &b64).await;
    let body = p.json();
    let rows = body["rows"].as_array().unwrap();

    // Categorizar las filas SUPERMERCADO como Supermercado; el resto según sugerencia (descartando transferencias).
    let decisions: Vec<Value> = rows
        .iter()
        .map(|r| {
            if r["concept"].as_str().unwrap_or("").contains("SUPERMERCADO") {
                json!({ "kind": "expense", "category_id": super_cat })
            } else {
                json!({
                    "discard": r["suggested_transfer"].as_bool().unwrap_or(false),
                    "kind": r["suggested_kind"],
                    "category_id": r["suggested_category_id"],
                })
            }
        })
        .collect();
    let sha = body["file_sha256"].as_str().unwrap();
    let c = confirm(&app, &owner.cookie, "auto", &b64, sha, decisions, true).await;
    assert_eq!(c.status, http::StatusCode::OK, "confirm: {c:?}");
    assert!(c.json()["rules_learned"].as_u64().unwrap() >= 1, "≥1 regla aprendida");

    // GET /rules → existe una regla que asigna Supermercado con patrón derivado de SUPERMERCADO.
    let rules = app.get_with_cookie("/v1/transactions/rules", &owner.cookie).await;
    let rbody = rules.json();
    let arr = rbody.as_array().unwrap();
    let found = arr.iter().any(|r| {
        r["assign_category_id"] == json!(super_cat)
            && r["pattern"].as_str().unwrap_or("").contains("SUPERMERCADO")
            && r["source"] == "n26"
    });
    assert!(found, "regla SUPERMERCADO→Supermercado no encontrada: {rbody:?}");

    // Re-preview → las filas SUPERMERCADO llegan pre-categorizadas por la regla.
    let re = preview(&app, &owner.cookie, "auto", &b64).await;
    let re_rows = re.json();
    let super_row = row_by_concept(re_rows["rows"].as_array().unwrap(), "SUPERMERCADO");
    assert_eq!(super_row["suggested_category_id"], json!(super_cat), "pre-categorizada");
    assert!(super_row["matched_rule_id"].is_string(), "matched_rule_id presente");
}

// ---------------------------------------------------------------------------
// Hint de savings insensible a acentos (fold de diacríticos)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn savings_hint_accent_insensitive_cartera() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Grafía real acentuada; antes solo se salvaba por el token "CARTERA".
    let b64 = B64.encode(myinvestor_csv("Aportación automática cartera", "-100"));
    let resp = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    assert_eq!(resp.status, http::StatusCode::OK, "preview: {resp:?}");
    let body = resp.json();
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows[0]["suggested_kind"], "savings", "acentuada real → savings");
}

#[tokio::test]
async fn savings_hint_accent_insensitive_no_cartera() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Sin "cartera": el fold de APORTACIÓN/INVERSIÓN es lo único que puede disparar el hint.
    let b64 = B64.encode(myinvestor_csv("Aportación periódica inversión", "-250"));
    let resp = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    assert_eq!(resp.status, http::StatusCode::OK, "preview: {resp:?}");
    let body = resp.json();
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows[0]["suggested_kind"], "savings", "acentuada sin 'cartera' → savings vía fold");
}

// ---------------------------------------------------------------------------
// Matching de reglas aprendidas insensible a acentos (fold en ambos lados)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn learned_rule_matches_accent_insensitive() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Fondos").await;
    // Regla con patrón ACENTUADO.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({ "match_kind": "substring", "pattern": "Aportación cartera",
                    "source": "myinvestor", "assign_kind": "expense", "assign_category_id": cat }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "rule: {r:?}");
    // Concepto SIN tilde en el CSV → debe matchear la regla acentuada.
    let b64 = B64.encode(myinvestor_csv("Aportacion cartera fondo indexado", "-100"));
    let p = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let body = p.json();
    let row = &body["rows"][0];
    assert!(
        row["matched_rule_id"].is_string(),
        "regla acentuada debe matchear concepto sin tilde: {row:?}"
    );
    // La regla aprendida (expense) gana al hint savings de «CARTERA/APORTACION».
    assert_eq!(row["suggested_kind"], "expense", "kind de la regla");
    assert_eq!(row["suggested_category_id"], json!(cat), "categoría de la regla");
}

#[tokio::test]
async fn learned_rule_matches_accent_insensitive_reverse() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Fondos").await;
    // Regla con patrón SIN tilde (dirección inversa: pattern sin acento vs concepto acentuado).
    let r = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({ "match_kind": "substring", "pattern": "Aportacion cartera",
                    "source": "myinvestor", "assign_kind": "expense", "assign_category_id": cat }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "rule: {r:?}");
    // Concepto CON tilde → debe matchear la regla sin tilde.
    let b64 = B64.encode(myinvestor_csv("Aportación cartera fondo", "-100"));
    let p = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let body = p.json();
    let row = &body["rows"][0];
    assert!(
        row["matched_rule_id"].is_string(),
        "regla sin tilde debe matchear concepto acentuado: {row:?}"
    );
    assert_eq!(row["suggested_kind"], "expense", "kind de la regla");
    assert_eq!(row["suggested_category_id"], json!(cat), "categoría de la regla");
}

#[tokio::test]
async fn confirm_savings_learns_kind_rule() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = B64.encode(myinvestor_csv("Aportación automática cartera", "-100"));
    let p = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let pb = p.json();
    assert_eq!(pb["rows"][0]["suggested_kind"], "savings", "sugerido savings");
    let sha = pb["file_sha256"].as_str().unwrap();

    // Confirmar una fila kind=savings (sin categoría) con learn_rules → aprende la regla.
    let c = confirm(
        &app,
        &owner.cookie,
        "myinvestor",
        &b64,
        sha,
        vec![json!({ "kind": "savings" })],
        true,
    )
    .await;
    assert_eq!(c.status, http::StatusCode::OK, "confirm: {c:?}");
    assert!(c.json()["rules_learned"].as_u64().unwrap() >= 1, "≥1 regla aprendida: {:?}", c.json());

    // La regla aprendida asigna kind savings (sin categoría).
    let rules = app.get_with_cookie("/v1/transactions/rules", &owner.cookie).await;
    let arr = rules.json();
    let savings_rule = arr
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["assign_kind"] == "savings")
        .unwrap_or_else(|| panic!("no savings rule learned: {arr:?}"));
    assert!(savings_rule["assign_category_id"].is_null(), "savings sin categoría");
}

#[tokio::test]
async fn learned_expense_rule_precedes_savings_hint() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Fondos").await;
    let b64 = B64.encode(myinvestor_csv("Aportación automática cartera", "-100"));

    // Primer preview: sin regla, el hint sugiere savings.
    let p1 = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let pb1 = p1.json();
    assert_eq!(pb1["rows"][0]["suggested_kind"], "savings", "primer preview: hint savings");
    let sha1 = pb1["file_sha256"].as_str().unwrap();

    // Confirmar como expense con categoría → aprende una regla expense.
    let c = confirm(
        &app,
        &owner.cookie,
        "myinvestor",
        &b64,
        sha1,
        vec![json!({ "kind": "expense", "category_id": cat })],
        true,
    )
    .await;
    assert_eq!(c.status, http::StatusCode::OK, "confirm expense: {c:?}");
    assert!(c.json()["rules_learned"].as_u64().unwrap() >= 1);

    // Segundo preview del mismo archivo: la regla aprendida (expense) gana al hint savings (by-design).
    let p2 = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let pb2 = p2.json();
    let row = &pb2["rows"][0];
    assert_eq!(row["suggested_kind"], "expense", "la regla aprendida tiene precedencia sobre el hint");
    assert_eq!(row["suggested_category_id"], json!(cat), "categoría de la regla");
    assert!(row["matched_rule_id"].is_string(), "matched_rule_id presente");
}

// ---------------------------------------------------------------------------
// Windows-1252
// ---------------------------------------------------------------------------

#[tokio::test]
async fn windows1252_decoding() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = fixture_b64("myinvestor_win1252.csv");

    let resp = preview(&app, &owner.cookie, "auto", &b64).await;
    assert_eq!(resp.status, http::StatusCode::OK, "win1252 preview: {resp:?}");
    let body = resp.json();
    assert_eq!(body["source"], "myinvestor");
    let rows = body["rows"].as_array().unwrap();
    // Los acentos y el € se decodifican correctamente.
    let cafe = row_by_concept(rows, "Café");
    assert!(cafe["concept"].as_str().unwrap().contains("€"), "€ decodificado");
    row_by_concept(rows, "Nómina");
    // Importe español -3,50 → -3.5.
    assert_eq!(cafe["amount"].as_str().unwrap().parse::<f64>().unwrap(), -3.5);
}

// ---------------------------------------------------------------------------
// Guardias: viewer 403, sha mismatch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_import_403() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vic", "viewer").await;
    let b64 = fixture_b64("myinvestor_junio.csv");

    let p = preview(&app, &viewer.cookie, "auto", &b64).await;
    assert_eq!(p.status, http::StatusCode::FORBIDDEN, "viewer preview 403");
    let c = confirm(&app, &viewer.cookie, "auto", &b64, "deadbeef", vec![], true).await;
    assert_eq!(c.status, http::StatusCode::FORBIDDEN, "viewer confirm 403");
}

#[tokio::test]
async fn confirm_sha_mismatch_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = fixture_b64("myinvestor_junio.csv");

    let p = preview(&app, &owner.cookie, "auto", &b64).await;
    let rows = p.json()["rows"].as_array().unwrap().clone();
    let decisions = decisions_from_preview(&rows);
    // sha que no corresponde al archivo.
    let c = confirm(&app, &owner.cookie, "auto", &b64, "0000", decisions, true).await;
    assert_eq!(c.status, http::StatusCode::BAD_REQUEST, "sha mismatch: {c:?}");
    assert!(c.json()["message"].as_str().unwrap().contains("preview_confirm_mismatch"));
}

#[tokio::test]
async fn confirm_decisions_count_mismatch_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = fixture_b64("myinvestor_junio.csv");
    let p = preview(&app, &owner.cookie, "auto", &b64).await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    // Solo 1 decisión para muchas filas.
    let c = confirm(
        &app,
        &owner.cookie,
        "auto",
        &b64,
        &sha,
        vec![json!({ "kind": "expense" })],
        true,
    )
    .await;
    assert_eq!(c.status, http::StatusCode::BAD_REQUEST);
    assert!(c.json()["message"].as_str().unwrap().contains("preview_confirm_mismatch"));
}

#[tokio::test]
async fn unrecognized_source_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // CSV que no matchea ningún preset.
    let b64 = B64.encode("col1,col2\nfoo,bar\n");
    let p = preview(&app, &owner.cookie, "auto", &b64).await;
    assert_eq!(p.status, http::StatusCode::BAD_REQUEST);
    assert!(p.json()["message"].as_str().unwrap().contains("csv_preset_unrecognized"));
}

// ---------------------------------------------------------------------------
// pending_assignments: reglas efímeras del preview (automatch en vivo, 4.14.0)
// ---------------------------------------------------------------------------

/// CSV MyInvestor sintético con tres comercios (dos del mismo patrón derivado).
fn cafe_csv_b64() -> String {
    B64.encode(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
         10/06/2026;10/06/2026;CAFE EJEMPLO 111;-3;EUR\n\
         11/06/2026;11/06/2026;CAFE EJEMPLO 222;-4;EUR\n\
         12/06/2026;12/06/2026;OTRO COMERCIO;-5;EUR\n",
    )
}

async fn preview_with_pending(
    app: &TestApp,
    cookie: &str,
    file_b64: &str,
    pending: Value,
) -> ResponseParts {
    app.post_json_with_cookie(
        "/v1/transactions/import/preview",
        json!({ "source": "myinvestor", "file_b64": file_b64, "pending_assignments": pending }),
        cookie,
    )
    .await
}

#[tokio::test]
async fn pending_assignment_propagates_to_similar_rows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Cafés").await;

    let b64 = cafe_csv_b64();
    let p = preview_with_pending(
        &app,
        &owner.cookie,
        &b64,
        json!([{ "concept": "CAFE EJEMPLO 111", "kind": "expense", "category_id": cat }]),
    )
    .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    let body = p.json();
    let rows = body["rows"].as_array().unwrap();

    // El patrón derivado «CAFE EJEMPLO» (sin el sufijo numérico) alcanza a AMBAS filas del
    // mismo comercio — exactamente lo que la regla aprendida hará en imports futuros.
    for needle in ["CAFE EJEMPLO 111", "CAFE EJEMPLO 222"] {
        let r = row_by_concept(rows, needle);
        assert_eq!(r["suggested_category_id"], json!(cat), "{needle}: {r}");
        assert_eq!(r["suggested_kind"], "expense", "{needle}: {r}");
        // La regla efímera no está persistida: no publica matched_rule_id.
        assert!(r["matched_rule_id"].is_null(), "{needle} sin rule id: {r}");
    }
    // No propaga a otros comercios: esa fila no la toca ninguna regla y por eso sale con la
    // categoría POR DEFECTO (4.15.0) y `suggested_category_source: "fallback"` — que es justo lo
    // que la distingue de una sugerencia de verdad.
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let other = row_by_concept(rows, "OTRO COMERCIO");
    assert_eq!(
        other["suggested_category_id"],
        json!(otros_gastos),
        "no propaga a otros: {other}"
    );
    assert_eq!(other["suggested_category_source"], "fallback", "{other}");
}

#[tokio::test]
async fn pending_assignment_wins_over_shorter_persisted_rule() {
    // Misma precedencia que tendrá tras persistirse: substring más largo gana. Si divergiera,
    // el preview enseñaría una propagación que el próximo import desharía.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_a = app.create_category(&owner, "expense", "Genérica").await;
    let cat_b = app.create_category(&owner, "expense", "Específica").await;

    let r = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({
                "pattern": "CAFE",
                "match_kind": "substring",
                "source": "myinvestor",
                "assign_kind": "expense",
                "assign_category_id": cat_a,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let b64 = cafe_csv_b64();
    // Sin pending: gana la persistida.
    let p0 = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let b0 = p0.json();
    let r0 = row_by_concept(b0["rows"].as_array().unwrap(), "CAFE EJEMPLO 222");
    assert_eq!(r0["suggested_category_id"], json!(cat_a), "{r0}");

    // Con pending sobre un concepto del comercio: «CAFE EJEMPLO» (más largo) la desbanca.
    let p1 = preview_with_pending(
        &app,
        &owner.cookie,
        &b64,
        json!([{ "concept": "CAFE EJEMPLO 111", "kind": "expense", "category_id": cat_b }]),
    )
    .await;
    let b1 = p1.json();
    let r1 = row_by_concept(b1["rows"].as_array().unwrap(), "CAFE EJEMPLO 222");
    assert_eq!(r1["suggested_category_id"], json!(cat_b), "{r1}");
}

#[tokio::test]
async fn pending_assignment_gate_mirrors_learn_rules() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Sin categoría y sin savings → no genera regla efímera (mismo gate que el aprendizaje).
    let b64 = cafe_csv_b64();
    let p = preview_with_pending(
        &app,
        &owner.cookie,
        &b64,
        json!([{ "concept": "CAFE EJEMPLO 111", "kind": "income" }]),
    )
    .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    let body = p.json();
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let r = row_by_concept(body["rows"].as_array().unwrap(), "CAFE EJEMPLO 222");
    assert_eq!(r["suggested_kind"], "expense", "default por signo intacto: {r}");
    assert_eq!(
        r["suggested_category_id"],
        json!(otros_gastos),
        "sin regla efímera, la fila cae en la por defecto: {r}"
    );
    assert_eq!(r["suggested_category_source"], "fallback", "{r}");

    // savings sin categoría SÍ propaga (los savings no llevan categoría por diseño).
    let hucha = B64.encode(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
         10/06/2026;10/06/2026;HUCHA MENSUAL 1;-30;EUR\n\
         11/06/2026;11/06/2026;HUCHA MENSUAL 2;-30;EUR\n",
    );
    let p2 = preview_with_pending(
        &app,
        &owner.cookie,
        &hucha,
        json!([{ "concept": "HUCHA MENSUAL 1", "kind": "savings" }]),
    )
    .await;
    let b2 = p2.json();
    let r2 = row_by_concept(b2["rows"].as_array().unwrap(), "HUCHA MENSUAL 2");
    assert_eq!(r2["suggested_kind"], "savings", "{r2}");
    assert!(
        r2["suggested_category_source"].is_null(),
        "la inversión no tiene categoría que atribuir: {r2}"
    );

    // 4.15.0: una asignación pendiente CON la categoría por defecto tampoco genera regla efímera.
    // Sin este gate, el propio wizard propagaría «Otros gastos» a todo el comercio dentro de la
    // sesión, y el confirm lo aprendería después como regla — que es la puerta gemela.
    let p3 = preview_with_pending(
        &app,
        &owner.cookie,
        &b64,
        json!([{ "concept": "CAFE EJEMPLO 111", "kind": "expense", "category_id": otros_gastos }]),
    )
    .await;
    assert_eq!(p3.status, http::StatusCode::OK, "{p3:?}");
    let b3 = p3.json();
    let r3 = row_by_concept(b3["rows"].as_array().unwrap(), "CAFE EJEMPLO 222");
    assert_eq!(
        r3["suggested_category_source"], "fallback",
        "la por defecto llega por el cajón, NO por una regla efímera propagada: {r3}"
    );
    assert!(r3["matched_rule_id"].is_null(), "{r3}");
}

#[tokio::test]
async fn pending_assignments_validation() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = cafe_csv_b64();

    // Kind inválido → 422-style 400 estricto, nunca coerción silenciosa.
    let p = preview_with_pending(
        &app,
        &owner.cookie,
        &b64,
        json!([{ "concept": "CAFE EJEMPLO 111", "kind": "foobar" }]),
    )
    .await;
    assert_eq!(p.status, http::StatusCode::BAD_REQUEST, "{p:?}");
    assert!(p.json()["message"].as_str().unwrap().contains("invalid_kind"));

    // Cota de sanidad: 201 entradas → 400.
    let many: Vec<Value> = (0..201)
        .map(|i| json!({ "concept": format!("X {i}"), "kind": "savings" }))
        .collect();
    let p2 = preview_with_pending(&app, &owner.cookie, &b64, json!(many)).await;
    assert_eq!(p2.status, http::StatusCode::BAD_REQUEST, "{p2:?}");
    assert!(p2.json()["message"].as_str().unwrap().contains("pending_assignments_too_many"));
}

#[tokio::test]
async fn empty_concept_never_learns_a_catch_all_rule() {
    // Una fila SIN concepto (venta de participaciones, p.ej.) confirmada como savings con
    // learn_rules activo NO puede envenenar los imports futuros: `clean_concept` la convierte
    // en «(sin concepto)» (regla específica, legítima) y el guard del confirm — el mismo de
    // las reglas efímeras del preview — descarta cualquier patrón derivado vacío, que como
    // substring matchearía TODOS los conceptos del banco.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let comercio = app.create_category(&owner, "expense", "Comercio").await;
    let csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
               10/06/2026;10/06/2026;;100;EUR\n\
               11/06/2026;11/06/2026;COMERCIO NORMAL;-5;EUR\n";
    let b64 = B64.encode(csv);
    let p = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();

    let c = confirm(
        &app,
        &owner.cookie,
        "myinvestor",
        &b64,
        &sha,
        vec![
            json!({ "kind": "savings", "category_id": null }),
            json!({ "kind": "expense", "category_id": comercio }),
        ],
        true,
    )
    .await;
    assert_eq!(c.status, http::StatusCode::OK, "{c:?}");

    // Ninguna regla con patrón vacío…
    let rules = app.get_with_cookie("/v1/transactions/rules", &owner.cookie).await.json();
    for r in rules.as_array().unwrap() {
        assert!(
            !r["pattern"].as_str().unwrap_or("").is_empty(),
            "regla con patrón vacío aprendida: {r}"
        );
    }

    // …y un preview posterior con un concepto cualquiera no sale contaminado a savings.
    let csv2 = B64.encode(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
         12/06/2026;12/06/2026;CONCEPTO CUALQUIERA;-7;EUR\n",
    );
    let p2 = preview(&app, &owner.cookie, "myinvestor", &csv2).await;
    let b2 = p2.json();
    let row = &b2["rows"][0];
    assert_eq!(row["suggested_kind"], "expense", "sin contaminación: {row}");
}

// ---------------------------------------------------------------------------
// 4.15.0 — la categoría por defecto en el wizard
// ---------------------------------------------------------------------------

/// El preview pre-rellena la categoría de toda fila de ingreso/gasto y **dice de dónde sale**.
/// Las dos categorías se pintan igual en el wizard y significan cosas distintas: una la eligió el
/// usuario alguna vez (`"rule"`), la otra es el cajón que el servidor pone porque no sabe
/// (`"fallback"`). Sin el campo, el wizard no puede evitar propagar el cajón por automatch ni
/// avisar de que no se aprenderá como regla.
#[tokio::test]
async fn preview_suggests_the_fallback_and_says_where_it_came_from() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cafes = app.create_category(&owner, "expense", "Cafés").await;
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;

    // Una regla que cubre SOLO uno de los dos conceptos del fichero.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions/rules",
            json!({ "pattern": "CAFE EJEMPLO", "match_kind": "substring", "source": "myinvestor",
                    "assign_kind": "expense", "assign_category_id": cafes }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let p = preview(&app, &owner.cookie, "myinvestor", &cafe_csv_b64()).await;
    let body = p.json();
    let rows = body["rows"].as_array().unwrap();

    let con_regla = row_by_concept(rows, "CAFE EJEMPLO 111");
    assert_eq!(con_regla["suggested_category_id"], json!(cafes), "{con_regla}");
    assert_eq!(con_regla["suggested_category_source"], "rule", "{con_regla}");

    let sin_regla = row_by_concept(rows, "OTRO COMERCIO");
    assert_eq!(sin_regla["suggested_category_id"], json!(otros_gastos), "{sin_regla}");
    assert_eq!(sin_regla["suggested_category_source"], "fallback", "{sin_regla}");
    assert_eq!(sin_regla["suggested_category_name"], "Otros gastos", "{sin_regla}");

    // El contador de precategorizadas sigue contando REGLAS, no cajones: es lo que el wizard
    // enseña como «ya clasificadas», y el cajón no clasifica nada.
    assert_eq!(body["precategorized_count"].as_u64(), Some(2), "{body}");
}

/// El confirm es la ÚNICA vía de escritura que no rellena el cajón en silencio: rechaza una
/// decisión de ingreso/gasto sin categoría con `category_required` y el índice de la fila.
///
/// Es estricto a propósito. En el wizard la categoría de cada fila se ve y el preview ya la trae
/// puesta, así que una decisión sin categoría no es una elección: es una que se perdió por el
/// camino. Aceptarla y taparla con el cajón enterraría el error en la atribución de un mes entero.
#[tokio::test]
async fn confirm_rejects_an_expense_decision_without_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let b64 = cafe_csv_b64();
    let p = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    let n = p.json()["rows"].as_array().unwrap().len();

    // Todas con categoría menos la segunda.
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;
    let mut decisions: Vec<Value> = (0..n)
        .map(|_| json!({ "kind": "expense", "category_id": otros_gastos }))
        .collect();
    decisions[1] = json!({ "kind": "expense" });

    let c = confirm(&app, &owner.cookie, "myinvestor", &b64, &sha, decisions, false).await;
    assert_eq!(c.status, http::StatusCode::BAD_REQUEST, "{c:?}");
    let body = c.json();
    assert_eq!(body["code"], "category_required", "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("row 1"),
        "el 400 debe nombrar la fila: {body}"
    );
    // Todo-o-nada: ni la fila 0 se ha escrito.
    assert_eq!(app.count_rows("transactions").await, 0);

    // La inversión sí puede ir sin categoría: no lleva ninguna por diseño.
    let hucha = B64.encode(&myinvestor_csv("HUCHA", "-30"));
    let p2 = preview(&app, &owner.cookie, "myinvestor", &hucha).await;
    let sha2 = p2.json()["file_sha256"].as_str().unwrap().to_string();
    let c2 = confirm(
        &app,
        &owner.cookie,
        "myinvestor",
        &hucha,
        &sha2,
        vec![json!({ "kind": "savings" })],
        false,
    )
    .await;
    assert_eq!(c2.status, http::StatusCode::OK, "{c2:?}");
    assert_eq!(c2.json()["imported"].as_u64(), Some(1), "{}", c2.json());
}

/// `learn_rules` no aprende NUNCA la categoría por defecto. Aprenderla escribiría una regla
/// «este concepto → Otros gastos» por cada concepto nuevo del extracto —cientos tras el primer
/// import— y esas reglas ganarían después la precedencia sobre las que el usuario sí quiso.
#[tokio::test]
async fn learn_rules_never_learns_the_fallback_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cafes = app.create_category(&owner, "expense", "Cafés").await;
    let otros_gastos = fallback_category(&app, &owner.cookie, "expense").await;

    let b64 = cafe_csv_b64();
    let p = preview(&app, &owner.cookie, "myinvestor", &b64).await;
    let body = p.json();
    let sha = body["file_sha256"].as_str().unwrap().to_string();
    let rows = body["rows"].as_array().unwrap().clone();

    // Una fila con categoría de verdad, el resto con el cajón.
    let decisions: Vec<Value> = rows
        .iter()
        .map(|r| {
            let elegida = r["concept"].as_str().unwrap().contains("CAFE EJEMPLO 111");
            json!({ "kind": "expense",
                    "category_id": if elegida { &cafes } else { &otros_gastos } })
        })
        .collect();

    let c = confirm(&app, &owner.cookie, "myinvestor", &b64, &sha, decisions, true).await;
    assert_eq!(c.status, http::StatusCode::OK, "{c:?}");
    assert_eq!(c.json()["rules_learned"].as_u64(), Some(1), "solo la elegida: {}", c.json());

    let reglas = app
        .get_with_cookie("/v1/transactions/rules", &owner.cookie)
        .await
        .json();
    let reglas = reglas.as_array().unwrap();
    assert_eq!(reglas.len(), 1, "{reglas:?}");
    assert_eq!(reglas[0]["assign_category_id"], json!(cafes), "{reglas:?}");
    for r in reglas {
        assert_ne!(
            r["assign_category_id"],
            json!(otros_gastos),
            "ninguna regla puede asignar la categoría por defecto: {r}"
        );
    }
}
