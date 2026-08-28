//! Integración de la serie histórica interpolada (`GET /v1/history/series`) — WP4.
//!
//! Los valores por punto viajan como **f64** (excepción D4/I3, misma que los arrays de
//! `/v1/projection/series`): se comparan con epsilon relativo 1e-6 porque cruzaron
//! Decimal→f64. Cada test PREDICE sus números en comentarios antes de asertar; las
//! predicciones se calculan desde el `anchor_date_ymd` que devuelve el propio endpoint
//! (el hoy civil del servidor), no desde suposiciones sobre el reloj local.

mod common;

use chrono::{Datelike, Duration, NaiveDate, Utc};
use common::{LoggedInOwner, TestApp};
use futurefin_engine::{add_months_signed, month_index_of};
use uuid::Uuid;

/// Epsilon relativo 1e-6 (los valores cruzaron Decimal→f64) **con un suelo absoluto de 0,005**:
/// desde 4.4.0 las series de chart se publican redondeadas a 2 decimales (`CHART_DP` en
/// `handlers/history.rs`), así que ningún valor puede coincidir con la predicción más allá de medio
/// céntimo. El suelo es exactamente la mitad del último decimal publicado — más laxo aceptaría un
/// error de redondeo real, más estricto exigiría una precisión que la respuesta ya no promete.
fn assert_close(got: f64, expected: f64, ctx: &str) {
    let tol = (1e-6_f64 * expected.abs().max(1.0)).max(0.005);
    assert!(
        (got - expected).abs() <= tol,
        "{ctx}: got {got}, expected {expected} (tol {tol})"
    );
}

/// Igual, para `month_fraction`, que se publica a 4 decimales (`MONTH_FRACTION_DP`): medio
/// diezmilésimo.
fn assert_close_fraction(got: f64, expected: f64, ctx: &str) {
    let tol = 0.00005_f64;
    assert!(
        (got - expected).abs() <= tol,
        "{ctx}: got {got}, expected {expected} (tol {tol})"
    );
}

fn ymd(v: &serde_json::Value) -> NaiveDate {
    NaiveDate::parse_from_str(v.as_str().expect("ymd string"), "%Y-%m-%d").expect("parse ymd")
}

fn f64_of(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("expected f64, got {v:?}"))
}

fn i32_of(v: &serde_json::Value) -> i32 {
    v.as_i64().unwrap_or_else(|| panic!("expected int, got {v:?}")) as i32
}

/// `points[].net_worth`: `None` cuando el pasivo del scope no está fotografiado entero.
/// El campo NUNCA se omite (viaja como `null` explícito), así que la ausencia de la clave
/// es un fallo, no un `None`.
fn nw_of(p: &serde_json::Value) -> Option<f64> {
    let v = p
        .get("net_worth")
        .unwrap_or_else(|| panic!("net_worth debe estar SIEMPRE presente, aunque sea null: {p}"));
    if v.is_null() {
        None
    } else {
        Some(f64_of(v))
    }
}

/// Todos los puntos con `net_worth = null` (el contrato es «en TODA la serie», no punto a punto).
fn assert_all_nw_null(body: &serde_json::Value, ctx: &str) {
    assert_eq!(
        body["liabilities_snapshotted"],
        serde_json::json!(false),
        "{ctx}: se esperaba el flag en false"
    );
    for p in body["points"].as_array().expect("points array") {
        assert!(
            nw_of(p).is_none(),
            "{ctx}: net_worth debe ser null en k={} — {p}",
            i32_of(&p["month_index"])
        );
    }
}

/// GET /v1/history/series → (JSON, anchor_date, anchor_month_first).
async fn get_series(app: &TestApp, cookie: &str, qs: &str) -> (serde_json::Value, NaiveDate, NaiveDate) {
    let resp = app
        .get_with_cookie(&format!("/v1/history/series{qs}"), cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "series GET: {resp:?}");
    let body = resp.json();
    let anchor_date = ymd(&body["anchor_date_ymd"]);
    let anchor_mf = ymd(&body["anchor_month_first_ymd"]);
    (body, anchor_date, anchor_mf)
}

/// Hoy civil del servidor: el endpoint lo devuelve incluso sin snapshots (arrays vacíos).
async fn server_today(app: &TestApp, cookie: &str) -> (NaiveDate, NaiveDate) {
    let (_, today, anchor) = get_series(app, cookie, "").await;
    (today, anchor)
}

/// Punto con `month_index == k` dentro de `points`.
fn point_at(body: &serde_json::Value, k: i32) -> &serde_json::Value {
    body["points"]
        .as_array()
        .expect("points array")
        .iter()
        .find(|p| i32_of(&p["month_index"]) == k)
        .unwrap_or_else(|| panic!("no point with month_index {k} in {body}"))
}

async fn backfill(
    app: &TestApp,
    user: &LoggedInOwner,
    kind: &str,
    date: NaiveDate,
    items: serde_json::Value,
) -> serde_json::Value {
    let resp = app
        .post_json_with_cookie(
            "/v1/history/snapshots",
            serde_json::json!({
                "kind": kind,
                "snapshot_date": date.format("%Y-%m-%d").to_string(),
                "items": items,
            }),
            &user.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::CREATED, "backfill: {resp:?}");
    resp.json()
}

// ---------------------------------------------------------------------------
// 1. Sin snapshots → 200 con arrays vacíos
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_empty_without_snapshots() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    // Filas vivas SIN snapshots: siguen sin producir serie (usuarios sin snapshots no aportan).
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "Bolsa", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;

    // Predicción: 200, view household, points/asset_series/markers = [] (0 snapshots en scope).
    let (body, anchor_date, anchor_mf) = get_series(&app, &owner.cookie, "").await;
    assert_eq!(body["view"], "household");
    assert_eq!(body["points"].as_array().unwrap().len(), 0);
    assert_eq!(body["asset_series"].as_array().unwrap().len(), 0);
    assert_eq!(body["markers"].as_array().unwrap().len(), 0);
    // anchor_month_first = día 1 del mes del hoy civil.
    assert_eq!(anchor_mf, add_months_signed(anchor_date, 0));

    // Predicción: view=mine idéntico (también vacío) con label "mine".
    let (mine, _, _) = get_series(&app, &owner.cookie, "?view=mine").await;
    assert_eq!(mine["view"], "mine");
    assert_eq!(mine["points"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// 2. Lineal exacto entre dos snapshots de assets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_linear_between_two_asset_snapshots() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_, anchor) = server_today(&app, &owner.cookie).await;

    // Backfill 100 → 400 con tres meses de separación, ambos en primero-de-mes: la
    // interpolación (lineal en días civiles) cae EXACTAMENTE en los grid points.
    let d1 = add_months_signed(anchor, -4);
    let d2 = add_months_signed(anchor, -1);
    let item_id = Uuid::new_v4().to_string();
    backfill(&app, &owner, "asset", d1,
        serde_json::json!([{"item_id": item_id, "label": "Cash", "value": "100"}])).await;
    backfill(&app, &owner, "asset", d2,
        serde_json::json!([{"item_id": item_id, "label": "Cash", "value": "400"}])).await;

    let (body, _, anchor_mf) = get_series(&app, &owner.cookie, "").await;
    assert_eq!(anchor_mf, anchor, "anchor no debe moverse durante el test");

    // Predicción de la rejilla: k_min = -4, puntos contiguos -4..=0 (5 puntos).
    let pts = body["points"].as_array().unwrap();
    assert_eq!(pts.len(), 5);
    let ks: Vec<i32> = pts.iter().map(|p| i32_of(&p["month_index"])).collect();
    assert_eq!(ks, vec![-4, -3, -2, -1, 0]);

    // Predicciones exactas (v(g) = 100 + 300·days(d1→g)/days(d1→d2)):
    //   k=-4 (= d1)  → 100 exacto.
    //   k=-3, k=-2  → fracción de días civiles (p. ej. con anchor 2026-07-01:
    //                  d1=2026-03-01, d2=2026-06-01, total 92 días →
    //                  k=-3: 100+300·31/92 = 201.0869…; k=-2: 100+300·61/92 = 298.9130…).
    //   k=-1 (= d2)  → 400 exacto.
    //   k=0          → 0 (sin asset vivo: la observación virtual de hoy no contiene el
    //                  item → observado solo a la izquierda del segmento [d2, hoy] → 0).
    let total_days = (d2 - d1).num_days() as f64;
    let expect = |g: NaiveDate| -> f64 { 100.0 + 300.0 * ((g - d1).num_days() as f64) / total_days };
    let expected = [
        expect(d1),                            // 100
        expect(add_months_signed(anchor, -3)), // fracción de días
        expect(add_months_signed(anchor, -2)), // fracción de días
        expect(d2),                            // 400
        0.0,
    ];
    // `net_worth` NO se publica: solo hay snapshots de asset, así que el pasivo no está
    // fotografiado y la resta sería `assets_total` con nombre de patrimonio neto.
    assert_all_nw_null(&body, "solo snapshots de asset");
    for (p, exp) in pts.iter().zip(expected) {
        let k = i32_of(&p["month_index"]);
        assert_close(f64_of(&p["assets_total"]), exp, &format!("assets_total k={k}"));
        assert_close(f64_of(&p["liabilities_total"]), 0.0, &format!("liabilities k={k}"));
    }

    // asset_series: una única serie, paralela a points, con los mismos valores.
    let series = body["asset_series"].as_array().unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["asset_id"].as_str().unwrap(), item_id);
    assert_eq!(series[0]["asset_name"], "Cash");
    let values = series[0]["values"].as_array().unwrap();
    assert_eq!(values.len(), 5);
    for (v, exp) in values.iter().zip(expected) {
        assert_close(f64_of(v), exp, "asset_series value");
    }

    assert_eq!(body["markers"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// 3. Empalme con los valores vivos; asset borrado → 0 en k=0
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_joins_last_snapshot_to_live_values() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let (today, anchor) = server_today(&app, &owner.cookie).await;

    // Asset vivo a 400; snapshot de hace 2 meses (backfill) a 100 con item_id = id del asset.
    let created = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "Bolsa Global", "current_value": "400"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let asset_id = created.json()["id"].as_str().unwrap().to_string();

    let d1 = add_months_signed(anchor, -2);
    backfill(&app, &owner, "asset", d1,
        serde_json::json!([{"item_id": asset_id, "label": "Bolsa antigua", "value": "100"}])).await;

    let (body, _, _) = get_series(&app, &owner.cookie, "").await;

    // Predicciones (timeline [d1, hoy] con observación virtual de hoy = valor vivo 400;
    // v(g) = 100 + 300·days(d1→g)/days(d1→hoy)):
    //   k=-2 (= d1) → 100 exacto.
    //   k=-1        → interpolado en su primero-de-mes.
    //   k=0         → 400 EXACTO cualquier día del mes. Desde 4.0.0 el punto 0 se evalúa en `today`,
    //                 que ES la fecha de la observación virtual, así que devuelve el valor vivo sin
    //                 interpolar. Antes se evaluaba el día 1 y solo daba 400 si hoy era día 1: el
    //                 resto del mes la serie iba por detrás del patrimonio real, y este test pasaba
    //                 en verde 1 de cada 30 días midiendo el interpolante en vez del empalme.
    let total_days = (today - d1).num_days() as f64;
    let expect = |g: NaiveDate| -> f64 { 100.0 + 300.0 * ((g - d1).num_days() as f64) / total_days };
    let pts = body["points"].as_array().unwrap();
    assert_eq!(pts.len(), 3);
    assert_close(f64_of(&point_at(&body, -2)["assets_total"]), 100.0, "k=-2");
    assert_close(f64_of(&point_at(&body, -1)["assets_total"]), expect(add_months_signed(anchor, -1)), "k=-1");
    assert_close(f64_of(&point_at(&body, 0)["assets_total"]), 400.0, "k=0 = valor vivo exacto");
    // El empalme con lo vivo se comprueba sobre `assets_total`: sin snapshots de pasivo,
    // `net_worth` es null en toda la serie (es lo que antes valía 400 y se leía como patrimonio).
    assert_all_nw_null(&body, "empalme con valores vivos, sin pasivo fotografiado");

    // Nombre: el asset vivo gana sobre el label del snapshot.
    let series = body["asset_series"].as_array().unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["asset_name"], "Bolsa Global");

    // Borrar el asset → su aportación en k=0 pasa a 0 (observado solo a la izquierda).
    let del = app
        .delete_with_cookie(&format!("/v1/assets/{asset_id}"), &owner.cookie)
        .await;
    assert_eq!(del.status, http::StatusCode::NO_CONTENT, "{del:?}");

    let (body2, _, _) = get_series(&app, &owner.cookie, "").await;
    // Predicciones: k=-2 → 100 (exacto en su fecha de snapshot); k=-1 → 0; k=0 → 0.
    assert_close(f64_of(&point_at(&body2, -2)["assets_total"]), 100.0, "post-delete k=-2");
    assert_close(f64_of(&point_at(&body2, -1)["assets_total"]), 0.0, "post-delete k=-1");
    assert_close(f64_of(&point_at(&body2, 0)["assets_total"]), 0.0, "post-delete k=0");
    // Sin asset vivo, el nombre cae al label del snapshot más reciente que lo contiene.
    let series2 = body2["asset_series"].as_array().unwrap();
    assert_eq!(series2[0]["asset_name"], "Bolsa antigua");
}

// ---------------------------------------------------------------------------
// 4. Amortización: curva por encima de la cuerda, extremos exactos
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_amortization_curve_above_chord_and_hits_endpoints() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_, anchor) = server_today(&app, &owner.cookie).await;

    // Pasivo backfilleado en dos fechas (10 meses de separación) con términos
    // apr 5 % / cuota 1200 (cubre el interés: 1200 > 200000·0.05/12 ≈ 833).
    let d1 = add_months_signed(anchor, -12);
    let d2 = add_months_signed(anchor, -2);
    let item_id = Uuid::new_v4().to_string();
    let terms = |value: &str| {
        serde_json::json!([{
            "item_id": item_id, "label": "Hipoteca", "value": value,
            "apr_percent": "5", "payment_amount": "1200", "payment_frequency": "monthly",
        }])
    };
    backfill(&app, &owner, "liability", d1, terms("200000")).await;
    backfill(&app, &owner, "liability", d2, terms("190000")).await;

    let (body, _, _) = get_series(&app, &owner.cookie, "").await;
    let pts = body["points"].as_array().unwrap();
    assert_eq!(pts.len(), 13, "rejilla -12..=0");

    // Predicciones exactas en los extremos (corrección residual):
    //   k=-12 (= d1) → liabilities_total = 200000, net_worth = -200000.
    //   k=-2  (= d2) → 190000.
    assert_close(f64_of(&point_at(&body, -12)["liabilities_total"]), 200_000.0, "k=-12");
    // Alice es la única que aporta serie y SÍ tiene snapshots de pasivo → el pasivo del scope está
    // fotografiado entero y `net_worth` sí se publica (aquí, negativo: solo hay deuda).
    assert_eq!(body["liabilities_snapshotted"], serde_json::json!(true), "{body}");
    assert_close(
        nw_of(point_at(&body, -12)).expect("net_worth con pasivo fotografiado"),
        -200_000.0,
        "nw k=-12",
    );
    assert_close(f64_of(&point_at(&body, -2)["liabilities_total"]), 190_000.0, "k=-2");

    // Predicción interior: la curva francesa es cóncava decreciente → estrictamente POR
    // ENCIMA de la cuerda lineal en todo punto interior. Gap previsto (P_a−M/i = −88000,
    // u = 1+0.05/12, N ≈ 10 meses): ≈ +7 € en k=-11 y ≈ +19 € a mitad (k=-7).
    let total_days = (d2 - d1).num_days() as f64;
    for k in -11..=-3_i32 {
        let g = add_months_signed(anchor, k);
        let chord = 200_000.0 + (190_000.0 - 200_000.0) * ((g - d1).num_days() as f64) / total_days;
        let got = f64_of(&point_at(&body, k)["liabilities_total"]);
        assert!(
            got > chord + 0.5,
            "k={k}: amortización {got} debe superar la cuerda {chord}"
        );
    }
    let mid = f64_of(&point_at(&body, -7)["liabilities_total"]);
    let g_mid = add_months_signed(anchor, -7);
    let chord_mid =
        200_000.0 - 10_000.0 * ((g_mid - d1).num_days() as f64) / total_days;
    assert!(mid > chord_mid + 5.0, "midpoint {mid} vs cuerda {chord_mid}");

    // Predicción tras d2: sin pasivo vivo, la observación virtual de hoy no contiene el
    // item → 0 en k=-1 y k=0. Sin assets: assets_total = 0 en toda la rejilla y no hay
    // asset_series (solo los assets tienen serie por item).
    assert_close(f64_of(&point_at(&body, -1)["liabilities_total"]), 0.0, "k=-1");
    assert_close(f64_of(&point_at(&body, 0)["liabilities_total"]), 0.0, "k=0");
    for p in pts {
        assert_close(f64_of(&p["assets_total"]), 0.0, "assets_total");
    }
    assert_eq!(body["asset_series"].as_array().unwrap().len(), 0);
    let markers = body["markers"].as_array().unwrap();
    assert_eq!(markers.len(), 2);
    assert!(markers.iter().all(|m| m["kind"] == "liability"));
}

// ---------------------------------------------------------------------------
// 5. Household suma usuarios; mine filtra; kind sin snapshots no aporta
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_household_sums_users_and_mine_filters() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let liab_cat = app.create_category(&owner, "liability", "Deuda").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;
    let (_, anchor) = server_today(&app, &owner.cookie).await;

    // Snapshots de asset de AMBOS usuarios en la MISMA fecha (hace 1 mes).
    let d = add_months_signed(anchor, -1);
    backfill(&app, &owner, "asset", d, serde_json::json!([{"label": "A1", "value": "1000"}])).await;
    backfill(&app, &bob, "asset", d, serde_json::json!([{"label": "B1", "value": "600"}])).await;

    // Bob además tiene un pasivo VIVO pero cero snapshots de liability → ese kind no
    // aporta nada a la serie (usuarios sin snapshots de un kind no tienen timeline).
    let future = (Utc::now().date_naive() + Duration::days(365)).format("%Y-%m-%d").to_string();
    app.post_json_with_cookie(
        "/v1/liabilities",
        serde_json::json!({
            "category_id": liab_cat, "expense_category_id": liab_exp_cat, "label": "Coche", "principal": "5000",
            "payment_amount": "150", "payment_frequency": "monthly", "payment_end_date": future,
        }),
        &bob.cookie,
    )
    .await;

    // Household (predicción): rejilla -1..=0.
    //   k=-1 → assets 1000+600 = 1600 (suma server-side de las series por usuario).
    //   k=0  → 0 (ningún usuario tiene assets vivos; items solo-izquierda → 0).
    //   liabilities_total = 0 en TODA la rejilla (kind sin snapshots).
    let (hh, _, _) = get_series(&app, &owner.cookie, "").await;
    assert_eq!(hh["view"], "household");
    assert_eq!(hh["points"].as_array().unwrap().len(), 2);
    assert_close(f64_of(&point_at(&hh, -1)["assets_total"]), 1600.0, "household k=-1");
    // Bob tiene un pasivo VIVO sin fotografiar: el neto del hogar no se publica (antes valía
    // 1600, es decir, los activos, ignorando los 5.000 € de Bob).
    assert_all_nw_null(&hh, "household con pasivo vivo sin fotografiar");
    assert_close(f64_of(&point_at(&hh, 0)["assets_total"]), 0.0, "household k=0");
    for p in hh["points"].as_array().unwrap() {
        assert_close(f64_of(&p["liabilities_total"]), 0.0, "household liabilities");
    }
    // Dos series de asset (item ids distintos), orden asset_name ASC: A1, B1.
    let hh_series = hh["asset_series"].as_array().unwrap();
    assert_eq!(hh_series.len(), 2);
    assert_eq!(hh_series[0]["asset_name"], "A1");
    assert_eq!(hh_series[1]["asset_name"], "B1");
    assert_close(f64_of(&hh_series[0]["values"][0]), 1000.0, "A1 k=-1");
    assert_close(f64_of(&hh_series[1]["values"][0]), 600.0, "B1 k=-1");
    assert_eq!(hh["markers"].as_array().unwrap().len(), 2);

    // Mine de alice (predicción): solo su snapshot → k=-1: 1000; k=0: 0; 1 marker suyo.
    let (mine_a, _, _) = get_series(&app, &owner.cookie, "?view=mine").await;
    assert_eq!(mine_a["view"], "mine");
    assert_close(f64_of(&point_at(&mine_a, -1)["assets_total"]), 1000.0, "mine alice k=-1");
    assert_eq!(mine_a["asset_series"].as_array().unwrap().len(), 1);
    let a_markers = mine_a["markers"].as_array().unwrap();
    assert_eq!(a_markers.len(), 1);
    assert_eq!(a_markers[0]["owner_user_id"].as_str().unwrap(), owner.user_id.to_string());

    // Mine de bob (predicción): k=-1: 600.
    let (mine_b, _, _) = get_series(&app, &bob.cookie, "?view=mine").await;
    assert_close(f64_of(&point_at(&mine_b, -1)["assets_total"]), 600.0, "mine bob k=-1");
    assert_eq!(mine_b["markers"].as_array().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// 6. Markers: fechas, kinds, owner y totales
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_markers_carry_dates_kinds_and_totals() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (_, anchor) = server_today(&app, &owner.cookie).await;

    // Tres snapshots: asset a primero-de-mes (fracción exacta .0), asset a día 15
    // (fracción = k + 14/días_del_mes) y liability a primero-de-mes.
    let d_a = add_months_signed(anchor, -3);
    let d_b = add_months_signed(anchor, -2).with_day(15).expect("day 15 valid");
    let d_l = add_months_signed(anchor, -1);
    backfill(&app, &owner, "asset", d_a,
        serde_json::json!([{"label": "X", "value": "100"}, {"label": "Y", "value": "200"}])).await;
    backfill(&app, &owner, "asset", d_b, serde_json::json!([{"label": "X", "value": "500"}])).await;
    backfill(&app, &owner, "liability", d_l, serde_json::json!([{"label": "L", "value": "700"}])).await;

    let (body, _, _) = get_series(&app, &owner.cookie, "").await;
    let markers = body["markers"].as_array().unwrap();
    assert_eq!(markers.len(), 3, "un marker por snapshot");

    // Predicción (orden snapshot_date ASC):
    //   m0: d_a, kind asset,     total 300 (Σ items), fraction = k(d_a) + 0/días = k exacto.
    //   m1: d_b, kind asset,     total 500, fraction = k(d_b) + 14/días_del_mes.
    //   m2: d_l, kind liability, total 700, fraction = k(d_l).
    let days_b = (add_months_signed(d_b, 1) - add_months_signed(d_b, 0)).num_days() as f64;
    let expected = [
        (d_a, "asset", 300.0, month_index_of(d_a, anchor) as f64),
        (d_b, "asset", 500.0, month_index_of(d_b, anchor) as f64 + 14.0 / days_b),
        (d_l, "liability", 700.0, month_index_of(d_l, anchor) as f64),
    ];
    for (m, (date, kind, total, fraction)) in markers.iter().zip(expected) {
        assert_eq!(ymd(&m["date_ymd"]), date);
        assert_eq!(m["kind"], kind);
        assert_eq!(i32_of(&m["month_index"]), month_index_of(date, anchor));
        assert_close(f64_of(&m["total"]), total, "marker total");
        assert_close_fraction(f64_of(&m["month_fraction"]), fraction, "marker fraction");
        assert_eq!(m["owner_user_id"].as_str().unwrap(), owner.user_id.to_string());
    }
}

// ---------------------------------------------------------------------------
// 7. Snapshot único de hoy (capture) → un solo punto k=0 con lo observado
// ---------------------------------------------------------------------------

#[tokio::test]
async fn series_single_snapshot_today() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "Fondo", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;
    let cap = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({"kinds": ["asset"]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(cap.status, http::StatusCode::OK, "{cap:?}");

    // Predicción: único snapshot = hoy → k_min = 0 → un solo punto {k: 0, assets 10000,
    // liabilities 0, nw 10000}. Sin observación virtual (el último snapshot real ES hoy).
    //
    // El número no cambia con 4.0.0 pero el MECANISMO sí, y conviene dejarlo escrito o el
    // siguiente lector deduce mal: antes el punto se evaluaba el día 1, que cae ANTES del
    // snapshot, y salía por el «enganche del primer mes». Ahora se evalúa en `today`, que ES la
    // fecha del snapshot, así que sale por la rama del valor observado exacto y el enganche ya no
    // interviene.
    let (body, anchor_date, anchor_mf) = get_series(&app, &owner.cookie, "").await;
    let pts = body["points"].as_array().unwrap();
    assert_eq!(pts.len(), 1);
    assert_eq!(i32_of(&pts[0]["month_index"]), 0);
    assert_close(f64_of(&pts[0]["assets_total"]), 10_000.0, "assets k=0");
    assert_close(f64_of(&pts[0]["liabilities_total"]), 0.0, "liabilities k=0");
    // Capture solo de `asset` → el pasivo no está fotografiado → sin `net_worth`.
    assert_all_nw_null(&body, "capture solo de kind asset");

    // asset_series: la serie del asset vivo, nombre vivo, un valor.
    let series = body["asset_series"].as_array().unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["asset_name"], "Fondo");
    assert_eq!(series[0]["values"].as_array().unwrap().len(), 1);
    assert_close(f64_of(&series[0]["values"][0]), 10_000.0, "serie k=0");

    // Marker: fecha = hoy civil, k=0, fraction = (día−1)/días_del_mes, total observado.
    let markers = body["markers"].as_array().unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(ymd(&markers[0]["date_ymd"]), anchor_date);
    assert_eq!(i32_of(&markers[0]["month_index"]), 0);
    let days_in_month =
        (add_months_signed(anchor_mf, 1) - anchor_mf).num_days() as f64;
    let expected_fraction = (anchor_date.day() as f64 - 1.0) / days_in_month;
    assert_close_fraction(f64_of(&markers[0]["month_fraction"]), expected_fraction, "fraction hoy");
    assert_close(f64_of(&markers[0]["total"]), 10_000.0, "marker total");
}

// ---------------------------------------------------------------------------
// 8. REGRESIÓN (auditoría MCP §2) — la serie llega a sus propios snapshots
// ---------------------------------------------------------------------------

/// El repro del issue, reducido: dos snapshots del mes en curso y un activo que solo aparece en el
/// más reciente.
///
/// Hasta 4.0.0 el último punto se evaluaba el **día 1**, así que (a) la curva terminaba por debajo
/// de fotos reales del propio usuario, una de ellas tomada hoy, y (b) un activo cuya primera foto
/// era la última disponible valía `0` en TODA la ventana, porque el segmento que contiene el día 1
/// no lo tenía observado por la izquierda. Un solo hecho, dos síntomas.
#[tokio::test]
async fn series_reaches_snapshots_taken_this_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let (today, anchor) = server_today(&app, &owner.cookie).await;

    // Activo viejo: está en las dos fotos. Activo nuevo: solo en la de hoy.
    let viejo = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "Cartera", "current_value": "1000"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let nuevo = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "N26", "current_value": "230"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Foto de hace dos meses: solo la cartera.
    backfill(
        &app,
        &owner,
        "asset",
        add_months_signed(anchor, -2),
        serde_json::json!([{"item_id": viejo, "label": "Cartera", "value": "800"}]),
    )
    .await;
    // Foto de HOY: las dos. Es la que la serie no alcanzaba.
    backfill(
        &app,
        &owner,
        "asset",
        today,
        serde_json::json!([
            {"item_id": viejo, "label": "Cartera", "value": "1000"},
            {"item_id": nuevo, "label": "N26", "value": "230"},
        ]),
    )
    .await;

    let (body, _, _) = get_series(&app, &owner.cookie, "").await;

    // El último punto vale EXACTAMENTE el total de la foto de hoy, y no un céntimo menos.
    assert_close(
        f64_of(&point_at(&body, 0)["assets_total"]),
        1230.0,
        "k=0 debe alcanzar el snapshot de hoy",
    );

    // Y el marker de hoy dice lo mismo que el punto: era la contradicción visible del issue.
    let marker_hoy = body["markers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["date_ymd"] == serde_json::json!(today.format("%Y-%m-%d").to_string()))
        .expect("marker de hoy");
    assert_close(
        f64_of(&marker_hoy["total"]),
        f64_of(&point_at(&body, 0)["assets_total"]),
        "el punto 0 y el marker de hoy son la misma cifra",
    );

    // El activo que solo está en la foto reciente ya no vale 0 en el último punto.
    let (body_s, _, _) = get_series(&app, &owner.cookie, "?include_asset_series=true").await;
    let n26 = body_s["asset_series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["asset_name"] == serde_json::json!("N26"))
        .expect("serie de N26");
    let values = n26["values"].as_array().unwrap();
    assert_close(
        f64_of(values.last().unwrap()),
        230.0,
        "un activo cuya primera foto es la última disponible ya no vale 0 en k=0",
    );
}

/// `liabilities_snapshotted` distingue «no lo he fotografiado» de «no debo nada», y **manda sobre
/// `net_worth`**.
///
/// Sin snapshots de pasivo no hay timeline de ese kind y no hay fallback a las filas vivas, así que
/// `liabilities_total` es 0 en toda la serie aunque haya una hipoteca abierta. Esas dos cifras no
/// cambian —el histórico es lo que fotografiaste, y ése es su contrato—, pero `net_worth` sí:
/// pasa a `null`, porque `assets_total − 0` no es un patrimonio neto. Antes se publicaba como
/// número, idéntico a `assets_total`, y contradecía a `GET /v1/summary` en la misma conversación.
#[tokio::test]
async fn liabilities_snapshotted_tells_missing_data_from_no_debt() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_a = app.create_category(&owner, "asset", "Cash").await;
    let cat_l = app.create_category(&owner, "liability", "Préstamos").await;
    let (_, anchor) = server_today(&app, &owner.cookie).await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat_a, "name": "Cuenta", "current_value": "5000"}),
            &owner.cookie,
        )
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Pasivo VIVO de 548 €, sin fotografiar.
    let cat_e = app.create_category(&owner, "expense", "Cuotas").await;
    let liab = app
        .post_json_with_cookie(
            "/v1/liabilities",
            serde_json::json!({
                "category_id": cat_l, "expense_category_id": cat_e,
                "label": "Préstamo", "principal": "548",
                "payment_amount": "50", "payment_frequency": "monthly",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");

    backfill(
        &app,
        &owner,
        "asset",
        add_months_signed(anchor, -1),
        serde_json::json!([{"item_id": asset, "label": "Cuenta", "value": "4000"}]),
    )
    .await;

    let (body, _, _) = get_series(&app, &owner.cookie, "").await;
    assert_eq!(
        body["liabilities_snapshotted"],
        serde_json::json!(false),
        "hay deuda viva pero ningún snapshot de pasivo: {body}"
    );
    assert_close(
        f64_of(&point_at(&body, 0)["liabilities_total"]),
        0.0,
        "la cifra NO cambia: el histórico sigue siendo lo fotografiado",
    );
    // `assets_total` sigue publicándose intacto…
    assert_close(
        f64_of(&point_at(&body, -1)["assets_total"]),
        4000.0,
        "assets_total intacto con el flag en false",
    );
    // …y `net_worth` desaparece de TODOS los puntos (no solo del último).
    assert_all_nw_null(&body, "deuda viva sin fotografiar");

    // Con una foto de pasivo, el flag se enciende y `net_worth` vuelve, ya restando de verdad:
    // en k=-1 (la fecha de ambos snapshots) vale 4000 − 600 = 3400 exacto.
    backfill(
        &app,
        &owner,
        "liability",
        add_months_signed(anchor, -1),
        serde_json::json!([{"item_id": liab.json()["id"].as_str().unwrap(), "label": "Préstamo", "value": "600"}]),
    )
    .await;
    let (body2, _, _) = get_series(&app, &owner.cookie, "").await;
    assert_eq!(body2["liabilities_snapshotted"], serde_json::json!(true), "{body2}");
    assert_close(
        nw_of(point_at(&body2, -1)).expect("net_worth con el flag en true"),
        3400.0,
        "nw k=-1 = 4000 − 600",
    );
}

/// EL CASO DIFÍCIL: el hogar es la **suma** de las series de cada usuario, así que basta con que
/// UNO no haya fotografiado su pasivo para que el neto agregado esté inflado.
///
/// El flag es por eso un `all`, no un `any`: con Alice fotografiando su préstamo y Bob no, un `any`
/// daría `true` y el hogar publicaría `activos − deuda_de_Alice`, un número que ya no coincide con
/// `assets_total` y que por tanto **parece** un patrimonio neto correcto. Es la misma mentira que
/// el flag existe para impedir, solo que indetectable a ojo.
///
/// Y `?view=mine` no se contagia: el scope de Alice está completo, así que ella sí ve su neto.
#[tokio::test]
async fn household_net_worth_needs_every_member_to_have_snapshotted_liabilities() {
    let app = TestApp::spawn().await;
    let alice = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&alice, "bob", "member").await;
    let (_, anchor) = server_today(&app, &alice.cookie).await;
    let d = add_months_signed(anchor, -1);

    // Alice: activos 1000 + pasivo 300, ambos fotografiados. Bob: solo activos 600 (su deuda,
    // exista o no, nunca se ha fotografiado).
    backfill(&app, &alice, "asset", d, serde_json::json!([{"label": "A1", "value": "1000"}])).await;
    backfill(&app, &alice, "liability", d, serde_json::json!([{"label": "L1", "value": "300"}])).await;
    backfill(&app, &bob, "asset", d, serde_json::json!([{"label": "B1", "value": "600"}])).await;

    // Household: assets 1600, liabilities 300 (solo las de Alice) → `any` habría publicado
    // net_worth = 1300, que no es el neto del hogar ni el de nadie.
    let (hh, _, _) = get_series(&app, &alice.cookie, "").await;
    assert_close(f64_of(&point_at(&hh, -1)["assets_total"]), 1600.0, "household assets k=-1");
    assert_close(f64_of(&point_at(&hh, -1)["liabilities_total"]), 300.0, "household liabs k=-1");
    assert_all_nw_null(&hh, "hogar con un miembro sin pasivo fotografiado");

    // Mine de Alice: su scope está completo → 1000 − 300 = 700.
    let (mine_a, _, _) = get_series(&app, &alice.cookie, "?view=mine").await;
    assert_eq!(mine_a["liabilities_snapshotted"], serde_json::json!(true), "{mine_a}");
    assert_close(
        nw_of(point_at(&mine_a, -1)).expect("net_worth de Alice"),
        700.0,
        "mine alice nw k=-1",
    );

    // Mine de Bob: incompleto → sin neto, con sus activos intactos.
    let (mine_b, _, _) = get_series(&app, &bob.cookie, "?view=mine").await;
    assert_close(f64_of(&point_at(&mine_b, -1)["assets_total"]), 600.0, "mine bob assets k=-1");
    assert_all_nw_null(&mine_b, "mine de bob, sin pasivo fotografiado");

    // Bob DECLARA que no debe nada capturando un snapshot de pasivo (la cabecera se escribe aunque
    // no tenga ni una fila viva). El hogar recupera su neto: 1600 − 300 = 1300 — el mismo número
    // que el `any` habría dado, pero ahora respaldado por un hecho que Bob ha afirmado.
    let cap = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({"kinds": ["liability"]}),
            &bob.cookie,
        )
        .await;
    assert_eq!(cap.status, http::StatusCode::OK, "{cap:?}");

    let (hh2, _, _) = get_series(&app, &alice.cookie, "").await;
    assert_eq!(hh2["liabilities_snapshotted"], serde_json::json!(true), "{hh2}");
    assert_close(
        nw_of(point_at(&hh2, -1)).expect("net_worth del hogar ya completo"),
        1300.0,
        "household nw k=-1 = 1600 − 300",
    );
}
