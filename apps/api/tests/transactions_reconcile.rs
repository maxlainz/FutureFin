//! Integración de la conciliación de transferencias (3.5.0):
//! `POST /v1/transactions/reconcile` (pase explícito), `POST/DELETE /v1/transactions/{id}/reconcile`
//! (par manual / desconciliar) y el pase automático post-commit de las mutaciones.
//!
//! Contrato bajo test: el pase empareja importes exactamente opuestos del MISMO owner y misma
//! divisa a ≤5 días, greedy determinista (gana la Δfecha menor), punto fijo (re-ejecutar → 0),
//! los pares desconciliados a mano NO resucitan (rechazo persistido), borrar/editar una pata
//! desconcilia la otra, y las patas conciliadas siguen visibles en el listado.

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::TestApp;
use serde_json::{json, Value};

/// Alta manual mínima; devuelve el body del 201 (que ya refleja el pase post-commit).
async fn create_txn(
    app: &TestApp,
    cookie: &str,
    op_date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
) -> Value {
    let resp = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op_date, "concept": concept, "amount": amount, "kind": kind }),
            cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "create_txn: {resp:?}");
    resp.json()
}

/// Recarga un movimiento por id desde el listado del mes.
async fn fetch_txn(app: &TestApp, cookie: &str, month: &str, id: &str) -> Value {
    let resp = app
        .get_with_cookie(&format!("/v1/transactions?month={month}"), cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "list: {resp:?}");
    resp.json()
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(id))
        .cloned()
        .unwrap_or_else(|| panic!("transaction {id} not in month {month}"))
}

fn id_of(t: &Value) -> String {
    t["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Pase automático
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_reconcile_pairs_opposite_amounts_within_window() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida traspaso", "-100", "expense").await;
    // El alta de la contrapartida dispara el pase → su 201 ya llega conciliado.
    let b = create_txn(&app, &owner.cookie, "2026-06-12", "Entrada traspaso", "100", "income").await;

    assert_eq!(b["transfer_counterpart_id"], a["id"], "B apunta a A");
    assert_eq!(b["transfer_reconciled_source"], "auto");
    assert_eq!(b["transfer_counterpart_concept"], "Salida traspaso");
    assert_eq!(b["transfer_counterpart_op_date"], "2026-06-10");
    assert!(b["transfer_reconciled_at"].is_string());

    // Simetría: A apunta a B.
    let a2 = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&a)).await;
    assert_eq!(a2["transfer_counterpart_id"], b["id"], "A apunta a B");
    assert_eq!(a2["transfer_reconciled_source"], "auto");
}

#[tokio::test]
async fn window_boundary_five_days_is_matched() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-50", "expense").await;
    // Δ = 5 días exactos → dentro de la ventana.
    let b = create_txn(&app, &owner.cookie, "2026-06-15", "Entrada", "50", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"], "borde exacto de 5 días empareja");
}

#[tokio::test]
async fn six_days_apart_is_not_matched() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-50", "expense").await;
    // Δ = 6 días → fuera.
    let b = create_txn(&app, &owner.cookie, "2026-06-16", "Entrada", "50", "income").await;
    assert!(b["transfer_counterpart_id"].is_null(), "6 días no empareja: {b:?}");
}

#[tokio::test]
async fn cross_import_pair_is_reconciled() {
    // EL CASO DEL BUG: pata de salida en un extracto, pata de entrada en OTRO, importados por
    // separado. El confirm del segundo debe cruzar TODA la BD del owner y conciliarlas.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let mi_csv = "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
                  15/06/2026;15/06/2026;Traspaso a N26;-600;EUR\n";
    let mi_b64 = B64.encode(mi_csv);
    let p1 = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": mi_b64 }),
            &owner.cookie,
        )
        .await;
    let p1b = p1.json();
    let c1 = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "myinvestor", "file_b64": mi_b64,
                    "file_sha256": p1b["file_sha256"], "decisions": [{ "kind": "expense" }],
                    "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    assert_eq!(c1.status, http::StatusCode::OK, "confirm 1: {c1:?}");
    assert_eq!(c1.json()["reconciled_pairs"].as_u64(), Some(0), "sin contrapartida aún");

    let n26_csv = "\"Booking Date\",\"Value Date\",\"Partner Name\",\"Partner Iban\",Type,\"Payment Reference\",\"Account Name\",\"Amount (EUR)\",\"Original Amount\",\"Original Currency\",\"Exchange Rate\"\n\
                   2026-06-16,2026-06-16,\"MyInvestor\",,\"Credit Transfer\",\"Entrada traspaso\",\"Cuenta principal\",600.00,,,\n";
    let n26_b64 = B64.encode(n26_csv);
    let p2 = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "n26", "file_b64": n26_b64 }),
            &owner.cookie,
        )
        .await;
    let p2b = p2.json();
    let c2 = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "n26", "file_b64": n26_b64,
                    "file_sha256": p2b["file_sha256"], "decisions": [{ "kind": "income" }],
                    "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    assert_eq!(c2.status, http::StatusCode::OK, "confirm 2: {c2:?}");
    assert_eq!(c2.json()["reconciled_pairs"].as_u64(), Some(1), "par cross-import conciliado");

    // Ambas patas enlazadas y visibles en el listado.
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await
        .json();
    let rows = list.as_array().unwrap().clone();
    assert_eq!(rows.len(), 2, "las dos patas siguen visibles");
    for t in &rows {
        assert!(t["transfer_counterpart_id"].is_string(), "pata conciliada: {t:?}");
        assert_eq!(t["transfer_reconciled_source"], "auto");
    }
}

#[tokio::test]
async fn reconcile_pass_is_idempotent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;
    create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "100", "income").await;

    // El par ya quedó enlazado por el pase post-alta → el pase explícito no encuentra nada.
    let r = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "reconcile: {r:?}");
    let rb = r.json();
    assert_eq!(rb["pairs_created"].as_u64(), Some(0), "punto fijo");
    assert_eq!(rb["transactions_reconciled"].as_u64(), Some(0));
}

#[tokio::test]
async fn greedy_picks_the_closest_date_deterministically() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Dos entradas +100 (no se emparejan entre sí: mismo signo) y una salida −100.
    let b1 = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada cercana", "100", "income").await;
    let b2 = create_txn(&app, &owner.cookie, "2026-06-14", "Entrada lejana", "100", "income").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;

    // Gana la Δfecha menor: A↔B1 (1 día), no A↔B2 (4 días).
    assert_eq!(a["transfer_counterpart_id"], b1["id"], "greedy elige la más cercana");
    let b2_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&b2)).await;
    assert!(b2_now["transfer_counterpart_id"].is_null(), "la lejana queda suelta");
}

#[tokio::test]
async fn different_owner_never_matched() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    create_txn(&app, &owner.cookie, "2026-06-10", "Salida alice", "-100", "expense").await;
    let b = create_txn(&app, &bob.cookie, "2026-06-10", "Entrada bob", "100", "income").await;
    assert!(
        b["transfer_counterpart_id"].is_null(),
        "owners distintos jamás se emparejan: {b:?}"
    );
}

#[tokio::test]
async fn savings_leg_participates_in_matching() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Una aportación (savings −200) y su contrapartida +200 en el bróker: sin conciliar, el
    // income quedaría inflado en 200 → savings PARTICIPA en el matching.
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Aportación", "-200", "savings").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada bróker", "200", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"], "savings participa");
}

// ---------------------------------------------------------------------------
// Desconciliar / rechazos / roturas de par
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unreconcile_persists_rejection_and_survives_reruns() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Gasto real", "-75", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Reembolso ajeno", "75", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"], "falso positivo enlazado");

    // Desconciliar (p. ej. un reembolso que casualmente cuadra) → ambas sueltas.
    let u = app
        .delete_with_cookie(&format!("/v1/transactions/{}/reconcile", id_of(&a)), &owner.cookie)
        .await;
    assert_eq!(u.status, http::StatusCode::OK, "unreconcile: {u:?}");
    let ub = u.json();
    assert!(ub["transaction"]["transfer_counterpart_id"].is_null());
    assert!(ub["counterpart"]["transfer_counterpart_id"].is_null());

    // El pase explícito NO resucita el par (rechazo persistido). Punto fijo intacto.
    let r = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(r.json()["pairs_created"].as_u64(), Some(0), "el rechazo bloquea el re-emparejado");
    let a_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&a)).await;
    assert!(a_now["transfer_counterpart_id"].is_null(), "sigue suelta tras el pase");
}

#[tokio::test]
async fn deleting_one_leg_unreconciles_the_other() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "100", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"]);

    let d = app
        .delete_with_cookie(&format!("/v1/transactions/{}", id_of(&b)), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "delete: {d:?}");

    // La superviviente queda suelta (FK ON DELETE SET NULL) y sin metadata fantasma.
    let a_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&a)).await;
    assert!(a_now["transfer_counterpart_id"].is_null(), "superviviente desconciliada");
    assert!(a_now["transfer_reconciled_at"].is_null(), "reconciled_at no se serializa suelta");
    assert!(a_now["transfer_reconciled_source"].is_null());
}

#[tokio::test]
async fn patch_amount_breaks_the_pair_and_reverting_relinks() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "100", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"]);

    // Cambiar el importe rompe el par (ya no suma cero) en AMBAS patas…
    let p = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{}", id_of(&a)),
            json!({ "amount": "-90" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "patch: {p:?}");
    assert!(p.json()["transfer_counterpart_id"].is_null(), "par roto tras el PATCH");
    let b_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&b)).await;
    assert!(b_now["transfer_counterpart_id"].is_null(), "la otra pata también");

    // …pero SIN rechazo: volver al importe original re-empareja en el pase del propio PATCH.
    let p2 = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{}", id_of(&a)),
            json!({ "amount": "-100" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p2.json()["transfer_counterpart_id"], b["id"], "revertir re-empareja");
}

#[tokio::test]
async fn patch_op_date_out_of_window_breaks_the_pair() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "100", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"]);

    let p = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{}", id_of(&a)),
            json!({ "op_date": "2026-06-30" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "patch: {p:?}");
    assert!(p.json()["transfer_counterpart_id"].is_null(), "fuera de ventana: par roto");
    let b_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&b)).await;
    assert!(b_now["transfer_counterpart_id"].is_null());
}

#[tokio::test]
async fn delete_import_unreconciles_surviving_counterparts() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Pata manual + pata importada conciliadas; deshacer el import debe soltar la manual.
    let a = create_txn(&app, &owner.cookie, "2026-06-15", "Salida manual", "-600", "expense").await;
    let n26_csv = "\"Booking Date\",\"Value Date\",\"Partner Name\",\"Partner Iban\",Type,\"Payment Reference\",\"Account Name\",\"Amount (EUR)\",\"Original Amount\",\"Original Currency\",\"Exchange Rate\"\n\
                   2026-06-16,2026-06-16,\"MyInvestor\",,\"Credit Transfer\",\"Entrada traspaso\",\"Cuenta principal\",600.00,,,\n";
    let b64 = B64.encode(n26_csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "n26", "file_b64": b64 }),
            &owner.cookie,
        )
        .await;
    let pb = p.json();
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "n26", "file_b64": b64, "file_sha256": pb["file_sha256"],
                    "decisions": [{ "kind": "income" }], "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    assert_eq!(c.json()["reconciled_pairs"].as_u64(), Some(1), "conciliadas al importar");
    let import_id = c.json()["import_id"].as_str().unwrap().to_string();

    let d = app
        .delete_with_cookie(
            &format!("/v1/transactions/imports/{import_id}?confirm=true"),
            &owner.cookie,
        )
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "delete import: {d:?}");
    let a_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&a)).await;
    assert!(a_now["transfer_counterpart_id"].is_null(), "superviviente suelta tras deshacer el lote");
}

// ---------------------------------------------------------------------------
// Conciliación manual
// ---------------------------------------------------------------------------

#[tokio::test]
async fn manual_reconcile_outside_window_succeeds() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // 24 días de distancia: el pase automático no las toca, la manual SÍ (sin ventana).
    let a = create_txn(&app, &owner.cookie, "2026-06-01", "Salida lenta", "-300", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-25", "Entrada lenta", "300", "income").await;
    assert!(b["transfer_counterpart_id"].is_null(), "auto no empareja a 24 días");

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&a)),
            json!({ "counterpart_id": id_of(&b) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "manual: {r:?}");
    let rb = r.json();
    assert_eq!(rb["transaction"]["transfer_counterpart_id"], b["id"]);
    assert_eq!(rb["counterpart"]["transfer_counterpart_id"], a["id"]);
    assert_eq!(rb["transaction"]["transfer_reconciled_source"], "manual");
}

#[tokio::test]
async fn manual_reconcile_validations_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-01", "Salida", "-300", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-25", "Entrada", "290", "income").await;

    // Importes que no se cancelan → 400 (conciliar jamás altera el neto del hogar).
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&a)),
            json!({ "counterpart_id": id_of(&b) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST);
    assert!(r.json()["message"].as_str().unwrap().contains("transfer_amounts_not_opposite"));

    // Autoconciliarse → 400.
    let r2 = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&a)),
            json!({ "counterpart_id": id_of(&a) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r2.status, http::StatusCode::BAD_REQUEST);
    assert!(r2.json()["message"].as_str().unwrap().contains("transfer_same_transaction"));

    // Desconciliar una suelta → 400 not_reconciled.
    let r3 = app
        .delete_with_cookie(&format!("/v1/transactions/{}/reconcile", id_of(&a)), &owner.cookie)
        .await;
    assert_eq!(r3.status, http::StatusCode::BAD_REQUEST);
    assert!(r3.json()["message"].as_str().unwrap().contains("not_reconciled"));
}

#[tokio::test]
async fn manual_reconcile_against_taken_leg_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "100", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"], "par previo");
    // Una tercera pata −100 lejana (el pase no la toca: 06-25).
    let c = create_txn(&app, &owner.cookie, "2026-06-25", "Otra salida", "-100", "expense").await;

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&c)),
            json!({ "counterpart_id": id_of(&b) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "pata ya tomada: {r:?}");
    assert!(r.json()["message"].as_str().unwrap().contains("already_reconciled"));
}

#[tokio::test]
async fn manual_reconcile_clears_a_previous_rejection() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "100", "income").await;
    assert_eq!(b["transfer_counterpart_id"], a["id"]);

    // Desconciliar (rechazo) y re-conciliar A MANO el mismo par → el rechazo se borra.
    let u = app
        .delete_with_cookie(&format!("/v1/transactions/{}/reconcile", id_of(&a)), &owner.cookie)
        .await;
    assert_eq!(u.status, http::StatusCode::OK);
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&a)),
            json!({ "counterpart_id": id_of(&b) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "manual tras rechazo: {r:?}");
    assert_eq!(r.json()["transaction"]["transfer_reconciled_source"], "manual");

    // Y tras desconciliar otra vez, el pase sigue sin resucitarlo (rechazo re-persistido).
    let u2 = app
        .delete_with_cookie(&format!("/v1/transactions/{}/reconcile", id_of(&a)), &owner.cookie)
        .await;
    assert_eq!(u2.status, http::StatusCode::OK);
    let pass = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(pass.json()["pairs_created"].as_u64(), Some(0));
}

// ---------------------------------------------------------------------------
// Guardias
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_403_on_all_reconcile_routes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "vic", "viewer").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-100", "expense").await;

    let r = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &viewer.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::FORBIDDEN, "pase: {r:?}");
    let r2 = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&a)),
            json!({ "counterpart_id": id_of(&a) }),
            &viewer.cookie,
        )
        .await;
    assert_eq!(r2.status, http::StatusCode::FORBIDDEN, "par: {r2:?}");
    let r3 = app
        .delete_with_cookie(&format!("/v1/transactions/{}/reconcile", id_of(&a)), &viewer.cookie)
        .await;
    assert_eq!(r3.status, http::StatusCode::FORBIDDEN, "desconciliar: {r3:?}");
}

#[tokio::test]
async fn reconcile_cross_user_transaction_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida alice", "-100", "expense").await;

    // bob no puede operar sobre un movimiento de alice (owner-guard → 404).
    let r = app
        .delete_with_cookie(&format!("/v1/transactions/{}/reconcile", id_of(&a)), &bob.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::NOT_FOUND, "cross-user: {r:?}");
}
