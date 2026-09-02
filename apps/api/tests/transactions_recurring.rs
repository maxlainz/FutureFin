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

/// Deja una regla con SOLO su instancia de origen: borra las instancias fuera del mes `(oy, om)`.
/// Desde 3.9.0 no hay cursor que rebobinar — la convergencia es idempotente por existencia, así
/// que basta con quitar las filas. Se usa para volver a ejercitar el ENDPOINT `materialize` desde
/// un estado conocido. OJO: la convergencia post-mutación las recrearía si sus meses siguen
/// activos, por eso el borrado va por SQL directo y no por la API.
async fn keep_only_origin_instance(app: &TestApp, rule_id: &str, oy: i32, om: u32) {
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
    .expect("delete non-origin instances");
}

/// ACTIVA un mes: le mete un movimiento REAL (no recurrente, no conciliado) por SQL directo.
/// Desde 3.9.0 un mes solo materializa sus recurrentes si está activo, así que casi todos los
/// escenarios necesitan sembrar esto primero. Por SQL y no por la API para no disparar la
/// convergencia post-commit antes de tiempo (los tests quieren observar CUÁNDO ocurre).
async fn activate_month_raw(app: &TestApp, owner_id: Uuid, y: i32, m: u32) {
    let iid = app.installation_id().await;
    let op = NaiveDate::from_ymd_opt(y, m, 15).unwrap();
    // Con categoría: desde 4.15.0 el CHECK `transactions_category_required_check` no admite un
    // gasto sin ella ni por SQL directo, que es justo lo que este helper hace.
    let cat = fallback_category(app, "expense").await;
    sqlx::query(
        "INSERT INTO transactions (installation_id, owner_user_id, source, op_date, concept, \
         amount, currency, kind, category_id, fingerprint, fingerprint_ordinal) \
         VALUES ($1, $2, 'manual', $3, $4, -1, 'EUR', 'expense', $5, $6, 0)",
    )
    .bind(iid)
    .bind(owner_id)
    .bind(op)
    .bind(format!("Activador {y}-{m:02}"))
    .bind(cat)
    .bind(format!("fp-activator-{y}-{m:02}"))
    .execute(&app.pool)
    .await
    .expect("insert activator movement");
}

/// Id de la categoría POR DEFECTO de un scope (4.15.0).
async fn fallback_category(app: &TestApp, scope: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM categories WHERE installation_id = (SELECT id FROM installation LIMIT 1) \
         AND scope = $1 AND is_fallback",
    )
    .bind(scope)
    .fetch_one(&app.pool)
    .await
    .expect("categoría por defecto del scope")
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
    let op = today.format("%Y-%m-%d").to_string();

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
        rb[0]["origin_month"].as_str().unwrap(),
        date_in(today.year(), today.month(), 1)
    );
}

#[tokio::test]
async fn create_with_recurrence_batch_item_creates_rule() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let op = today.format("%Y-%m-%d").to_string();

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
    let op = today.format("%Y-%m-%d").to_string();

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
async fn materialize_only_fills_active_months_and_is_idempotent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -3);

    // Regla con origen 3 meses atrás. Desde 3.9.0 el alta NO backfillea: los meses intermedios
    // están vacíos, y un mes vacío no genera recurrentes. Solo queda la instancia de origen.
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
        1,
        "solo el origen: sin datos reales en M-2/M-1 no hay nada que materializar"
    );

    // Activamos M-2 y M-1 con un movimiento real cada uno.
    for back in [2i32, 1] {
        let (my, mm) = shift_month(today.year(), today.month(), -back);
        activate_month_raw(&app, owner.user_id, my, mm).await;
    }
    assert_eq!(app.count_rows("transactions").await, 3, "origen + 2 activadores");

    // Ahora sí: el endpoint materializa M-2 y M-1, cada una fechada a fin de mes.
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(mat.json()["rules_processed"].as_u64().unwrap(), 1);
    assert_eq!(
        mat.json()["materialized"].as_u64().unwrap(),
        2,
        "M-2 y M-1 (activos); el mes en curso jamás"
    );
    assert_eq!(mat.json()["pruned"].as_u64().unwrap(), 0);
    assert_eq!(app.count_rows("transactions").await, 5);
    for back in [2i32, 1] {
        let (my, mm) = shift_month(today.year(), today.month(), -back);
        let l = list_month(&app, &owner.cookie, &format!("{my:04}-{mm:02}")).await;
        assert_eq!(l.as_array().unwrap().len(), 2, "activador + instancia M-{back}");
        let inst = l
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["recurring_rule_id"].as_str() == Some(rule_id.as_str()))
            .expect("la instancia recurrente del mes");
        assert_eq!(
            inst["op_date"].as_str().unwrap(),
            end_of_month(my, mm),
            "fechada el último día de su mes"
        );
    }

    // Idempotente por EXISTENCIA (ya no por cursor): 2ª llamada no genera ni poda nada.
    let mat2 = materialize(&app, &owner.cookie).await;
    assert_eq!(mat2.json()["materialized"].as_u64().unwrap(), 0, "idempotente");
    assert_eq!(mat2.json()["pruned"].as_u64().unwrap(), 0);
    assert_eq!(app.count_rows("transactions").await, 5);

    // Y el borrado del activador de M-1 PODA su instancia: el mes deja de estar activo.
    keep_only_origin_instance(&app, &rule_id, oy, om).await;
    let (my, mm) = shift_month(today.year(), today.month(), -1);
    sqlx::query("DELETE FROM transactions WHERE op_date >= $1 AND op_date < $2 AND recurring_rule_id IS NULL")
        .bind(NaiveDate::from_ymd_opt(my, mm, 1).unwrap())
        .bind(NaiveDate::from_ymd_opt(my, mm, days_in_month(my, mm)).unwrap() + chrono::Duration::days(1))
        .execute(&app.pool)
        .await
        .expect("borrar el activador de M-1");
    let mat3 = materialize(&app, &owner.cookie).await;
    assert_eq!(
        mat3.json()["materialized"].as_u64().unwrap(),
        1,
        "M-2 sigue activo y recupera su instancia"
    );
    assert_eq!(
        mat3.json()["pruned"].as_u64().unwrap(),
        0,
        "M-1 ya no tenía instancia que podar (la borró keep_only_origin_instance)"
    );
}

#[tokio::test]
async fn create_with_past_date_only_fills_months_that_have_real_data() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -4);

    // INVERSIÓN DELIBERADA DE CONTRATO (3.9.0). Antes, un alta con fecha pasada backfilleaba
    // TODOS los meses cerrados intermedios en el mismo commit. Ahora las instancias siguen a los
    // datos: sembramos un movimiento real solo en M-2, así que solo M-2 debe recibir la suya.
    let (ay, am) = shift_month(today.year(), today.month(), -2);
    activate_month_raw(&app, owner.user_id, ay, am).await;

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 15), "concept": "Nomina", "amount": "1800",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // M-2 (activo) recibe su instancia en el mismo request, vía la convergencia post-commit.
    let l2 = list_month(&app, &owner.cookie, &format!("{ay:04}-{am:02}")).await;
    assert_eq!(l2.as_array().unwrap().len(), 2, "activador + instancia: {l2:?}");
    let inst = l2
        .as_array()
        .unwrap()
        .iter()
        .find(|t| !t["recurring_rule_id"].is_null())
        .expect("instancia recurrente en M-2");
    assert_eq!(inst["op_date"].as_str().unwrap(), end_of_month(ay, am));

    // M-3 y M-1 están VACÍOS: sin movimientos reales no se genera relleno sintético.
    for back in [3i32, 1] {
        let (my, mm) = shift_month(today.year(), today.month(), -back);
        let l = list_month(&app, &owner.cookie, &format!("{my:04}-{mm:02}")).await;
        assert_eq!(
            l.as_array().unwrap().len(),
            0,
            "mes M-{back} sin datos reales debe quedar VACÍO: {l:?}"
        );
    }

    // El origen (M-4) sigue presente con su fecha real (día 15): la poda lo exime siempre.
    let origin = list_month(&app, &owner.cookie, &format!("{oy:04}-{om:02}")).await;
    assert_eq!(origin.as_array().unwrap().len(), 1, "origen M-4 exento de la poda");
    assert_eq!(origin[0]["op_date"].as_str().unwrap(), date_in(oy, om, 15));

    // Mes en curso: JAMÁS se materializa (sea cual sea el día de ejecución).
    let cur_ym = format!("{:04}-{:02}", today.year(), today.month());
    let cur = list_month(&app, &owner.cookie, &cur_ym).await;
    assert_eq!(cur.as_array().unwrap().len(), 0, "mes en curso vacío");

    // Total = origen + activador de M-2 + instancia de M-2.
    assert_eq!(app.count_rows("transactions").await, 3);

    // Un materialize posterior es no-op: ya está convergido.
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(mat.json()["materialized"].as_u64().unwrap(), 0, "ya convergido");
    assert_eq!(mat.json()["pruned"].as_u64().unwrap(), 0);
    assert_eq!(app.count_rows("transactions").await, 3);
}

/// Las instancias van fechadas al último día natural de su mes: 28/29 en febrero, 30 en abril.
#[tokio::test]
async fn closed_month_instances_dated_end_of_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let year = today.year() - 1; // año natural completamente en el pasado.

    // 3.9.0: solo los meses ACTIVOS materializan, así que los sembramos primero.
    for m in [2u32, 4, 12] {
        activate_month_raw(&app, owner.user_id, year, m).await;
    }

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
    let feb_day = if NaiveDate::from_ymd_opt(year, 2, 29).is_some() { 29 } else { 28 };
    for (m, day) in [(2u32, feb_day), (4, 30), (12, 31)] {
        let l = list_month(&app, &owner.cookie, &format!("{year:04}-{m:02}")).await;
        let inst = l
            .as_array()
            .unwrap()
            .iter()
            .find(|t| !t["recurring_rule_id"].is_null())
            .unwrap_or_else(|| panic!("instancia recurrente en {year}-{m:02}: {l:?}"));
        assert_eq!(
            inst["op_date"].as_str().unwrap(),
            date_in(year, m, day),
            "fechada al último día natural de su mes"
        );
    }

    // Un mes NO activo del mismo año no recibe instancia (marzo).
    let mar = list_month(&app, &owner.cookie, &format!("{year:04}-03")).await;
    assert_eq!(mar.as_array().unwrap().len(), 0, "marzo sin datos reales: vacío");
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

    // Activamos M-1 Y el mes EN CURSO: el mes abierto tiene datos reales y aun así no debe
    // recibir su recurrente. Es la aserción que discrimina «cerrado» de «con datos».
    activate_month_raw(&app, owner.user_id, m1y, m1m).await;
    activate_month_raw(&app, owner.user_id, today.year(), today.month()).await;

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

    keep_only_origin_instance(&app, &rule_id, oy, om).await;

    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");
    assert_eq!(
        mat.json()["materialized"].as_u64().unwrap(),
        1,
        "sólo M-1; el mes en curso jamás, aunque tenga datos reales"
    );
    let l = list_month(&app, &owner.cookie, &format!("{m1y:04}-{m1m:02}")).await;
    let inst = l
        .as_array()
        .unwrap()
        .iter()
        .find(|t| !t["recurring_rule_id"].is_null())
        .expect("instancia en M-1");
    assert_eq!(
        inst["op_date"].as_str().unwrap(),
        end_of_month(m1y, m1m),
        "instancia del mes cerrado fechada a su último día"
    );

    // El mes en curso solo tiene su activador, ninguna instancia sintética.
    let cur = list_month(
        &app,
        &owner.cookie,
        &format!("{:04}-{:02}", today.year(), today.month()),
    )
    .await;
    assert_eq!(cur.as_array().unwrap().len(), 1, "solo el activador: {cur:?}");
    assert!(
        cur[0]["recurring_rule_id"].is_null(),
        "el mes abierto nunca muestra movimientos sintéticos"
    );

    // El ANCLA no se mueve: no es un cursor, no avanza con la materialización.
    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    assert_eq!(
        rules.json()[0]["origin_month"].as_str().unwrap(),
        date_in(oy, om, 1),
        "origin_month es un ancla inmóvil"
    );
}

#[tokio::test]
async fn deleted_instance_is_recreated_while_its_month_stays_active() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -3);
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);

    // INVERSIÓN DELIBERADA DE CONTRATO (3.9.0). Con el cursor, borrar una instancia la borraba
    // PARA SIEMPRE. Con idempotencia por existencia, la regla es la fuente de verdad: mientras su
    // mes siga activo, la instancia vuelve. Para quitarla de verdad se borra la PLANTILLA.
    activate_month_raw(&app, owner.user_id, m1y, m1m).await;

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
        "origen + activador M-1 + instancia M-1"
    );

    // Borra la instancia de M-1 por la API.
    let l = list_month(&app, &owner.cookie, &format!("{m1y:04}-{m1m:02}")).await;
    let del_id = l
        .as_array()
        .unwrap()
        .iter()
        .find(|t| !t["recurring_rule_id"].is_null())
        .expect("instancia en M-1")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let d = app
        .delete_with_cookie(&format!("/v1/transactions/{del_id}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT);

    // La convergencia post-commit del PROPIO delete ya la ha repuesto: el mes sigue activo.
    assert_eq!(
        app.count_rows("transactions").await,
        3,
        "la instancia vuelve en el mismo request: la plantilla manda"
    );

    // Y si el mes deja de estar activo (se borra su último movimiento real), se PODA.
    let act_id = list_month(&app, &owner.cookie, &format!("{m1y:04}-{m1m:02}"))
        .await
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["recurring_rule_id"].is_null())
        .expect("activador en M-1")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let d2 = app
        .delete_with_cookie(&format!("/v1/transactions/{act_id}"), &owner.cookie)
        .await;
    assert_eq!(d2.status, http::StatusCode::NO_CONTENT);
    assert_eq!(
        app.count_rows("transactions").await,
        1,
        "sin movimientos reales el mes queda vacío: solo sobrevive el origen (exento)"
    );
}

#[tokio::test]
async fn delete_rule_keeps_instances_and_nullifies_link() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);
    activate_month_raw(&app, owner.user_id, m1y, m1m).await;

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 1), "concept": "Cuota", "amount": "-50",
                    "kind": "expense", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    let rule_id = r.json()["recurring_rule_id"].as_str().unwrap().to_string();
    materialize(&app, &owner.cookie).await;
    // origen(M-2) + activador(M-1) + instancia(M-1) = 3; el mes en curso fuera.
    assert_eq!(app.count_rows("transactions").await, 3);

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
        3,
        "instancias conservadas"
    );
    let still_linked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE recurring_rule_id IS NOT NULL")
            .fetch_one(&app.pool)
            .await
            .expect("count linked");
    assert_eq!(still_linked, 0, "FK SET NULL: sin enlaces colgantes");

    // Y sin plantilla no hay nada que converger: las huérfanas no se podan ni se recrean.
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.json()["rules_processed"].as_u64().unwrap(), 0);
    assert_eq!(mat.json()["materialized"].as_u64().unwrap(), 0);
    assert_eq!(mat.json()["pruned"].as_u64().unwrap(), 0);
    assert_eq!(app.count_rows("transactions").await, 3);

    let rules = app
        .get_with_cookie("/v1/transactions/recurring", &owner.cookie)
        .await;
    assert_eq!(rules.json().as_array().unwrap().len(), 0, "plantilla borrada");
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
    keep_only_origin_instance(&app, &rule_id, oy, om).await;

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
async fn cross_user_isolation_survives_installation_wide_convergence() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app
        .register_and_approve_member(&owner, "bob", "member")
        .await;
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);
    let (m1y, m1m) = shift_month(today.year(), today.month(), -1);

    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(oy, om, 10), "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    let alice_rule = r.json()["recurring_rule_id"].as_str().unwrap().to_string();
    keep_only_origin_instance(&app, &alice_rule, oy, om).await;

    // Bob no ve las reglas de Alice (el listado es own-user, sin `?view`).
    let bl = app
        .get_with_cookie("/v1/transactions/recurring", &bob.cookie)
        .await;
    assert_eq!(bl.json().as_array().unwrap().len(), 0, "bob no ve reglas de alice");

    // Bob no puede borrar la regla de Alice → 404.
    let bd = app
        .delete_with_cookie(
            &format!("/v1/transactions/recurring/{alice_rule}"),
            &bob.cookie,
        )
        .await;
    assert_eq!(bd.status, http::StatusCode::NOT_FOUND, "cross-user delete");

    // CAMBIO DE CONTRATO (3.9.0): la convergencia es de ámbito INSTALACIÓN, así que el
    // materialize de BOB sí procesa la regla de Alice. Es deliberado — la activación de un mes
    // también es de instalación, y con ámbito por owner un conviviente que solo tiene nómina
    // recurrente y no importa CSV se quedaría sin ella para siempre.
    let m1_activator_owner = bob.user_id; // el movimiento real lo mete BOB
    activate_month_raw(&app, m1_activator_owner, m1y, m1m).await;
    let bm = materialize(&app, &bob.cookie).await;
    assert_eq!(bm.status, http::StatusCode::OK);
    assert_eq!(
        bm.json()["rules_processed"].as_u64().unwrap(),
        1,
        "ámbito instalación: bob procesa también la regla de alice"
    );
    assert_eq!(
        bm.json()["materialized"].as_u64().unwrap(),
        1,
        "el mes que activó bob materializa la recurrente de alice"
    );

    // Pero la instancia creada es DE ALICE: el scoping per-owner no se rompe.
    let bob_mine = app
        .get_with_cookie(
            &format!("/v1/transactions?month={m1y:04}-{m1m:02}&view=mine"),
            &bob.cookie,
        )
        .await;
    let bob_rows = bob_mine.json();
    assert_eq!(
        bob_rows.as_array().unwrap().len(),
        1,
        "bob solo ve su propio activador: {bob_rows:?}"
    );
    assert!(bob_rows[0]["recurring_rule_id"].is_null());

    let alice_mine = app
        .get_with_cookie(
            &format!("/v1/transactions?month={m1y:04}-{m1m:02}&view=mine"),
            &owner.cookie,
        )
        .await;
    let alice_rows = alice_mine.json();
    assert_eq!(alice_rows.as_array().unwrap().len(), 1, "alice ve su instancia");
    assert_eq!(
        alice_rows[0]["recurring_rule_id"].as_str().unwrap(),
        alice_rule
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

// ---------------------------------------------------------------------------
// 4.15.0 — la plantilla también lleva categoría
// ---------------------------------------------------------------------------

/// Una plantilla recurrente creada sin categoría cae en la POR DEFECTO de su clase —igual que la
/// backfillea la migración `20260902120000` para las que ya existían— y **cada instancia que
/// materializa la hereda**.
///
/// Es la mitad que faltaba: `materialize_rule` inserta `rule.category_id` tal cual, así que una
/// plantilla con `NULL` habría reventado contra `transactions_category_required_check` una vez al
/// mes, en un pase de convergencia post-commit que corre en best-effort y se traga los errores en
/// un `tracing::warn!` — el peor sitio posible para descubrirlo.
#[tokio::test]
async fn materializing_a_legacy_rule_after_the_backfill_keeps_the_fallback() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let otros_gastos = fallback_category(&app, "expense").await;
    let today = server_today(&app, &owner.cookie).await;
    let (py, pm) = shift_month(today.year(), today.month(), -1);

    // La plantilla nace SIN categoría en la petición → el servidor la resuelve al cajón.
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date_in(py, pm, 5), "concept": "Cuota gimnasio",
                    "amount": "-40", "kind": "expense", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create: {r:?}");
    assert_eq!(
        r.json()["category_id"],
        json!(otros_gastos.to_string()),
        "el movimiento de origen ya nace en el cajón: {}",
        r.json()
    );

    let rule_cat: Option<Uuid> =
        sqlx::query_scalar("SELECT category_id FROM recurring_transaction_rules")
            .fetch_one(&app.pool)
            .await
            .expect("plantilla");
    assert_eq!(rule_cat, Some(otros_gastos), "la plantilla también");

    // El mes de origen ya está activo (lo activa su propia instancia), así que materializar no
    // añade nada nuevo ahí; se activa el mes siguiente para que sí haya trabajo.
    activate_month_raw(&app, owner.user_id, today.year(), today.month()).await;
    let (ny, nm) = shift_month(py, pm, 1);
    if (ny, nm) != (today.year(), today.month()) {
        activate_month_raw(&app, owner.user_id, ny, nm).await;
    }
    let mat = materialize(&app, &owner.cookie).await;
    assert_eq!(mat.status, http::StatusCode::OK, "materialize: {mat:?}");

    // Toda instancia de la plantilla lleva la categoría de la plantilla; ninguna queda a NULL.
    let cats: Vec<Option<Uuid>> = sqlx::query_scalar(
        "SELECT category_id FROM transactions WHERE recurring_rule_id IS NOT NULL",
    )
    .fetch_all(&app.pool)
    .await
    .expect("instancias");
    assert!(!cats.is_empty(), "debe haber al menos la instancia de origen");
    for c in cats {
        assert_eq!(c, Some(otros_gastos), "instancia sin la categoría de la plantilla");
    }
}
