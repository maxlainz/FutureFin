//! Idempotencia del alta EN LOTE (`POST /v1/transactions/batch`).
//!
//! El lote llevaba desde la Fase 3 rechazando toda clave de idempotencia con el argumento de que
//! «3 de 5 se reproducen» no tiene semántica. Es cierto — y es una pregunta mal planteada: el lote
//! no son N unidades de trabajo con N claves, es UNA unidad atómica, y por tanto lleva UNA clave.
//! Estos tests fijan las tres salidas del contrato: reproducir, chocar y rechazar la clave por ítem.

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

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_batch_key_replays_the_whole_batch_and_conflicts_on_a_different_one() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("batch_owner").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let body = |amount: &str| {
        json!({
            "idempotency_key": "importador-2026-08-28",
            "transactions": [
                { "op_date": date_in(sy, sm, 4), "concept": "Compra A", "amount": "-40", "kind": "expense" },
                { "op_date": date_in(sy, sm, 6), "concept": "Compra B", "amount": amount, "kind": "expense" },
            ]
        })
    };

    let first = app
        .post_json_with_cookie("/v1/transactions/batch", body("-25"), &owner.cookie)
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{first:?}");
    let first_ids = ids(&first.json());
    assert_eq!(first_ids.len(), 2, "{:?}", first.json());
    assert_eq!(app.count_rows("transactions").await, 2);

    // Reintento con el MISMO cuerpo: mismos ids, mismo orden, y ni una fila nueva. Esa igualdad ES
    // la idempotencia — «3 de 5» no puede ocurrir porque los N INSERT y las N claves son una sola
    // transacción.
    let replay = app
        .post_json_with_cookie("/v1/transactions/batch", body("-25"), &owner.cookie)
        .await;
    assert_eq!(replay.status, StatusCode::CREATED, "{replay:?}");
    assert_eq!(ids(&replay.json()), first_ids, "{:?}", replay.json());
    assert_eq!(
        app.count_rows("transactions").await,
        2,
        "un reintento no puede crear filas"
    );

    // Misma clave, un importe distinto: gana el primero y el segundo es un 409 ruidoso.
    let conflict = app
        .post_json_with_cookie("/v1/transactions/batch", body("-26"), &owner.cookie)
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{conflict:?}");
    assert_eq!(
        conflict.json()["code"],
        json!("idempotency_key_conflict"),
        "{:?}",
        conflict.json()
    );
    assert_eq!(app.count_rows("transactions").await, 2);

    // Cambiar el NÚMERO de ítems también choca: el tamaño entra en la huella a propósito.
    let shorter = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({
                "idempotency_key": "importador-2026-08-28",
                "transactions": [
                    { "op_date": date_in(sy, sm, 4), "concept": "Compra A", "amount": "-40", "kind": "expense" },
                ]
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(shorter.status, StatusCode::CONFLICT, "{shorter:?}");
    assert_eq!(app.count_rows("transactions").await, 2);

    // Sin clave, el comportamiento histórico intacto: reenviar el lote crea OTRO lote.
    let unkeyed = json!({
        "transactions": [
            { "op_date": date_in(sy, sm, 4), "concept": "Compra A", "amount": "-40", "kind": "expense" },
        ]
    });
    for _ in 0..2 {
        let r = app
            .post_json_with_cookie("/v1/transactions/batch", unkeyed.clone(), &owner.cookie)
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{r:?}");
    }
    assert_eq!(app.count_rows("transactions").await, 4);
}

#[tokio::test]
async fn a_per_item_key_is_still_rejected_and_says_where_the_key_goes() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("batch_item_key").await;
    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({
                "transactions": [
                    { "op_date": date_in(sy, sm, 4), "concept": "Compra A", "amount": "-40",
                      "kind": "expense", "idempotency_key": "k-1" },
                ]
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(
        r.json()["code"],
        json!("idempotency_key_batch_unsupported"),
        "{:?}",
        r.json()
    );
    assert_eq!(app.count_rows("transactions").await, 0);
}
