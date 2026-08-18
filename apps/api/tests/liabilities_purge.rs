//! Fase 1.1 — La purga de pasivos vencidos debe salir de los GETs.
//!
//! Contrato esperado tras el cambio:
//! - `GET /v1/liabilities` filtra por `payment_end_date IS NULL OR >= today`, pero no borra filas.
//! - `GET /v1/summary` no incluye principales de pasivos vencidos en `total_liabilities`
//!   ni en los breakdowns de categoría/type_tag.
//! - Las filas vencidas siguen existiendo en BD (no hay mutación en lecturas).

mod common;

use chrono::{Duration, Utc};
use common::TestApp;

#[tokio::test]
async fn expired_liability_is_hidden_from_listing_but_persists_in_db() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_id = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = Utc::now().date_naive();
    let past = (today - Duration::days(5)).format("%Y-%m-%d").to_string();
    let future = (today + Duration::days(30)).format("%Y-%m-%d").to_string();

    // Pasivo vencido (con plan de pago acabado).
    let expired = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_id,
                "expense_category_id": exp_cat,
                "label": "Tarjeta cerrada",
                "principal": "1000",
                "payment_amount": "100",
                "payment_frequency": "monthly",
                "payment_end_date": past,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        expired.status,
        http::StatusCode::CREATED,
        "expired liability create failed: {expired:?}"
    );

    // Pasivo activo (plan futuro).
    let active = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_id,
                "expense_category_id": exp_cat,
                "label": "Hipoteca",
                "principal": "50000",
                "payment_amount": "300",
                "payment_frequency": "monthly",
                "payment_end_date": future,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        active.status,
        http::StatusCode::CREATED,
        "active liability create failed: {active:?}"
    );

    // Antes de listar: ambos existen en BD.
    assert_eq!(app.count_rows("liabilities").await, 2);

    let list = app.get_with_cookie("/v1/liabilities", &owner.cookie).await;
    assert_eq!(list.status, http::StatusCode::OK);
    let body = list.json();
    let arr = body.as_array().expect("liabilities list is an array");
    assert_eq!(arr.len(), 1, "solo el pasivo activo debe aparecer en la lista");
    assert_eq!(arr[0]["label"], "Hipoteca");

    // ⚠️ Tras la Fase 1.1, las filas vencidas deben PERSISTIR. Hoy el handler las purga.
    let remaining = app.count_rows("liabilities").await;
    assert_eq!(
        remaining, 2,
        "GET /v1/liabilities no debe borrar filas vencidas (hoy fallará: el handler purga)"
    );
}

#[tokio::test]
async fn summary_excludes_expired_liability_principal_and_breakdown() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bob").await;
    let cat_id = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = Utc::now().date_naive();
    let past = (today - Duration::days(2)).format("%Y-%m-%d").to_string();
    let future = (today + Duration::days(60)).format("%Y-%m-%d").to_string();

    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": cat_id,
                "expense_category_id": exp_cat,
            "label": "Vencido",
            "principal": "9999",
            "payment_amount": "100",
            "payment_frequency": "monthly",
            "payment_end_date": past,
        }),
        &owner.cookie,
    )
    .await;
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": cat_id,
                "expense_category_id": exp_cat,
            "label": "Activo",
            "principal": "100",
            "payment_amount": "10",
            "payment_frequency": "monthly",
            "payment_end_date": future,
        }),
        &owner.cookie,
    )
    .await;

    let summary = app.get_with_cookie("/v1/summary", &owner.cookie).await;
    assert_eq!(summary.status, http::StatusCode::OK);
    let body = summary.json();

    let total: f64 = body["total_liabilities"]
        .as_str()
        .expect("total_liabilities is string")
        .parse()
        .expect("parse decimal");
    assert!(
        (total - 100.0).abs() < 0.001,
        "total_liabilities solo debe sumar el pasivo activo (esperado ≈ 100, recibido {total})"
    );

    let breakdown = body["liabilities_by_category"]
        .as_array()
        .expect("liabilities_by_category es array");
    let prestamo_total: f64 = breakdown
        .iter()
        .find(|row| row["category_name"] == "Préstamo")
        .and_then(|row| row["total"].as_str())
        .expect("Préstamo en breakdown")
        .parse()
        .expect("parse breakdown total");
    assert!(
        (prestamo_total - 100.0).abs() < 0.001,
        "breakdown por categoría no debe incluir el principal vencido (esperado ≈ 100, recibido {prestamo_total})"
    );

    assert_eq!(
        app.count_rows("liabilities").await,
        2,
        "GET /v1/summary no debe borrar filas vencidas"
    );
}

/// Reforma 3.4.0 (fix C-10): la proyección también filtra pasivos vencidos. Antes
/// `build_installation_projection_input` cargaba TODOS los pasivos del scope y el engine restaba
/// su principal del net worth en cada mes — `projection.starting_net_worth` divergía de
/// `summary.net_worth` exactamente en el principal vencido (contra el contrato D5/I5).
#[tokio::test]
async fn projection_excludes_expired_liability_principal() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("carol").await;
    let cat_id = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = Utc::now().date_naive();
    let past = (today - Duration::days(30)).format("%Y-%m-%d").to_string();
    let future = (today + Duration::days(365 * 5)).format("%Y-%m-%d").to_string();

    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": cat_id,
                "expense_category_id": exp_cat,
            "label": "Vencido",
            "principal": "9999",
            "payment_amount": "100",
            "payment_frequency": "monthly",
            "payment_end_date": past,
        }),
        &owner.cookie,
    )
    .await;
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": cat_id,
                "expense_category_id": exp_cat,
            "label": "Activo",
            "principal": "50000",
            "payment_amount": "300",
            "payment_frequency": "monthly",
            "payment_end_date": future,
        }),
        &owner.cookie,
    )
    .await;

    // Sin activos: summary.net_worth = −50000 (solo el principal activo).
    let summary = app.get_with_cookie("/v1/summary", &owner.cookie).await;
    assert_eq!(summary.status, http::StatusCode::OK);
    let summary_nw: f64 = summary.json()["net_worth"]
        .as_str()
        .expect("net_worth is string")
        .parse()
        .expect("parse net_worth");

    // `?months=240` esquiva la cache (solo cachea sin `months`): siempre estado fresco.
    let proj = app
        .get_with_cookie("/v1/projection/series?months=240", &owner.cookie)
        .await;
    assert_eq!(proj.status, http::StatusCode::OK);
    let body = proj.json();
    let starting: f64 = body["starting_net_worth"]
        .as_str()
        .expect("starting_net_worth is string")
        .parse()
        .expect("parse starting_net_worth");

    assert!(
        (summary_nw - (-50_000.0)).abs() < 0.001,
        "summary.net_worth solo debe restar el principal activo, got {summary_nw}"
    );
    assert!(
        (starting - summary_nw).abs() < 0.001,
        "projection.starting_net_worth ({starting}) debe coincidir con summary.net_worth ({summary_nw}) — el principal vencido no puede lastrar la serie"
    );

    // Y en TODA la serie el vencido sigue sin aparecer: con presupuesto vacío el delta es 0 y el
    // único movimiento del NW es la amortización del pasivo activo (modo A) — nunca los −9999.
    let points = body["points"].as_array().expect("points");
    let first_nw = points[0]["net_worth"].as_f64().expect("net_worth f64");
    assert!(
        (first_nw - starting).abs() < 0.001,
        "points[0] debe arrancar en starting_net_worth, got {first_nw}"
    );
}

/// 3.4.0: `expense_category_id` es obligatoria al crear (la categoría de gasto donde vive la
/// cuota) y debe ser de scope `expense`. Los pasivos legacy con NULL solo existen pre-migración.
#[tokio::test]
async fn liability_create_requires_expense_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("dave").await;
    let cat_id = app.create_category(&owner, "liability", "Préstamo").await;

    // Sin el campo → rechazo del deserializador (4xx, nunca 201).
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_id,
                "label": "Sin categoría de cuota",
                "principal": "1000",
            }),
            &owner.cookie,
        )
        .await;
    assert!(
        r.status.is_client_error(),
        "crear sin expense_category_id debe fallar, got {:?}",
        r.status
    );

    // Con scope equivocado (liability) → 400 con mensaje claro.
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_id,
                "expense_category_id": cat_id,
                "label": "Scope equivocado",
                "principal": "1000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");

    assert_eq!(app.count_rows("liabilities").await, 0, "nada creado");
}

/// Borrado/remap de la categoría de gasto de la cuota: el remap la sigue (misma transacción que
/// el resto de tablas); un borrado sin otras referencias no se bloquea (SET NULL, como las
/// categorization_rules) y degrada la atribución a «sin asignar».
#[tokio::test]
async fn expense_category_remap_and_set_null() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("erin").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    let cat_a = app.create_category(&owner, "expense", "Vivienda A").await;
    let cat_b = app.create_category(&owner, "expense", "Vivienda B").await;

    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": liab_cat,
                "expense_category_id": cat_a,
                "label": "Hipoteca",
                "principal": "50000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let liab_id = r.json()["id"].as_str().unwrap().to_string();

    // cat_a tiene una referencia CONTADA (transacción) → el borrado exige remap_to.
    let t = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({ "op_date": "2026-01-10", "concept": "Recibo", "amount": "-500",
                                "kind": "expense", "category_id": cat_a }),
            &owner.cookie,
        )
        .await;
    assert_eq!(t.status, http::StatusCode::CREATED, "{t:?}");

    let del = app
        .delete_with_cookie(
            &format!("/v1/categories/{cat_a}?remap_to={cat_b}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT, "remap: {del:?}");

    let liabs = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    let row = liabs
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == liab_id.as_str())
        .expect("liability");
    assert_eq!(
        row["expense_category_id"].as_str().unwrap(),
        cat_b,
        "el remap debe arrastrar expense_category_id"
    );

    // cat_b ahora solo la referencian la transacción remapeada... y el pasivo (que NO cuenta).
    // Borra la transacción para dejar a cat_b sin referencias contadas → borrado directo → SET NULL.
    let txns = app
        .get_with_cookie("/v1/transactions?month=2026-01", &owner.cookie)
        .await
        .json();
    let txn_id = txns["items"]
        .as_array()
        .or_else(|| txns.as_array())
        .and_then(|a| a.first())
        .and_then(|t| t["id"].as_str())
        .expect("txn id")
        .to_string();
    let dtx = app
        .delete_with_cookie(&format!("/v1/transactions/{txn_id}"), &owner.cookie)
        .await;
    assert!(dtx.status.is_success(), "delete txn: {dtx:?}");

    let del_b = app
        .delete_with_cookie(&format!("/v1/categories/{cat_b}"), &owner.cookie)
        .await;
    assert_eq!(
        del_b.status,
        http::StatusCode::NO_CONTENT,
        "el pasivo (SET NULL) no debe bloquear el borrado: {del_b:?}"
    );

    let liabs = app.get_with_cookie("/v1/liabilities", &owner.cookie).await.json();
    let row = liabs
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == liab_id.as_str())
        .expect("liability");
    assert!(
        row.get("expense_category_id").is_none(),
        "borrar la categoría degrada la atribución a NULL (campo omitido): {row:?}"
    );
}
