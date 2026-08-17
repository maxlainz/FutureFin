//! Integración de las reglas de transacción recurrente (`/v1/transactions/recurring`).
//!
//! Desde 3.2.0 las reglas tienen resolución MENSUAL (sin `day_of_month`): la instancia del mes M
//! se fecha en el ÚLTIMO día de M y solo se materializa con M ya cerrado (servidor en M+1). Cubre:
//! alta con recurrencia (single + batch) creando regla + instancia enlazada; materialización
//! idempotente que jamás incluye el mes en curso; instancias fechadas a fin de mes (feb/abr);
//! campo legacy `day_of_month` ignorado; instancia borrada que no se recrea; borrado de regla que
//! conserva instancias (FK SET NULL); viewer 403; aislamiento cross-user; y dedup sin 409 frente a
//! un manual idéntico. El "hoy" se deriva del servidor (anchor de history) para no depender del
//! reloj de la máquina.

mod common;

use chrono::{Datelike, NaiveDate};
use common::TestApp;
use serde_json::{json, Value};
use uuid::Uuid;

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    (
        (zero.div_euclid(12)) as i32,
        (zero.rem_euclid(12) + 1) as u32,
    )
}

/// Rebobina una regla al estado "recién creada, SIN backfill": borra sus instancias fuera del mes de
/// origen `(oy, om)` y devuelve el cursor al primer día de ese mes. Necesario desde el fix del
/// backfill-en-create (un alta con fecha pasada ya materializa los meses cerrados intermedios en el
/// commit del alta): para volver a ejercitar el ENDPOINT `materialize` dejamos sólo el origen
/// pendiente.
async fn rewind_rule_to_origin(app: &TestApp, rule_id: &str, oy: i32, om: u32) {
    let rid = Uuid::parse_str(rule_id).unwrap();
    let origin_start = NaiveDate::from_ymd_opt(oy, om, 1).unwrap();
    let (ny, nm) = shift_month(oy, om, 1);
    let next_start = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    sqlx::query(
        "DELETE FROM transactions \
         WHERE recurring_rule_id = $1 AND (op_date < $2 OR op_date >= $3)",
    )
    .bind(rid)
    .bind(origin_start)
    .bind(next_start)
    .execute(&app.pool)
    .await
    .expect("delete backfilled instances");
    sqlx::query(
        "UPDATE recurring_transaction_rules SET last_materialized_month = $1 WHERE id = $2",
    )
    .bind(origin_start)
    .bind(rid)
    .execute(&app.pool)
    .await
    .expect("rewind cursor");
}

/// Nº de días del mes civil `(year, month)`.
fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = shift_month(year, month, 1);
    (NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - NaiveDate::from_ymd_opt(year, month, 1).unwrap())
        .num_days() as u32
}

fn date_in(year: i32, month: u32, day: u32) -> String {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}

/// Último día del mes civil `(year, month)` como `YYYY-MM-DD`.
fn end_of_month(year: i32, month: u32) -> String {
    date_in(year, month, days_in_month(year, month))
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn list_month(app: &TestApp, cookie: &str, ym: &str) -> Value {
    app.get_with_cookie(&format!("/v1/transactions?month={ym}"), cookie)
        .await
        .json()
}

async fn materialize(app: &TestApp, cookie: &str) -> common::ResponseParts {
    app.post_json_with_cookie("/v1/transactions/recurring/materialize", json!({}), cookie)
        .await
}

// ---------------------------------------------------------------------------
// Alta con recurrencia
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_with_recurrence_single_creates_rule_and_instance() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let op = date_in(today.year(), today.month(), 20);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op, "concept": "Nomina", "amount": "1800", "kind": "income",
                    "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create: {r:?}");
    let rule_id = r.json()["recurring_rule_id"]
        .as_str()
        .expect("origin linked to rule")
        .to_string();

    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    let rb = rules.json();
    assert_eq!(rb.as_array().unwrap().len(), 1, "una regla: {rb:?}");
    assert_eq!(rb[0]["id"], json!(rule_id));
    assert_eq!(rb[0]["concept"], "Nomina");
    assert_eq!(rb[0]["kind"], "income");
    assert!(
        rb[0].get("day_of_month").is_none(),
        "las reglas son mensuales: sin day_of_month en la respuesta"
    );
    assert_eq!(
        rb[0]["amount"].as_str().unwrap().parse::<f64>().unwrap(),
        1800.0
    );
    // Cursor = primer día del mes de op_date (la instancia de origen no se re-materializa).
    assert_eq!(
        rb[0]["last_materialized_month"].as_str().unwrap(),
        date_in(today.year(), today.month(), 1)
    );
}

#[tokio::test]
async fn create_with_recurrence_batch_item_creates_rule() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let op = date_in(today.year(), today.month(), 5);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "transactions": [
                { "op_date": op, "concept": "Cafe", "amount": "-3", "kind": "expense" },
                { "op_date": op, "concept": "Alquiler", "amount": "-800", "kind": "expense",
                  "recurrence": {} },
            ] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "batch: {r:?}");
    let arr = r.json();
    // El primer ítem no tiene regla; el segundo sí.
    assert!(arr[0].get("recurring_rule_id").is_none(), "Cafe sin regla");
    assert!(
        arr[1]["recurring_rule_id"].is_string(),
        "Alquiler enlazado a regla"
    );

    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    let rb = rules.json();
    assert_eq!(rb.as_array().unwrap().len(), 1, "una sola regla del batch");
    assert_eq!(rb[0]["concept"], "Alquiler");
}

/// Un cliente ≤3.1.0 que aún envíe `recurrence.day_of_month` no rompe: el campo se ignora (las
/// reglas son mensuales) y el alta se crea igual.
#[tokio::test]
async fn legacy_day_of_month_field_is_ignored() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let op = date_in(today.year(), today.month(), 10);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op, "concept": "Nomina", "amount": "1500", "kind": "income",
                    "recurrence": { "day_of_month": 15 } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "legacy field: {r:?}");
    assert!(r.json()["recurring_rule_id"].is_string());
}

#[tokio::test]
async fn recurrence_op_date_too_old_422() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    // 11 años atrás → más allá de la cota de 10 años del backfill.
    let op_date = date_in(today.year() - 11, today.month(), 15);
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op_date, "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        r.status,
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "11 años: {r:?}"
    );
    assert!(r.json()["message"]
        .as_str()
        .unwrap()
        .contains("recurrence_too_old"));
}

#[tokio::test]
async fn recurrence_op_date_within_bound_created() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    // 9 años atrás → dentro de la cota; el alta se crea y backfillea sin 422.
    let op_date = date_in(today.year() - 9, today.month(), 15);
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op_date, "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "9 años: {r:?}");
    assert!(r.json()["recurring_rule_id"].is_string());
}

// ---------------------------------------------------------------------------
// Materialización
// ---------------------------------------------------------------------------

#[tokio::test]
async fn materialize_fills_from_origin_and_is_idempotent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -3);

    // Regla con origen 3 meses atrás. El alta con fecha pasada YA backfillea los meses CERRADOS
    // intermedios (M-2, M-1) dentro del mismo commit del create; el mes en curso jamás entra.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 1), "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();
    assert_eq!(
        app.count_rows("transactions").await,
        3,
        "origen + 2 backfilleadas (M-2, M-1) en el create; el mes en curso fuera"
    );

    // Rebobinamos (borra M-2/M-1, cursor → origen) para volver a ejercitar el ENDPOINT materialize.
    rewind_rule_to_origin(&app, &rule_id, oy, om).await;
    assert_eq!(
        app.count_rows("transactions").await,
        1,
        "sólo el origen tras rebobinar"
    );

    // El endpoint materializa M-2 y M-1 (meses cerrados), cada una fechada a fin de mes.
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(mat.json()["rules_processed"].as_u64().unwrap(), 1);
    assert_eq!(
        mat.json()["materialized"].as_u64().unwrap(),
        2,
        "M-2 y M-1; el mes en curso jamás"
    );
    assert_eq!(
        app.count_rows("transactions").await,
        3,
        "origen + 2 materializadas"
    );
    for back in [2i32, 1] {
        let (my, mm) = shift_month(today.year(), today.month(), -back);
        let l = list_month(&app, &owner.cookie, &format!("{my:04}-{mm:02}")).await;
        assert_eq!(l.as_array().unwrap().len(), 1, "instancia M-{back}");
        assert_eq!(
            l[0]["op_date"].as_str().unwrap(),
            end_of_month(my, mm),
            "fechada el último día de su mes"
        );
    }

    // Idempotente: 2ª llamada no genera nada.
    let mat2 = materialize(&app, &owner.cookie).await;
    assert_eq!(
        mat2.json()["materialized"].as_u64().unwrap(),
        0,
        "idempotente"
    );
    assert_eq!(app.count_rows("transactions").await, 3);
}

#[tokio::test]
async fn create_with_past_date_backfills_instances() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -4);

    // Alta manual con recurrencia y fecha ~4 meses atrás. NO llamamos a materialize: el backfill
    // debe ocurrir dentro del propio create, con las instancias fechadas a fin de mes.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 15), "concept": "Nomina", "amount": "1800",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // Meses cerrados intermedios M-3, M-2, M-1: una instancia (último día) en cada uno.
    for back in [3i32, 2, 1] {
        let (my, mm) = shift_month(today.year(), today.month(), -back);
        let l = list_month(&app, &owner.cookie, &format!("{my:04}-{mm:02}")).await;
        assert_eq!(
            l.as_array().unwrap().len(),
            1,
            "mes M-{back} backfilleado: {l:?}"
        );
        assert_eq!(l[0]["op_date"].as_str().unwrap(), end_of_month(my, mm));
    }
    // El origen (M-4) sigue presente con su fecha real (día 15).
    let origin = list_month(&app, &owner.cookie, &format!("{oy:04}-{om:02}")).await;
    assert_eq!(origin.as_array().unwrap().len(), 1, "origen M-4");
    assert_eq!(origin[0]["op_date"].as_str().unwrap(), date_in(oy, om, 15));

    // Mes en curso: JAMÁS se materializa (sea cual sea el día de ejecución).
    let cur_ym = format!("{:04}-{:02}", today.year(), today.month());
    let cur = list_month(&app, &owner.cookie, &cur_ym).await;
    assert_eq!(cur.as_array().unwrap().len(), 0, "mes en curso vacío");

    // Total = origen + 3 meses cerrados intermedios.
    assert_eq!(
        app.count_rows("transactions").await,
        4,
        "backfill hecho en el create"
    );

    // Un materialize posterior no crea nada (el cursor ya avanzó durante el create).
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(
        mat.json()["materialized"].as_u64().unwrap(),
        0,
        "backfill ya hecho en el create"
    );
    assert_eq!(
        app.count_rows("transactions").await,
        4,
        "sin cambios tras materialize"
    );
}

/// Las instancias van fechadas al último día natural de su mes: 28/29 en febrero, 30 en abril.
#[tokio::test]
async fn closed_month_instances_dated_end_of_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let year = today.year() - 1; // año natural completamente en el pasado.

    // Regla con origen 31 de enero del año pasado; el create backfillea todos los meses cerrados.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(year, 1, 31), "concept": "Cuota", "amount": "-100",
                    "kind": "expense", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    materialize(&app, &owner.cookie).await;

    // Febrero: último día (28 o 29 según bisiesto).
    let feb_day = if NaiveDate::from_ymd_opt(year, 2, 29).is_some() {
        29
    } else {
        28
    };
    let feb = list_month(&app, &owner.cookie, &format!("{year:04}-02")).await;
    assert_eq!(feb.as_array().unwrap().len(), 1, "una instancia en febrero");
    assert_eq!(
        feb[0]["op_date"].as_str().unwrap(),
        date_in(year, 2, feb_day)
    );

    // Abril: día 30. Diciembre: día 31.
    let apr = list_month(&app, &owner.cookie, &format!("{year:04}-04")).await;
    assert_eq!(apr.as_array().unwrap().len(), 1, "una instancia en abril");
    assert_eq!(apr[0]["op_date"].as_str().unwrap(), date_in(year, 4, 30));
    let dec = list_month(&app, &owner.cookie, &format!("{year:04}-12")).await;
    assert_eq!(dec.as_array().unwrap().len(), 1, "una instancia en diciembre");
    assert_eq!(dec[0]["op_date"].as_str().unwrap(), date_in(year, 12, 31));
}

/// El mes en curso jamás se materializa — ni siquiera cuando hoy es su último día. Sus recurrentes
/// aparecen en la primera llamada con el servidor ya en el mes siguiente (que aquí es exactamente
/// lo que le pasa a M-1: es un mes cerrado cuya instancia se crea "desde" el mes actual).
#[tokio::test]
async fn materialize_never_includes_current_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 1), "concept": "Cuota", "amount": "-30",
                    "kind": "expense", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();

    // Rebobinamos al origen para ejercitar el ENDPOINT materialize (el create ya backfilleó M-1).
    rewind_rule_to_origin(&app, &rule_id, oy, om).await;

    // Materializa sólo M-1 (cerrado), fechada a su fin de mes; el mes en curso queda fuera.
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(
        mat.json()["materialized"].as_u64().unwrap(),
        1,
        "sólo M-1; el mes en curso jamás"
    );
    let l = list_month(&app, &owner.cookie, &format!("{m1y:04}-{m1m:02}")).await;
    assert_eq!(l.as_array().unwrap().len(), 1);
    assert_eq!(
        l[0]["op_date"].as_str().unwrap(),
        end_of_month(m1y, m1m),
        "instancia del mes cerrado fechada a su último día"
    );

    // El cursor se queda en el mes anterior (no avanzó al mes en curso).
    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    assert_eq!(
        rules.json()[0]["last_materialized_month"].as_str().unwrap(),
        date_in(m1y, m1m, 1),
        "cursor en el mes anterior"
    );

    // El mes en curso no tiene ninguna instancia.
    let cur_ym = format!("{:04}-{:02}", today.year(), today.month());
    let cur = list_month(&app, &owner.cookie, &cur_ym).await;
    assert_eq!(cur.as_array().unwrap().len(), 0, "mes en curso vacío");

    // 2ª llamada tampoco lo crea (idempotente).
    let mat2 = materialize(&app, &owner.cookie).await;
    assert_eq!(
        mat2.json()["materialized"].as_u64().unwrap(),
        0,
        "sigue sin crear el mes en curso"
    );
    let cur2 = list_month(&app, &owner.cookie, &cur_ym).await;
    assert_eq!(cur2.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn deleted_instance_is_not_recreated() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -3);

    app.post_json_with_cookie(
        "/v1/transactions",
        json!({ "op_date": date_in(oy, om, 1), "concept": "Cuota", "amount": "-50",
                "kind": "expense", "recurrence": {} }),
        &owner.cookie,
    )
    .await;
    materialize(&app, &owner.cookie).await;
    assert_eq!(
        app.count_rows("transactions").await,
        3,
        "origen + M-2 + M-1"
    );

    // Borra la instancia de M-1.
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);
    let l = list_month(&app, &owner.cookie, &format!("{m1y:04}-{m1m:02}")).await;
    assert_eq!(l.as_array().unwrap().len(), 1);
    let del_id = l[0]["id"].as_str().unwrap().to_string();
    let d = app
        .delete_with_cookie(&format!("/v1/transactions/{del_id}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT);
    assert_eq!(app.count_rows("transactions").await, 2);

    // Re-materializar NO recrea la borrada (el cursor ya pasó ese mes).
    let mat2 = materialize(&app, &owner.cookie).await;
    assert_eq!(mat2.json()["materialized"].as_u64().unwrap(), 0);
    assert_eq!(
        app.count_rows("transactions").await,
        2,
        "la borrada no reaparece"
    );
}

#[tokio::test]
async fn delete_rule_keeps_instances_and_nullifies_link() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 1), "concept": "Cuota", "amount": "-50",
                    "kind": "expense", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();
    materialize(&app, &owner.cookie).await; // origen(M-2) + M-1 = 2 transacciones (M fuera).
    assert_eq!(app.count_rows("transactions").await, 2);

    // Borrar la regla: 204 y las instancias sobreviven con recurring_rule_id NULL.
    let d = app
        .delete_with_cookie(
            &format!("/v1/transactions/recurring/{rule_id}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT);
    assert_eq!(
        app.count_rows("transactions").await,
        2,
        "instancias conservadas"
    );
    let still_linked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE recurring_rule_id IS NOT NULL")
            .fetch_one(&app.pool)
            .await
            .expect("count linked");
    assert_eq!(still_linked, 0, "FK SET NULL: sin enlaces colgantes");

    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    assert_eq!(rules.json().as_array().unwrap().len(), 0, "regla borrada");
}

#[tokio::test]
async fn dedup_preexisting_manual_takes_next_ordinal_without_409() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);
    let (oy, om) = shift_month(today.year(), today.month(), -2);

    // Manual idéntico al que la regla generará en M-1: mismo concepto/importe y fechado el ÚLTIMO
    // día de M-1 (la fecha que usa ahora el materializador).
    let man = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": end_of_month(m1y, m1m), "concept": "Alquiler", "amount": "-800",
                    "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(man.status, http::StatusCode::CREATED, "{man:?}");

    // Regla con origen M-2: materializa M-1 (colisión de huella con el manual).
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 1), "concept": "Alquiler", "amount": "-800",
                    "kind": "expense", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();

    // Rebobinamos las instancias recurrentes al origen (el create ya había backfilleado M-1, con la
    // colisión ya resuelta). El manual pre-existente (M-1, sin regla) sobrevive al rebobinado.
    rewind_rule_to_origin(&app, &rule_id, oy, om).await;

    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(
        mat.status,
        http::StatusCode::OK,
        "materialize sin 409: {mat:?}"
    );
    assert_eq!(
        mat.json()["materialized"].as_u64().unwrap(),
        1,
        "sólo M-1 (el mes en curso jamás)"
    );

    // M-1 tiene 2 "Alquiler" (el manual ordinal 0 + la copia recurrente ordinal 1).
    let l = list_month(&app, &owner.cookie, &format!("{m1y:04}-{m1m:02}")).await;
    let alquiler: Vec<&Value> = l
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["concept"] == "Alquiler")
        .collect();
    assert_eq!(
        alquiler.len(),
        2,
        "manual + copia recurrente coexisten: {l:?}"
    );
}

// ---------------------------------------------------------------------------
// Autorización + aislamiento
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_materialize_or_delete_403() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app
        .register_and_approve_member(&owner, "vic", "viewer")
        .await;

    // Owner crea una regla (para tener un id que borrar).
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-15", "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();

    let m = materialize(&app, &viewer.cookie).await;
    assert_eq!(m.status, http::StatusCode::FORBIDDEN, "viewer materialize");
    let d = app
        .delete_with_cookie(
            &format!("/v1/transactions/recurring/{rule_id}"),
            &viewer.cookie,
        )
        .await;
    assert_eq!(d.status, http::StatusCode::FORBIDDEN, "viewer delete");
}

#[tokio::test]
async fn cross_user_isolation() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app
        .register_and_approve_member(&owner, "bob", "member")
        .await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 10), "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    let alice_rule = r.json()["recurring_rule_id"].as_str().unwrap().to_string();
    // El alta con fecha pasada backfillea; rebobinamos al origen para que la parte de aislamiento
    // (Bob no toca las reglas de Alice) parta de "cursor intacto en el origen, sólo la instancia de
    // origen de Alice".
    rewind_rule_to_origin(&app, &alice_rule, oy, om).await;

    // Bob no ve las reglas de Alice.
    let bl = app
        .get_with_cookie("/v1/transactions/recurring", &bob.cookie)
        .await;
    assert_eq!(
        bl.json().as_array().unwrap().len(),
        0,
        "bob no ve reglas de alice"
    );

    // Bob no puede borrar la regla de Alice → 404.
    let bd = app
        .delete_with_cookie(
            &format!("/v1/transactions/recurring/{alice_rule}"),
            &bob.cookie,
        )
        .await;
    assert_eq!(bd.status, http::StatusCode::NOT_FOUND, "cross-user delete");

    // El materialize de Bob no toca las reglas de Alice.
    let bm = materialize(&app, &bob.cookie).await;
    assert_eq!(bm.status, http::StatusCode::OK);
    assert_eq!(
        bm.json()["rules_processed"].as_u64().unwrap(),
        0,
        "bob no procesa reglas de alice"
    );
    assert_eq!(bm.json()["materialized"].as_u64().unwrap(), 0);

    // La regla de Alice sigue sin materializar (cursor intacto) y sólo existe su origen.
    let al = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    assert_eq!(
        al.json()[0]["last_materialized_month"].as_str().unwrap(),
        date_in(oy, om, 1),
        "cursor de alice intacto"
    );
    assert_eq!(
        app.count_rows("transactions").await,
        1,
        "sólo el origen de alice"
    );
}

// ---------------------------------------------------------------------------
// Categorías vs reglas recurrentes (FK ON DELETE RESTRICT)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn category_used_by_recurring_rule_requires_remap() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_a = app.create_category(&owner, "expense", "A").await;
    let cat_b = app.create_category(&owner, "expense", "B").await;
    let today = server_today(&app, &owner.cookie).await;
    let op = date_in(today.year(), today.month(), 1);

    // Alta con recurrencia usando la categoría A: crea la instancia de origen + la regla, ambas con A.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op, "concept": "Cuota", "amount": "-40", "kind": "expense",
                    "category_id": cat_a, "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let origin_id = r.json()["id"].as_str().unwrap().to_string();
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();

    // Borramos la transacción de origen → la categoría A queda referenciada SÓLO por la regla.
    let del_txn = app
        .delete_with_cookie(&format!("/v1/transactions/{origin_id}"), &owner.cookie)
        .await;
    assert_eq!(del_txn.status, http::StatusCode::NO_CONTENT, "{del_txn:?}");

    // Sin remap → 400 estándar de "en uso" (la regla la cuenta gracias al fix del reference-count).
    // El mensaje distingue este camino del 23503 opaco ("referenced record missing").
    let bad = app
        .delete_with_cookie(&format!("/v1/categories/{cat_a}"), &owner.cookie)
        .await;
    assert_eq!(
        bad.status,
        http::StatusCode::BAD_REQUEST,
        "categoría en uso por regla: {bad:?}"
    );
    assert!(
        bad.json()["message"].as_str().unwrap().contains("in use"),
        "mensaje de \"en uso\", no el 23503 opaco: {bad:?}"
    );

    // Con remap a B → 204 y la regla queda apuntando a B.
    let ok = app
        .delete_with_cookie(
            &format!("/v1/categories/{cat_a}?remap_to={cat_b}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::NO_CONTENT, "remap: {ok:?}");

    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    let rb = rules.json();
    let rule = rb
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == json!(rule_id))
        .expect("la regla sigue existiendo");
    assert_eq!(rule["category_id"], json!(cat_b), "regla remapeada a B");
}
