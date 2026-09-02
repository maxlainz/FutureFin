//! Sugerencias de conciliación (`GET /v1/transactions/transfer-matches`) y confirmación de una de
//! ellas (`POST /v1/transactions/transfer-matches/{match_id}`).
//!
//! Dos invariantes, y el primero tiene incidente propio en la arqueología del repositorio:
//!
//! 1. **El GET no muta.** Hasta 4.4.0 la única forma de ver un par candidato era ejecutar el pase
//!    (`POST /v1/transactions/reconcile`), y la tentación era un `?dry_run` sobre ese POST. Aquí se
//!    comprueba que listar dos veces sigue devolviendo lo mismo y que nada queda conciliado.
//! 2. **La confirmación se nombra por `match_id`, no por dos UUID.** Un id inventado no resuelve
//!    (404 `transfer_match_not_found`), así que el espacio de acciones alcanzables es exactamente
//!    el de los pares que el servidor propondría.

mod common;

use chrono::{Datelike, NaiveDate};
use common::TestApp;
use http::StatusCode;
use serde_json::{json, Value};

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
}

fn date_in(year: i32, month: u32, day: u32) -> String {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn manual(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
) -> Value {
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind }),
            cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "manual {concept}: {r:?}");
    r.json()
}

async fn txn(app: &TestApp, cookie: &str, id: &str) -> Value {
    let r = app.get_with_cookie("/v1/transactions", cookie).await;
    r.json()
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(id))
        .cloned()
        .unwrap_or_else(|| panic!("no existe el movimiento {id}"))
}

#[tokio::test]
async fn suggestions_are_a_pure_read_and_confirming_one_reconciles_the_pair() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("match_owner").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);

    // 12 días de separación: FUERA de la ventana del pase automático (5 días), así que el pase no
    // los toca — que es justo el caso donde una sugerencia sirve para algo.
    let out_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 3), "Traspaso salida", "-750", "expense").await;
    let in_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 15), "Traspaso entrada", "750", "income").await;
    let out_id = out_leg["id"].as_str().unwrap().to_string();
    let in_id = in_leg["id"].as_str().unwrap().to_string();
    assert!(
        txn(&app, &owner.cookie, &out_id).await["transfer_counterpart_id"].is_null(),
        "el pase automático NO debe alcanzar un par a 12 días"
    );

    // ---- El GET propone, y no toca nada ------------------------------------------------------
    let r = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["suggestion_count"].as_i64().unwrap(), 1, "{b}");
    assert_eq!(b["candidate_pair_count"].as_i64().unwrap(), 1, "{b}");
    assert_eq!(b["window_days"].as_i64().unwrap(), 30, "{b}");
    assert_eq!(b["auto_window_days"].as_i64().unwrap(), 5, "{b}");
    let s = &b["suggestions"][0];
    assert_eq!(s["day_gap"].as_i64().unwrap(), 12, "{s}");
    assert_eq!(s["within_auto_window"], json!(false), "{s}");
    assert_eq!(s["ambiguous"], json!(false), "{s}");
    assert_eq!(s["outgoing"]["id"], json!(out_id), "{s}");
    assert_eq!(s["incoming"]["id"], json!(in_id), "{s}");
    let match_id = s["match_id"].as_str().unwrap().to_string();

    // Repetir el GET: mismo `match_id`, y las patas siguen SUELTAS. Un GET que conciliara sería la
    // misma clase de bug que los GET que borraban pasivos vencidos.
    let again = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await
        .json();
    assert_eq!(again["suggestions"][0]["match_id"], json!(match_id), "{again}");
    for id in [&out_id, &in_id] {
        assert!(
            txn(&app, &owner.cookie, id).await["transfer_counterpart_id"].is_null(),
            "listar sugerencias NO puede conciliar (movimiento {id})"
        );
    }

    // ---- Un `match_id` inventado no resuelve --------------------------------------------------
    let bogus = app
        .post_json_with_cookie(
            "/v1/transactions/transfer-matches/deadbeefdeadbeefdeadbeef",
            json!({}),
            &owner.cookie,
        )
        .await;
    assert_eq!(bogus.status, StatusCode::NOT_FOUND, "{bogus:?}");
    assert_eq!(
        bogus.json()["code"],
        json!("transfer_match_not_found"),
        "{:?}",
        bogus.json()
    );

    // ---- Confirmar la propuesta ---------------------------------------------------------------
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/transfer-matches/{match_id}"),
            json!({}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{r:?}");
    let pair = r.json();
    assert_eq!(pair["transaction"]["id"], json!(out_id), "{pair}");
    assert_eq!(pair["counterpart"]["id"], json!(in_id), "{pair}");
    assert_eq!(
        pair["transaction"]["transfer_reconciled_source"],
        json!("manual"),
        "{pair}"
    );
    for id in [&out_id, &in_id] {
        assert!(
            txn(&app, &owner.cookie, id).await["transfer_counterpart_id"].is_string(),
            "el movimiento {id} debería estar conciliado"
        );
    }

    // Ya no queda nada que proponer.
    let after = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await
        .json();
    assert_eq!(after["suggestion_count"].as_i64().unwrap(), 0, "{after}");

    // ---- Reintentar la confirmación es idempotente --------------------------------------------
    let retry = app
        .post_json_with_cookie(
            &format!("/v1/transactions/transfer-matches/{match_id}"),
            json!({}),
            &owner.cookie,
        )
        .await;
    assert_eq!(retry.status, StatusCode::OK, "reintento: {retry:?}");
    assert_eq!(retry.json()["transaction"]["id"], json!(out_id), "{:?}", retry.json());
}

#[tokio::test]
async fn rejected_pairs_are_not_suggested_but_are_counted() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("reject_owner").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);

    // A 2 días: el pase automático los concilia solo al crear la segunda pata.
    let out_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 3), "Traspaso salida", "-300", "expense").await;
    let out_id = out_leg["id"].as_str().unwrap().to_string();
    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Traspaso entrada", "300", "income").await;
    assert!(
        txn(&app, &owner.cookie, &out_id).await["transfer_counterpart_id"].is_string(),
        "el pase automático debería haber conciliado el par de 2 días"
    );

    // Desconciliar persiste un rechazo: el pase no lo resucita, y la sugerencia tampoco debe
    // proponerlo — pero SÍ debe poder explicar por qué no está.
    let r = app
        .delete_with_cookie(&format!("/v1/transactions/{out_id}/reconcile"), &owner.cookie)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{r:?}");

    let b = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await
        .json();
    assert_eq!(b["suggestion_count"].as_i64().unwrap(), 0, "{b}");
    assert_eq!(b["rejected_pairs_excluded"].as_i64().unwrap(), 1, "{b}");
}

#[tokio::test]
async fn suggestion_window_is_validated_not_clamped() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("window_owner").await;

    for (qs, code) in [
        ("window_days=0", "window_days_out_of_range"),
        ("window_days=366", "window_days_out_of_range"),
        ("limit=0", "limit_out_of_range"),
        ("limit=101", "limit_out_of_range"),
    ] {
        let r = app
            .get_with_cookie(
                &format!("/v1/transactions/transfer-matches?{qs}"),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{qs}: {r:?}");
        assert_eq!(r.json()["code"], json!(code), "{qs}: {:?}", r.json());
    }
}

#[tokio::test]
async fn refund_pairs_are_absent_from_suggestions() {
    // La lectura de sugerencias comparte EL predicado con el pase (`candidates_from_where`), así
    // que la exclusión de las devoluciones de 4.15.0 tiene que verse también aquí — y sin que la
    // lista se quede muda para los pares legítimos. El contraste está dentro del mismo test a
    // propósito: un `suggestion_count = 0` puede significar «bien excluido» o «el endpoint no ve
    // nada», y solo el par bueno de al lado distingue las dos cosas.
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("refund_owner").await;
    let compras = app.create_category(&owner, "expense", "Compras online").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);

    // Devolución (+49,90 clasificada como expense) y su cargo espejo, a 12 días: fuera de la
    // ventana del pase, o sea justo el terreno de las sugerencias.
    let abono = manual(&app, &owner.cookie, &date_in(sy, sm, 3), "Abono TIENDA", "49.90", "income").await;
    let abono_id = abono["id"].as_str().unwrap().to_string();
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{abono_id}"),
            json!({ "kind": "expense", "category_id": compras }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "reclasificar a devolución: {r:?}");
    let cargo = manual(&app, &owner.cookie, &date_in(sy, sm, 15), "TIENDA EJEMPLO", "-49.90", "expense").await;
    let cargo_id = cargo["id"].as_str().unwrap().to_string();

    let s = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await;
    assert_eq!(s.status, StatusCode::OK, "{s:?}");
    assert_eq!(
        s.json()["suggestion_count"].as_i64().unwrap(),
        0,
        "la devolución no se propone: {s:?}"
    );

    // …y un traspaso de verdad, en el mismo dataset y a la misma distancia, SÍ se propone.
    let out_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 4), "Traspaso salida", "-300", "expense").await;
    let in_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 16), "Traspaso entrada", "300", "income").await;
    let s2 = app
        .get_with_cookie("/v1/transactions/transfer-matches", &owner.cookie)
        .await;
    let b2 = s2.json();
    assert_eq!(b2["suggestion_count"].as_i64().unwrap(), 1, "el par legítimo sí: {b2}");
    let sug = &b2["suggestions"][0];
    assert_eq!(sug["outgoing"]["id"], out_leg["id"], "{sug}");
    assert_eq!(sug["incoming"]["id"], in_leg["id"], "{sug}");

    // Nada se ha conciliado por el camino: el GET no muta (invariante 1 del módulo).
    for id in [&abono_id, &cargo_id] {
        assert!(
            txn(&app, &owner.cookie, id).await["transfer_counterpart_id"].is_null(),
            "el GET no concilia {id}"
        );
    }
}
