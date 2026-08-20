//! Contrato de cache de proyección condicionado al modo `savings_source`:
//! - Modo A (`budget`, default): las transacciones NO son inputs del engine → **ninguna** mutación
//!   invalida la cache (contrato histórico, espejo de `snapshot_mutations_do_not_touch_...`).
//! - Modo B (`transactions_avg`): las transacciones SÍ son inputs del engine → **cada** mutación que
//!   cambia el conjunto (create, batch, patch, delete, import confirm, delete import, materialize)
//!   invalida; borrar una regla recurrente NO (sus instancias sobreviven).
//! - Modo C (`budget_income_real_expense`): paridad con B — create/patch/delete invalidan.
//! - Flip A↔B vía PATCH /v1/installation invalida (el propio PATCH de installation refresca).
//! - Crear una **regla de categorización** no invalida en NINGÚN modo: solo hace INSERT y afecta a
//!   imports futuros (el backfill retroactivo es otra ruta, con su propia invalidación COND).

mod common;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{Datelike, NaiveDate};
use common::TestApp;
use serde_json::json;
use uuid::Uuid;

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
}

/// "Hoy" del servidor (anchor de history) para fechas relativas independientes del reloj.
async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let r = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(r.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn set_mode(app: &TestApp, cookie: &str, source: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": source } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode {source}: {r:?}");
}

/// Import CSV MyInvestor de una sola fila; devuelve `(file_b64, file_sha256)` listos para confirm.
async fn preview_csv(app: &TestApp, cookie: &str, concept: &str, amount: &str, day: u32) -> (String, String) {
    let csv = format!(
        "Fecha de operación;Fecha de valor;Concepto;Importe;Divisa\n\
         {day:02}/06/2026;{day:02}/06/2026;{concept};{amount};EUR\n"
    );
    let b64 = B64.encode(csv);
    let p = app
        .post_json_with_cookie(
            "/v1/transactions/import/preview",
            json!({ "source": "myinvestor", "file_b64": b64 }),
            cookie,
        )
        .await;
    let sha = p.json()["file_sha256"].as_str().unwrap().to_string();
    (b64, sha)
}

// ---------------------------------------------------------------------------
// Modo A (default `budget`): NINGUNA mutación invalida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_a_mutations_do_not_touch_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;

    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    app.warm_household(&owner.cookie, &key).await;

    // --- Batería de mutaciones de transacciones (modo A, default) ---
    // 1. Alta manual.
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-10", "concept": "Manual", "amount": "-25", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    let txn_id = created.json()["id"].as_str().unwrap().to_string();

    // 2. Batch.
    app.post_json_with_cookie(
        "/v1/transactions/batch",
        json!({ "transactions": [
            { "op_date": "2026-06-11", "concept": "Lote", "amount": "-5", "kind": "expense" }
        ] }),
        &owner.cookie,
    )
    .await;

    // 3. Import CSV (preview + confirm).
    let (b64, sha) = preview_csv(&app, &owner.cookie, "IMPORTADA", "-9", 15).await;
    let conf = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                    "decisions": [ { "kind": "expense" } ], "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    let import_id = conf.json()["import_id"].as_str().unwrap().to_string();

    // 4. PATCH + 5. delete_import + 6. DELETE de la manual.
    app.patch_json_with_cookie(
        &format!("/v1/transactions/{txn_id}"),
        json!({ "notes": "editada" }),
        &owner.cookie,
    )
    .await;
    app.delete_with_cookie(&format!("/v1/transactions/imports/{import_id}?confirm=true"), &owner.cookie)
        .await;
    app.delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie).await;

    // 7. Alta con recurrencia + 8. materialize + 9. borrado de la regla.
    let rec = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-15", "concept": "Nomina", "amount": "1500",
                    "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    let rule_id = rec.json()["recurring_rule_id"].as_str().unwrap().to_string();
    app.post_json_with_cookie("/v1/transactions/recurring/materialize", json!({}), &owner.cookie)
        .await;
    app.delete_with_cookie(&format!("/v1/transactions/recurring/{rule_id}"), &owner.cookie)
        .await;

    assert!(
        app.cache_contains(&key).await,
        "modo A: las mutaciones de transacciones NO deben invalidar la cache (no son inputs del engine)"
    );
}

// ---------------------------------------------------------------------------
// Modo B (`transactions_avg`): CADA mutación invalida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_b_each_mutation_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    set_mode(&app, &owner.cookie, "transactions_avg").await;

    let iid = app.installation_id().await;
    let key = app.household_key(iid);

    // 1. create
    app.warm_household(&owner.cookie, &key).await;
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-05-10", "concept": "Manual", "amount": "-25", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    let txn_id = created.json()["id"].as_str().unwrap().to_string();
    app.assert_invalidated(&key, "create").await;

    // 2. batch
    app.warm_household(&owner.cookie, &key).await;
    app.post_json_with_cookie(
        "/v1/transactions/batch",
        json!({ "transactions": [
            { "op_date": "2026-05-11", "concept": "Lote", "amount": "-5", "kind": "expense" }
        ] }),
        &owner.cookie,
    )
    .await;
    app.assert_invalidated(&key, "batch").await;

    // 3. patch
    app.warm_household(&owner.cookie, &key).await;
    app.patch_json_with_cookie(
        &format!("/v1/transactions/{txn_id}"),
        json!({ "notes": "editada" }),
        &owner.cookie,
    )
    .await;
    app.assert_invalidated(&key, "patch").await;

    // 4. import confirm
    app.warm_household(&owner.cookie, &key).await;
    let (b64, sha) = preview_csv(&app, &owner.cookie, "IMPORTADA", "-9", 15).await;
    let conf = app
        .post_json_with_cookie(
            "/v1/transactions/import/confirm",
            json!({ "source": "myinvestor", "file_b64": b64, "file_sha256": sha,
                    "decisions": [ { "kind": "expense" } ], "learn_rules": false }),
            &owner.cookie,
        )
        .await;
    let import_id = conf.json()["import_id"].as_str().unwrap().to_string();
    app.assert_invalidated(&key, "import confirm").await;

    // 5. delete import (cascadea sus transacciones)
    app.warm_household(&owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/imports/{import_id}?confirm=true"), &owner.cookie)
        .await;
    app.assert_invalidated(&key, "delete import").await;

    // 6. delete
    app.warm_household(&owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie).await;
    app.assert_invalidated(&key, "delete").await;

    // 7. materialize (regla con cursor un par de meses atrás → genera ≥1 instancia). Fechas
    // relativas al "hoy" del servidor para que el assert no dependa del reloj de la máquina.
    let today = server_today(&app, &owner.cookie).await;
    let (oy, om) = shift_month(today.year(), today.month(), -2);
    let op_date = NaiveDate::from_ymd_opt(oy, om, 15).unwrap();
    let rec = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": op_date.format("%Y-%m-%d").to_string(), "concept": "Nomina",
                    "amount": "1500", "kind": "income", "recurrence": {} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(rec.status, http::StatusCode::CREATED);
    let rule_id = rec.json()["recurring_rule_id"].as_str().unwrap().to_string();
    // El alta con fecha pasada ya backfillea los meses cerrados; rebobinamos por SQL (cursor a un mes previo
    // al origen, sin instancias) para que el ENDPOINT materialize vuelva a generar ≥1 y podamos
    // probar que invalida en modo B.
    let rid = Uuid::parse_str(&rule_id).unwrap();
    sqlx::query("DELETE FROM transactions WHERE recurring_rule_id = $1")
        .bind(rid)
        .execute(&app.pool)
        .await
        .expect("clear rule instances");
    let (cy, cm) = shift_month(oy, om, -1);
    let cursor = NaiveDate::from_ymd_opt(cy, cm, 1).unwrap();
    sqlx::query("UPDATE recurring_transaction_rules SET last_materialized_month = $1 WHERE id = $2")
        .bind(cursor)
        .bind(rid)
        .execute(&app.pool)
        .await
        .expect("rewind cursor");
    app.warm_household(&owner.cookie, &key).await;
    let mat = app
        .post_json_with_cookie("/v1/transactions/recurring/materialize", json!({}), &owner.cookie)
        .await;
    assert!(mat.json()["materialized"].as_u64().unwrap() >= 1, "materialize debe generar ≥1: {mat:?}");
    app.assert_invalidated(&key, "materialize").await;

    // 8. borrar la regla recurrente NO invalida (instancias sobreviven, conjunto sin cambios).
    app.warm_household(&owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/recurring/{rule_id}"), &owner.cookie)
        .await;
    assert!(
        app.cache_contains(&key).await,
        "modo B: borrar una regla recurrente NO cambia el conjunto de transacciones → no debe invalidar"
    );
}

// ---------------------------------------------------------------------------
// Flip A↔B vía PATCH /v1/installation invalida
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flipping_savings_source_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;

    let iid = app.installation_id().await;
    let key = app.household_key(iid);

    // A → B
    app.warm_household(&owner.cookie, &key).await;
    set_mode(&app, &owner.cookie, "transactions_avg").await;
    app.assert_invalidated(&key, "flip A→B").await;

    // B → A
    app.warm_household(&owner.cookie, &key).await;
    set_mode(&app, &owner.cookie, "budget").await;
    app.assert_invalidated(&key, "flip B→A").await;
}

// ---------------------------------------------------------------------------
// Modo C (`budget_income_real_expense`): también usa transacciones → invalida (paridad con B)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_c_mutation_invalidates_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    set_mode(&app, &owner.cookie, "budget_income_real_expense").await;

    let iid = app.installation_id().await;
    let key = app.household_key(iid);

    app.warm_household(&owner.cookie, &key).await;
    let created = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-05-10", "concept": "Manual", "amount": "-25", "kind": "expense" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED);
    app.assert_invalidated(&key, "modo C create").await;
    let txn_id = created.json()["id"].as_str().unwrap().to_string();

    // PATCH: `patch_transaction_core` invalida de forma incondicional (no solo cuando cambian
    // importe/fecha), así que un cambio de categoría/notas también tira la cache en modo C.
    app.warm_household(&owner.cookie, &key).await;
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/transactions/{txn_id}"),
            json!({ "notes": "editada" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK);
    app.assert_invalidated(&key, "modo C patch").await;

    // DELETE: cierra la paridad con el modo B para las tres rutas más usadas desde el chat.
    app.warm_household(&owner.cookie, &key).await;
    app.delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie)
        .await;
    app.assert_invalidated(&key, "modo C delete").await;
}

// ---------------------------------------------------------------------------
// Reglas de categorización: crear una regla NUNCA invalida, en NINGÚN modo
// ---------------------------------------------------------------------------

/// Contrato histórico: `create_categorization_rule_core` solo hace INSERT — no recategoriza nada,
/// así que el conjunto de transacciones no cambia y la cache sigue siendo válida incluso en los
/// modos que sí leen transacciones. Estaba documentado en tres sitios (`rules.rs`,
/// `transactions/mod.rs` y `.claude/tests.md`) como «regresión pinneada», pero ningún test lo
/// ejercía: el test de modo A solo tocaba la regla **recurrente**. Este es el pin que faltaba.
#[tokio::test]
async fn creating_a_categorization_rule_never_invalidates_projection_cache() {
    for mode in ["budget", "transactions_avg", "budget_income_real_expense"] {
        let app = TestApp::spawn().await;
        let owner = app.register_and_login_owner("alice").await;
        let cat = app.create_category(&owner, "asset", "Cash").await;
        app.post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
            &owner.cookie,
        )
        .await;
        let expense_cat = app.create_category(&owner, "expense", "Compras").await;
        set_mode(&app, &owner.cookie, mode).await;

        // Una transacción que la regla PODRÍA recategorizar: si algún día el core hiciera backfill
        // silencioso, este test lo delataría por la vía de la cache.
        app.post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-10", "concept": "WWW.AMAZON* MN34OP56",
                    "amount": "-104.45", "kind": "expense" }),
            &owner.cookie,
        )
        .await;

        let iid = app.installation_id().await;
        let key = app.household_key(iid);
        app.warm_household(&owner.cookie, &key).await;

        let created = app
            .post_json_with_cookie(
                "/v1/transactions/rules",
                json!({ "match_kind": "substring", "pattern": "AMAZON",
                        "assign_kind": "expense", "assign_category_id": expense_cat }),
                &owner.cookie,
            )
            .await;
        assert_eq!(created.status, http::StatusCode::CREATED, "modo {mode}: {created:?}");

        assert!(
            app.cache_contains(&key).await,
            "modo {mode}: crear una regla de categorización NO debe invalidar la cache \
             (solo afecta a imports futuros; no recategoriza nada existente)"
        );
    }
}

/// El **backfill** de una regla sí es una mutación del conjunto: cambia el `kind` (y la categoría)
/// de filas históricas, y `transactions_12m_avg` suma solo `kind IN ('income','expense')`. Por eso
/// invalida COND — al contrario que CREAR la regla, que no toca ninguna fila. Los dos contratos
/// conviven en el mismo módulo y este test los separa.
#[tokio::test]
async fn applying_a_rule_invalidates_cond_but_creating_it_still_does_not() {
    for (mode, should_invalidate) in [
        ("budget", false),
        ("transactions_avg", true),
        ("budget_income_real_expense", true),
    ] {
        let app = TestApp::spawn().await;
        let owner = app.register_and_login_owner("alice").await;
        let cat = app.create_category(&owner, "asset", "Cash").await;
        app.post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
            &owner.cookie,
        )
        .await;
        let compras = app.create_category(&owner, "expense", "Compras").await;
        set_mode(&app, &owner.cookie, mode).await;

        // Fila mal clasificada (income) y sin categoría: el backfill la pasará a expense, que es
        // exactamente lo que mueve el promedio real 12m — `transactions_12m_avg` suma por `kind`.
        let seeded = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({ "op_date": "2026-06-10", "concept": "WWW.AMAZON* MN34OP56",
                        "amount": "-104.45", "kind": "income" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(seeded.status, http::StatusCode::CREATED, "modo {mode}: {seeded:?}");

        let iid = app.installation_id().await;
        let key = app.household_key(iid);

        // 1. Crear la regla: NUNCA invalida, en ningún modo.
        app.warm_household(&owner.cookie, &key).await;
        let rule = app
            .post_json_with_cookie(
                "/v1/transactions/rules",
                json!({ "match_kind": "substring", "pattern": "AMAZON",
                        "assign_kind": "expense", "assign_category_id": compras }),
                &owner.cookie,
            )
            .await;
        assert_eq!(rule.status, http::StatusCode::CREATED, "modo {mode}: {rule:?}");
        let rule_id = rule.json()["id"].as_str().unwrap().to_string();
        assert!(
            app.cache_contains(&key).await,
            "modo {mode}: crear la regla no debe invalidar"
        );

        // 2. El PREVIEW tampoco: no escribe nada.
        let preview = app
            .post_json_with_cookie(
                &format!("/v1/transactions/rules/{rule_id}/apply"),
                json!({ "apply_to_existing": "all" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(preview.status, http::StatusCode::BAD_REQUEST, "sin confirm por HTTP");
        assert!(app.cache_contains(&key).await, "modo {mode}: el preview no invalida");

        // 3. El backfill sí, y solo en los modos que leen transacciones.
        let applied = app
            .post_json_with_cookie(
                &format!("/v1/transactions/rules/{rule_id}/apply"),
                json!({ "apply_to_existing": "all", "confirm": true }),
                &owner.cookie,
            )
            .await;
        assert_eq!(applied.status, http::StatusCode::OK, "modo {mode}: {applied:?}");
        assert_eq!(applied.json()["matched"], 1, "modo {mode}: {:?}", applied.json());
        assert_eq!(
            applied.json()["would_change_kind"], 1,
            "modo {mode}: el cambio de kind es la señal de que la proyección se mueve"
        );

        if should_invalidate {
            app.assert_invalidated(&key, &format!("modo {mode} backfill")).await;
        } else {
                assert!(
                app.cache_contains(&key).await,
                "modo A: las transacciones no son inputs del engine, tampoco tras un backfill"
            );
        }

        // 4. Un backfill que no cambia nada (idempotente) no debe tirar la cache caliente.
        app.warm_household(&owner.cookie, &key).await;
        let again = app
            .post_json_with_cookie(
                &format!("/v1/transactions/rules/{rule_id}/apply"),
                json!({ "apply_to_existing": "all", "confirm": true }),
                &owner.cookie,
            )
            .await;
        assert_eq!(again.json()["matched"], 0, "modo {mode}: {:?}", again.json());
        assert!(
            app.cache_contains(&key).await,
            "modo {mode}: un backfill sin filas afectadas no debe invalidar"
        );
    }
}

/// Contrato de cache del PATCH en lote: **COND**, igual que el PATCH individual — invalida en los
/// modos que leen transacciones y no en modo A.
///
/// Lo que motivó el lote fue el coste: 16 recategorizaciones seguidas en modo C tiraban la cache 16
/// veces. Que la invalidación ocurra **una sola vez** es una propiedad estructural de la core (la
/// llamada está fuera del bucle, después del commit) y no es observable desde fuera: el estado
/// final de la cache es el mismo tirándola una vez que cinco. Este test fija lo que sí se puede
/// observar —que invalida, y solo en los modos correctos—; la unicidad la sostiene la revisión del
/// código, no una aserción que no podría fallar.
#[tokio::test]
async fn batch_patch_invalidates_once_for_the_whole_batch() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    let compras = app.create_category(&owner, "expense", "Compras").await;
    set_mode(&app, &owner.cookie, "budget_income_real_expense").await;

    let mut ids = Vec::new();
    for i in 0..5 {
        let r = app
            .post_json_with_cookie(
                "/v1/transactions",
                json!({ "op_date": format!("2026-06-{:02}", 10 + i),
                        "concept": format!("COMPRA {i}"), "amount": "-10", "kind": "income" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
        ids.push(r.json()["id"].as_str().unwrap().to_string());
    }

    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    app.warm_household(&owner.cookie, &key).await;

    let r = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": ids, "kind": "expense", "category_id": compras }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["updated"], 5, "{:?}", r.json());
    app.assert_invalidated(&key, "batch patch en modo C").await;

    // Y en modo A sigue sin invalidar: el contrato es COND, no incondicional.
    set_mode(&app, &owner.cookie, "budget").await;
    app.warm_household(&owner.cookie, &key).await;
    let r = app
        .patch_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "ids": ids, "notes": "revisado" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(
        app.cache_contains(&key).await,
        "modo A: el lote tampoco invalida (las transacciones no son inputs del engine)"
    );
}

// ---------------------------------------------------------------------------
// Conciliación (3.5.0): mismas reglas COND que el resto de mutaciones
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_a_reconcile_endpoints_do_not_touch_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;

    // Par a >5 días para que el pase automático NO lo enlace: la conciliación será manual.
    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-01", "concept": "Salida", "amount": "-100", "kind": "expense" }),
            &owner.cookie,
        )
        .await
        .json();
    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-20", "concept": "Entrada", "amount": "100", "kind": "income" }),
            &owner.cookie,
        )
        .await
        .json();
    let (a_id, b_id) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());

    let iid = app.installation_id().await;
    let key = app.household_key(iid);
    app.warm_household(&owner.cookie, &key).await;

    // Conciliar par + desconciliar + pase explícito: en modo A nada invalida.
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{a_id}/reconcile"),
            json!({ "counterpart_id": b_id }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "pair: {r:?}");
    let u = app
        .delete_with_cookie(&format!("/v1/transactions/{a_id}/reconcile"), &owner.cookie)
        .await;
    assert_eq!(u.status, http::StatusCode::OK, "unreconcile: {u:?}");
    let p = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "pass: {p:?}");

    assert!(
        app.cache_contains(&key).await,
        "modo A: conciliar/desconciliar NO debe invalidar (las transacciones no son inputs)"
    );
}

#[tokio::test]
async fn mode_b_reconcile_endpoints_invalidate_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        json!({ "category_id": cat, "name": "X", "current_value": "10000" }),
        &owner.cookie,
    )
    .await;
    set_mode(&app, &owner.cookie, "transactions_avg").await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-01", "concept": "Salida", "amount": "-100", "kind": "expense" }),
            &owner.cookie,
        )
        .await
        .json();
    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": "2026-06-20", "concept": "Entrada", "amount": "100", "kind": "income" }),
            &owner.cookie,
        )
        .await
        .json();
    let (a_id, b_id) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap());

    let iid = app.installation_id().await;
    let key = app.household_key(iid);

    // 1. Conciliar par manual → invalida (cambia qué cuenta en el promedio 12m).
    app.warm_household(&owner.cookie, &key).await;
    let r = app
        .post_json_with_cookie(
            &format!("/v1/transactions/{a_id}/reconcile"),
            json!({ "counterpart_id": b_id }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "pair: {r:?}");
    app.assert_invalidated(&key, "reconcile pair").await;

    // 2. Desconciliar → invalida.
    app.warm_household(&owner.cookie, &key).await;
    let u = app
        .delete_with_cookie(&format!("/v1/transactions/{a_id}/reconcile"), &owner.cookie)
        .await;
    assert_eq!(u.status, http::StatusCode::OK, "unreconcile: {u:?}");
    app.assert_invalidated(&key, "unreconcile").await;

    // 3. Pase explícito sin pares nuevos (el rechazo bloquea el re-emparejado) → NO invalida.
    app.warm_household(&owner.cookie, &key).await;
    let p = app
        .post_json_with_cookie("/v1/transactions/reconcile", json!({}), &owner.cookie)
        .await;
    assert_eq!(p.json()["pairs_created"].as_u64(), Some(0), "sin pares nuevos: {p:?}");
    assert!(
        app.cache_contains(&key).await,
        "modo B: un pase que no enlaza nada no debe tirar la cache caliente"
    );
}
