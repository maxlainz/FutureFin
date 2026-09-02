//! Integración de la conciliación de transferencias (3.5.0):
//! `POST /v1/transactions/reconcile` (pase explícito), `POST/DELETE /v1/transactions/{id}/reconcile`
//! (par manual / desconciliar) y el pase automático post-commit de las mutaciones.
//!
//! Contrato bajo test: el pase empareja importes exactamente opuestos del MISMO owner y misma
//! divisa a ≤5 días **con el signo natural de cada pata** (salida `expense` negativa ↔ entrada
//! `income` positiva: ni savings, ni devoluciones, ni ingresos negativos), greedy determinista
//! (gana la Δfecha menor), punto fijo (re-ejecutar → 0),
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
    // 4.15.0: el confirm exige categoría en toda decisión income/expense.
    let cat_out = app.create_category(&owner, "expense", "Traspasos").await;
    let cat_in = app.create_category(&owner, "income", "Traspasos").await;

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
                    "file_sha256": p1b["file_sha256"],
                    "decisions": [{ "kind": "expense", "category_id": cat_out }],
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
                    "file_sha256": p2b["file_sha256"],
                    "decisions": [{ "kind": "income", "category_id": cat_in }],
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
async fn savings_leg_is_not_auto_matched_nor_suggested() {
    // Contrato desde 4.14.0: la candidatura automática (pase + sugerencias) solo considera
    // patas income/expense. Una fila savings de importe exactamente opuesto a un movimiento
    // real dentro de la ventana lo emparejaría y sacaría ese movimiento de los agregados —
    // y a diferencia de un par income/expense, el neto por bucket NO se conserva. (Hasta
    // 4.13.x savings participaba; la vía manual de abajo queda como escape.)
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    create_txn(&app, &owner.cookie, "2026-06-10", "Aportación", "-200", "savings").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada opuesta", "200", "income").await;
    assert!(b["transfer_counterpart_id"].is_null(), "savings no auto-empareja: {b:?}");

    // Ni el pase explícito…
    let r = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["pairs_created"].as_i64().unwrap(), 0, "pase no empareja savings");

    // …ni las sugerencias (mismo predicado compartido).
    let s = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await;
    assert_eq!(s.status, http::StatusCode::OK, "{s:?}");
    assert_eq!(s.json()["suggestion_count"].as_i64().unwrap(), 0, "sin sugerencias savings");
}

#[tokio::test]
async fn manual_reconcile_accepts_a_savings_leg() {
    // El emparejamiento manual sigue siendo kind-agnóstico a propósito: si el usuario trackea
    // también la cuenta destino y quiere cruzar la aportación con su entrada, puede.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Aportación", "-200", "savings").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada bróker", "200", "income").await;
    assert!(b["transfer_counterpart_id"].is_null(), "precondición: sin auto-par");

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&a)),
            json!({ "counterpart_id": id_of(&b) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "manual con savings: {r:?}");
    assert_eq!(r.json()["transaction"]["transfer_counterpart_id"], b["id"]);
}

#[tokio::test]
async fn savings_pull_from_import_does_not_eat_a_real_expense() {
    // EL CASO REAL que motivó el cambio: un espacio de ahorro reembolsa una compra concreta
    // (importes idénticos por construcción, mismo día). La retirada del espacio entra como
    // savings positivo vía import (el import está exento de la regla de signo) y NO debe
    // emparejarse con el gasto real de tarjeta.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Desde 4.15.0 el confirm exige categoría en toda decisión income/expense.
    let compras = app.create_category(&owner, "expense", "Compras online").await;

    let csv = "\"Booking Date\",\"Value Date\",\"Partner Name\",\"Partner Iban\",Type,\"Payment Reference\",\"Account Name\",\"Amount (EUR)\",\"Original Amount\",\"Original Currency\",\"Exchange Rate\"\n\
        2026-06-10,2026-06-10,\"TIENDA EJEMPLO\",,Presentment,,\"Cuenta principal\",-49.9,49.9,EUR,1\n\
        2026-06-10,2026-06-10,\"Cuenta de Ahorro\",,\"Credit Transfer\",\"TIENDA EJEMPLO\",\"Cuenta principal\",49.9,,,\n";
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "n26", "file_b64": b64 }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    let pb = p.json();
    let sha = pb["file_sha256"].as_str().unwrap().to_string();

    // Decisiones del wizard: el cargo es gasto real; la retirada del espacio, savings.
    let c = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({
                "source": "n26",
                "file_b64": b64,
                "file_sha256": sha,
                "decisions": [
                    { "kind": "expense", "category_id": compras },
                    { "kind": "savings", "category_id": null },
                ],
                "learn_rules": false,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(c.status, http::StatusCode::OK, "{c:?}");

    // Ninguna de las dos patas queda conciliada: el gasto sigue contando como gasto.
    let list = app
        .get_with_cookie("/v1/transactions?month=2026-06", &owner.cookie)
        .await;
    for t in list.json().as_array().unwrap() {
        assert!(
            t["transfer_counterpart_id"].is_null(),
            "nada conciliado en el par gasto↔savings: {t:?}"
        );
    }
}

/// Crea una DEVOLUCIÓN: un `expense` de importe POSITIVO.
///
/// No hay vía directa —`assert_amount_sign_matches_kind` rechaza el alta de un gasto positivo—,
/// así que se hace como en la vida real: el abono entra como `income` (es lo que deduce el
/// importador del signo) y se RE-clasifica a `expense`. Un PATCH que no toca `amount` no valida
/// el signo a propósito (`patch_transaction_core`): reclasificar dinero que ya existe es
/// exactamente el caso que ese comentario protege.
async fn create_refund(
    app: &TestApp,
    cookie: &str,
    op_date: &str,
    concept: &str,
    amount: &str,
    category_id: &str,
) -> Value {
    let row = create_txn(app, cookie, op_date, concept, amount, "income").await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{}", id_of(&row)),
            json!({ "kind": "expense", "category_id": category_id }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "reclasificar a devolución: {r:?}");
    let out = r.json();
    assert_eq!(out["kind"], "expense", "{out:?}");
    out
}

#[tokio::test]
async fn refund_is_not_auto_matched_nor_suggested() {
    // Contrato desde 4.15.0: la pata de ENTRADA es `income` POSITIVO, no «cualquier income/expense
    // positivo». Una devolución (gasto de importe positivo) cuadra por construcción con el cargo
    // que compensa —mismo importe, mismos días, mismo comercio— y emparejarla sacaba a las DOS
    // filas de todos los agregados de flujo justo cuando lo correcto es que se resten dentro de su
    // categoría. Caso real: un abono de +49,90 se comía el cargo de −49,90.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras online").await;

    let refund =
        create_refund(&app, &owner.cookie, "2026-06-11", "Abono TIENDA", "49.90", &compras).await;
    assert!(refund["transfer_counterpart_id"].is_null(), "precondición: suelta");

    // El cargo real llega después y dispara el pase post-commit.
    let cargo =
        create_txn(&app, &owner.cookie, "2026-06-10", "TIENDA EJEMPLO", "-49.90", "expense").await;
    assert!(
        cargo["transfer_counterpart_id"].is_null(),
        "una devolución no es pata de entrada: {cargo:?}"
    );

    // Ni el pase explícito…
    let r = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(
        r.json()["pairs_created"].as_i64().unwrap(),
        0,
        "el pase no empareja devoluciones"
    );

    // …ni las sugerencias (mismo predicado compartido).
    let s = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await;
    assert_eq!(s.status, http::StatusCode::OK, "{s:?}");
    assert_eq!(
        s.json()["suggestion_count"].as_i64().unwrap(),
        0,
        "sin sugerencias de devolución"
    );

    // Y el gasto sigue siendo gasto: la devolución netea dentro de su categoría, no lo tapa.
    let cargo_now = fetch_txn(&app, &owner.cookie, "2026-06", &id_of(&cargo)).await;
    assert!(cargo_now["transfer_counterpart_id"].is_null(), "{cargo_now:?}");
}

#[tokio::test]
async fn negative_income_is_no_longer_an_outgoing_leg() {
    // EFECTO COLATERAL DECLARADO de 4.15.0: al exigir `a.kind = 'expense'` en la pata de salida, un
    // `income` NEGATIVO (la simétrica de la devolución: un ingreso devuelto) deja de ser candidato
    // automático — hasta 4.14.x sí lo era. Se fija aquí para que quien lo eche de menos encuentre
    // la decisión y no un hueco.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let nomina = app.create_category(&owner, "income", "Nómina").await;

    // Mismo truco que la devolución con los signos al revés: nace expense negativo y se reclasifica.
    let neg =
        create_txn(&app, &owner.cookie, "2026-06-10", "Nómina devuelta", "-120", "expense").await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{}", id_of(&neg)),
            json!({ "kind": "income", "category_id": nomina }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "reclasificar a income negativo: {r:?}");
    assert_eq!(r.json()["kind"], "income");

    // La contrapartida positiva dispara el pase: bajo el predicado viejo habrían emparejado.
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Nómina", "120", "income").await;
    assert!(
        b["transfer_counterpart_id"].is_null(),
        "un income negativo ya no es pata de salida: {b:?}"
    );
    let s = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await;
    assert_eq!(
        s.json()["suggestion_count"].as_i64().unwrap(),
        0,
        "tampoco se sugiere: {s:?}"
    );
}

#[tokio::test]
async fn manual_reconcile_still_accepts_a_refund_leg() {
    // La vía manual sigue siendo kind/sign-agnóstica a propósito: si el usuario decide que ese
    // abono SÍ era el espejo de aquel cargo, puede cruzarlos. 4.15.0 retira el automatismo, no la
    // capacidad.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let compras = app.create_category(&owner, "expense", "Compras online").await;

    let refund =
        create_refund(&app, &owner.cookie, "2026-06-11", "Abono TIENDA", "49.90", &compras).await;
    let cargo =
        create_txn(&app, &owner.cookie, "2026-06-10", "TIENDA EJEMPLO", "-49.90", "expense").await;
    assert!(cargo["transfer_counterpart_id"].is_null(), "precondición: sin auto-par");

    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{}/reconcile", id_of(&cargo)),
            json!({ "counterpart_id": id_of(&refund) }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "manual con devolución: {r:?}");
    assert_eq!(r.json()["transaction"]["transfer_counterpart_id"], refund["id"]);
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
    let cat_in = app.create_category(&owner, "income", "Traspasos").await;
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
                    "decisions": [{ "kind": "income", "category_id": cat_in }], "learn_rules": false }),
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

// ---------------------------------------------------------------------------
// Barrido periódico (3.8.1): la red de reintento de los pases que fallaron
// ---------------------------------------------------------------------------

use futurefin_api::handlers::transactions::reconcile::sweep_all_owners;

/// Simula lo que deja atrás un pase post-mutación fallido: el par existe y encaja, pero nadie lo
/// enlazó. **Sin registrar rechazo** — un fallo del pase no es una decisión del usuario.
async fn unlink_silently(app: &TestApp, ids: &[&str]) {
    for id in ids {
        sqlx::query(
            "UPDATE transactions
             SET transfer_counterpart_id = NULL,
                 transfer_reconciled_at = NULL,
                 transfer_reconciled_source = NULL
             WHERE id = $1::uuid",
        )
        .bind(id)
        .execute(&app.pool)
        .await
        .expect("unlink");
    }
}

/// El caso que justifica el barrido: un pase best-effort falló, el par se quedó suelto y **nada
/// lo reintentaba**. El usuario no puede enterarse, así que tampoco iba a pedir el pase manual.
#[tokio::test]
async fn sweep_recovers_pairs_a_failed_pass_left_behind() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-250", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "250", "income").await;
    let (a_id, b_id) = (id_of(&a), id_of(&b));

    // El alta ya los concilió (el pase corre en cada mutación).
    let after_create = fetch_txn(&app, &owner.cookie, "2026-06", &a_id).await;
    assert!(
        !after_create["transfer_counterpart_id"].is_null(),
        "precondición: el alta debería haberlos conciliado: {after_create}"
    );

    // Ahora se simula el pase perdido.
    unlink_silently(&app, &[&a_id, &b_id]).await;
    let broken = fetch_txn(&app, &owner.cookie, "2026-06", &a_id).await;
    assert!(broken["transfer_counterpart_id"].is_null(), "{broken}");

    let out = sweep_all_owners(&app.state).await.expect("sweep");
    assert_eq!(out.pairs_created, 1, "{out:?}");
    assert_eq!(out.owners_failed, 0, "{out:?}");

    let fixed = fetch_txn(&app, &owner.cookie, "2026-06", &a_id).await;
    assert_eq!(
        fixed["transfer_counterpart_id"].as_str(),
        Some(b_id.as_str()),
        "el barrido debe re-enlazar el par: {fixed}"
    );

    // Punto fijo: repetirlo no crea nada. Es el caso NORMAL en una instalación sana.
    let again = sweep_all_owners(&app.state).await.expect("sweep 2");
    assert_eq!(again.pairs_created, 0, "{again:?}");
    assert_eq!(again.owners_failed, 0, "{again:?}");
}

/// Recorre TODOS los owners, no solo uno: cada miembro del hogar concilia sus propias patas y el
/// fallo de uno no puede dejar al otro sin reintento.
#[tokio::test]
async fn sweep_covers_every_owner_independently() {
    let app = TestApp::spawn().await;
    let alice = app.register_and_login_owner("alice").await;
    let bob = app
        .register_and_approve_member(&alice, "bob", "member")
        .await;

    let a1 = create_txn(&app, &alice.cookie, "2026-06-10", "A salida", "-100", "expense").await;
    let a2 = create_txn(&app, &alice.cookie, "2026-06-11", "A entrada", "100", "income").await;
    let b1 = create_txn(&app, &bob.cookie, "2026-06-12", "B salida", "-70", "expense").await;
    let b2 = create_txn(&app, &bob.cookie, "2026-06-13", "B entrada", "70", "income").await;
    unlink_silently(&app, &[&id_of(&a1), &id_of(&a2), &id_of(&b1), &id_of(&b2)]).await;

    let out = sweep_all_owners(&app.state).await.expect("sweep");
    assert_eq!(out.owners_scanned, 2, "un owner por miembro con patas sueltas: {out:?}");
    assert_eq!(out.pairs_created, 2, "{out:?}");

    for (cookie, id, expected) in [
        (&alice.cookie, id_of(&a1), id_of(&a2)),
        (&bob.cookie, id_of(&b1), id_of(&b2)),
    ] {
        let row = fetch_txn(&app, cookie, "2026-06", &id).await;
        assert_eq!(row["transfer_counterpart_id"].as_str(), Some(expected.as_str()), "{row}");
    }
}

/// El barrido **no resucita** un par que el usuario desconcilió a mano: el rechazo manda. Es la
/// diferencia entre «el pase falló» y «no son la misma transferencia».
#[tokio::test]
async fn sweep_never_resurrects_a_user_rejected_pair() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let a = create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-300", "expense").await;
    let b = create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "300", "income").await;
    let a_id = id_of(&a);
    assert_eq!(
        fetch_txn(&app, &owner.cookie, "2026-06", &a_id).await["transfer_counterpart_id"].as_str(),
        Some(id_of(&b).as_str())
    );

    // Desconciliar por la API persiste el rechazo anti-resurrección.
    let del = app
        .delete_with_cookie(&format!("/v1/transactions/{a_id}/reconcile"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::OK, "{del:?}");

    let out = sweep_all_owners(&app.state).await.expect("sweep");
    assert_eq!(out.pairs_created, 0, "el rechazo del usuario manda sobre el barrido: {out:?}");
    let row = fetch_txn(&app, &owner.cookie, "2026-06", &a_id).await;
    assert!(row["transfer_counterpart_id"].is_null(), "{row}");
}

/// Instalación al día: el barrido no encuentra owners con patas sueltas y no toca la base.
#[tokio::test]
async fn sweep_is_a_noop_when_everything_is_reconciled() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    create_txn(&app, &owner.cookie, "2026-06-10", "Salida", "-40", "expense").await;
    create_txn(&app, &owner.cookie, "2026-06-11", "Entrada", "40", "income").await;

    let out = sweep_all_owners(&app.state).await.expect("sweep");
    assert_eq!(
        out.owners_scanned, 0,
        "sin patas sueltas no hay owner que revisar: {out:?}"
    );
    assert_eq!(out.pairs_created, 0, "{out:?}");
}
