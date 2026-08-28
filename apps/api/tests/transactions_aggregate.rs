//! `GET /v1/transactions/aggregate`, el filtro `uncategorized` y `GET /v1/transactions/duplicates`.
//!
//! El test que importa es [`aggregate_matches_get_transactions_summary_month_by_month`]: la
//! agregación y la comparativa son **dos agregados de flujo del mismo módulo**, así que para el
//! mismo mes y la misma categoría tienen que dar el mismo número. Si divergen, uno de los dos
//! miente — y el modo de fallo de este repositorio es precisamente el número plausible y falso.
//!
//! El escenario incluye a propósito una **transferencia interna conciliada** de 500 €: si la
//! agregación se olvidara del predicado `transfer_counterpart_id IS NULL`, el gasto del mes saldría
//! 680 en vez de 180 y el ingreso 2.500 en vez de 2.000. Ambas cifras son perfectamente creíbles.

mod common;

use chrono::{Datelike, NaiveDate};
use common::TestApp;
use http::StatusCode;
use serde_json::{json, Value};

fn parse_dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un decimal como string, llegó {v:?}"))
        .parse::<f64>()
        .expect("parsear el decimal")
}

fn approx(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() < 0.01, "{what}: se esperaba ~{b}, llegó {a}");
}

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
    cat: Option<&str>,
) -> Value {
    let mut body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind });
    if let Some(c) = cat {
        body["category_id"] = json!(c);
    }
    let r = app
        .post_json_with_cookie("/v1/transactions", body, cookie)
        .await;
    assert_eq!(r.status, StatusCode::CREATED, "manual {concept}: {r:?}");
    r.json()
}

/// Línea de una lista de categorías del summary, por nombre.
fn line<'a>(arr: &'a Value, name: &str) -> &'a Value {
    arr.as_array()
        .unwrap()
        .iter()
        .find(|l| l["category_name"] == name)
        .unwrap_or_else(|| panic!("no hay línea '{name}' en {arr:?}"))
}

/// Fila de `by_category` del agregado, por `category_id` (`None` = sin categoría).
fn agg_cat<'a>(agg: &'a Value, cat_id: Option<&str>) -> &'a Value {
    agg["by_category"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| match cat_id {
            Some(id) => l["category_id"] == json!(id),
            None => l["category_id"].is_null(),
        })
        .unwrap_or_else(|| panic!("no hay fila de categoría {cat_id:?} en {agg:?}"))
}

#[tokio::test]
async fn aggregate_matches_get_transactions_summary_month_by_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("agg_owner").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;
    let nomina_cat = app.create_category(&owner, "income", "Nómina").await;

    let today = server_today(&app, &owner.cookie).await;
    // Mes seleccionado = 2 meses antes de hoy: siempre completo.
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let ym = format!("{sy:04}-{sm:02}");

    // Gasto: Super 100 + 50, sin categoría 30 → 180. Ingreso 2000. Ahorro 200.
    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Super A", "-100", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 12), "Super B", "-50", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 8), "Kiosko", "-30", "expense", None).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 1), "Sueldo", "2000", "income", Some(&nomina_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 15), "Aporte", "-200", "savings", None).await;

    // Transferencia interna: dos patas exactamente opuestas el mismo día → el pase automático las
    // concilia post-commit. NO son gasto ni ingreso, y ninguno de los dos agregados debe contarlas.
    let out_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 20), "Traspaso salida", "-500", "expense", None).await;
    let in_leg = manual(&app, &owner.cookie, &date_in(sy, sm, 20), "Traspaso entrada", "500", "income", None).await;
    for leg in [&out_leg, &in_leg] {
        let id = leg["id"].as_str().unwrap();
        let r = app
            .get_with_cookie(&format!("/v1/transactions?month={ym}"), &owner.cookie)
            .await;
        let found = r
            .json()
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == json!(id))
            .cloned()
            .unwrap();
        assert!(
            found["transfer_counterpart_id"].is_string(),
            "la pata {id} debería estar conciliada por el pase automático: {found}"
        );
    }

    // ---- Referencia: la comparativa ---------------------------------------------------------
    let summary = app
        .get_with_cookie(
            &format!("/v1/transactions/summary?year={sy}&month={sm}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(summary.status, StatusCode::OK, "summary: {summary:?}");
    let s = summary.json();
    approx(parse_dec(&s["totals"]["expense_actual"]), 180.0, "summary gasto");
    approx(parse_dec(&s["totals"]["income_actual"]), 2000.0, "summary ingreso");
    approx(parse_dec(&s["savings"]["actual"]), 200.0, "summary ahorro");

    // ---- El agregado, kind a kind ------------------------------------------------------------
    for (kind, expected_total, expected_excluded) in
        [("expense", 180.0, 1), ("income", 2000.0, 1), ("savings", 200.0, 0)]
    {
        let r = app
            .get_with_cookie(
                &format!("/v1/transactions/aggregate?month={ym}&kind={kind}"),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "aggregate {kind}: {r:?}");
        let a = r.json();
        assert_eq!(a["kind_basis"], json!(kind), "kind_basis de {kind}: {a}");
        approx(parse_dec(&a["total"]), expected_total, &format!("aggregate {kind}"));
        assert_eq!(
            a["reconciled_excluded_count"].as_i64().unwrap(),
            expected_excluded,
            "conciliadas excluidas en {kind}: {a}"
        );
        // El mes es único en el escenario: `by_month` debe cuadrar con el total.
        let months = a["by_month"].as_array().unwrap();
        assert_eq!(months.len(), 1, "un solo mes con datos: {a}");
        assert_eq!(months[0]["month"], json!(ym));
        approx(parse_dec(&months[0]["total"]), expected_total, "by_month");
    }

    // ---- Paridad LÍNEA A LÍNEA con el summary -------------------------------------------------
    // No basta con que cuadren los totales: la agregación por categoría es la que responde
    // «¿cuánto llevo gastado en X?», y es donde un filtro de menos pasa desapercibido.
    let agg_exp = app
        .get_with_cookie(
            &format!("/v1/transactions/aggregate?month={ym}&kind=expense"),
            &owner.cookie,
        )
        .await
        .json();
    for l in s["expense_categories"].as_array().unwrap() {
        let expected = parse_dec(&l["actual"]);
        if expected == 0.0 {
            // El summary materializa líneas de categorías presupuestadas sin movimientos; el
            // agregado solo enseña lo que existe. Cero contra ausencia es la misma afirmación.
            continue;
        }
        let cat_id = l["category_id"].as_str();
        let row = agg_cat(&agg_exp, cat_id);
        approx(
            parse_dec(&row["total"]),
            expected,
            &format!("categoría {}", l["category_name"]),
        );
    }
    approx(parse_dec(&agg_cat(&agg_exp, Some(&super_cat))["total"]), 150.0, "Super");
    approx(parse_dec(&agg_cat(&agg_exp, None)["total"]), 30.0, "sin categoría");
    // El reparto porcentual dentro del kind: 150/180 y 30/180.
    approx(parse_dec(&agg_cat(&agg_exp, Some(&super_cat))["share_pct"]), 83.3, "share Super");
    approx(parse_dec(&agg_cat(&agg_exp, None)["share_pct"]), 16.7, "share sin categoría");
    approx(parse_dec(&line(&s["expense_categories"], "Super")["actual"]), 150.0, "summary Super");

    // ---- Sin filtro de kind no hay magnitud, y se dice por qué -------------------------------
    let mixed = app
        .get_with_cookie(
            &format!("/v1/transactions/aggregate?month={ym}"),
            &owner.cookie,
        )
        .await
        .json();
    assert!(mixed["total"].is_null(), "mezclando kinds no hay magnitud: {mixed}");
    assert_eq!(mixed["total_absent_reason"], json!("mixed_kinds"), "{mixed}");
    assert_eq!(mixed["reconciled_excluded_count"].as_i64().unwrap(), 2, "{mixed}");
    // Σ firmada del mes: −180 + 2000 − 200 = 1620.
    approx(parse_dec(&mixed["total_signed"]), 1620.0, "total_signed mezclado");
    // `by_kind` sí tiene magnitud por fila, porque cada fila trae su propio kind.
    let by_kind = mixed["by_kind"].as_array().unwrap();
    assert_eq!(by_kind[0]["kind"], json!("expense"), "orden fijo: {mixed}");
    approx(parse_dec(&by_kind[0]["total"]), 180.0, "by_kind gasto");

    // ---- El top-N y el conjunto vacío ---------------------------------------------------------
    let top = app
        .get_with_cookie(
            &format!("/v1/transactions/aggregate?month={ym}&kind=expense&top=1"),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(top["top"].as_array().unwrap().len(), 1, "{top}");
    assert_eq!(top["top"][0]["concept"], json!("Super A"), "el mayor por importe absoluto: {top}");
    assert_eq!(top["top_truncated"], json!(true), "{top}");

    let empty = app
        .get_with_cookie(
            "/v1/transactions/aggregate?month=1999-01&kind=expense",
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(empty["transaction_count"].as_i64().unwrap(), 0, "{empty}");
    assert!(empty["total"].is_null(), "{empty}");
    assert_eq!(empty["total_absent_reason"], json!("no_transactions"), "{empty}");
    approx(parse_dec(&empty["total_signed"]), 0.0, "Σ del conjunto vacío");
}

#[tokio::test]
async fn uncategorized_filter_finds_the_gap_without_the_savings_false_positive() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("uncat_owner").await;
    let super_cat = app.create_category(&owner, "expense", "Super").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);
    let ym = format!("{sy:04}-{sm:02}");

    manual(&app, &owner.cookie, &date_in(sy, sm, 5), "Con categoria", "-100", "expense", Some(&super_cat)).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 8), "Kiosko", "-30", "expense", None).await;
    // `savings` NO lleva categoría POR DISEÑO (`savings_no_category`): sin la exclusión, la
    // respuesta a «¿qué me falta por categorizar?» encabezaría con una aportación que no se puede
    // categorizar nunca.
    manual(&app, &owner.cookie, &date_in(sy, sm, 15), "Aporte", "-200", "savings", None).await;

    let r = app
        .get_with_cookie(
            &format!("/v1/transactions?month={ym}&uncategorized=true"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{r:?}");
    let rows = r.json();
    let conceptos: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["concept"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(conceptos, vec!["Kiosko".to_string()], "solo el gasto sin categoría: {rows}");

    // Pedir `savings` explícitamente SÍ los devuelve: la exclusión es un default, no una
    // amputación — ningún conjunto queda inalcanzable.
    let r = app
        .get_with_cookie(
            &format!("/v1/transactions?month={ym}&uncategorized=true&kind=savings"),
            &owner.cookie,
        )
        .await;
    let rows = r.json();
    let conceptos: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["concept"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(conceptos, vec!["Aporte".to_string()], "{rows}");

    // Contradicción explícita → error, no una lista vacía plausible.
    let r = app
        .get_with_cookie(
            &format!("/v1/transactions?uncategorized=true&category_id={super_cat}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], json!("category_filter_exclusive"), "{:?}", r.json());

    // El mismo eje en la agregación, con el mismo código de error y el mismo conjunto.
    let agg = app
        .get_with_cookie(
            &format!("/v1/transactions/aggregate?month={ym}&uncategorized=true&kind=expense"),
            &owner.cookie,
        )
        .await
        .json();
    approx(parse_dec(&agg["total"]), 30.0, "agregado sin categoría");
    let r = app
        .get_with_cookie(
            &format!("/v1/transactions/aggregate?uncategorized=true&category_id={super_cat}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], json!("category_filter_exclusive"), "{:?}", r.json());
}

#[tokio::test]
async fn duplicates_group_by_fingerprint_and_flag_multi_origin() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("dup_owner").await;

    let today = server_today(&app, &owner.cookie).await;
    let (sy, sm) = shift_month(today.year(), today.month(), -2);

    // Dos filas idénticas (misma fecha, importe y concepto) → misma huella, ordinales 0 y 1.
    manual(&app, &owner.cookie, &date_in(sy, sm, 7), "Cafe", "-1.80", "expense", None).await;
    manual(&app, &owner.cookie, &date_in(sy, sm, 7), "Cafe", "-1.80", "expense", None).await;
    // Una fila sola: no debe generar grupo.
    manual(&app, &owner.cookie, &date_in(sy, sm, 9), "Libreria", "-12", "expense", None).await;

    let r = app
        .get_with_cookie("/v1/transactions/duplicates", &owner.cookie)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["group_count"].as_i64().unwrap(), 1, "{b}");
    assert_eq!(b["group_count_total"].as_i64().unwrap(), 1, "{b}");
    assert_eq!(b["truncated"], json!(false), "{b}");
    let g = &b["groups"][0];
    assert_eq!(g["transaction_count"].as_i64().unwrap(), 2, "{g}");
    assert_eq!(g["concept"], json!("Cafe"), "{g}");
    // Las dos son manuales: un solo origen → candidato DÉBIL (los duplicados legítimos existen).
    assert_eq!(g["distinct_import_count"].as_i64().unwrap(), 1, "{g}");
    assert_eq!(g["spans_multiple_imports"], json!(false), "{g}");
    let ordinals: Vec<i64> = g["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["fingerprint_ordinal"].as_i64().unwrap())
        .collect();
    assert_eq!(ordinals, vec![0, 1], "{g}");

    // El filtro es el mismo del listado: acotar a un mes sin duplicados no devuelve grupos.
    let (oy, om) = shift_month(sy, sm, -1);
    let r = app
        .get_with_cookie(
            &format!("/v1/transactions/duplicates?month={oy:04}-{om:02}"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.json()["group_count"].as_i64().unwrap(), 0, "{:?}", r.json());
}
