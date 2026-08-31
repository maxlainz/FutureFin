//! Fase 5 — **campos de procedencia y coste de contexto**.
//!
//! Este fichero fija lo que la Fase 5 añadió a las respuestas: el eco de `view`, la declaración
//! plan-vs-real de los equivalentes mensuales, los límites y defaults de las series, y los
//! `*_absent_reason` / `*_included` que convierten un hueco en un dato.
//!
//! El criterio común de todas estas aserciones: **una respuesta no puede tener dos lecturas**.
//! Un array vacío que puede significar «no hay nada» o «te lo he recortado», un `0` que puede
//! significar «cero euros» o «esta cifra no aplica», y dos respuestas byte a byte idénticas para
//! dos preguntas distintas son el mismo fallo con tres caras.

mod common;

use chrono::NaiveDate;
use common::{LoggedInOwner, TestApp};
use futurefin_engine::add_months_signed;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

fn dec(v: &serde_json::Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un decimal-string, llegó {v:?}"))
        .parse()
        .expect("parse decimal string")
}

/// Nº de decimales con los que un f64 llega SERIALIZADO en el JSON. Se mira sobre el texto, no
/// sobre el `f64` reconstruido: lo que cuesta contexto es la cadena.
fn decimals_of(v: &serde_json::Value) -> usize {
    let s = v.to_string();
    match s.split_once('.') {
        Some((_, frac)) => frac.trim_end_matches(|c: char| !c.is_ascii_digit()).len(),
        None => 0,
    }
}

async fn get(app: &TestApp, cookie: &str, uri: &str) -> serde_json::Value {
    let r = app.get_with_cookie(uri, cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "GET {uri}: {r:?}");
    r.json()
}

async fn server_anchor(app: &TestApp, cookie: &str) -> NaiveDate {
    let body = get(app, cookie, "/v1/history/series").await;
    NaiveDate::parse_from_str(body["anchor_month_first_ymd"].as_str().unwrap(), "%Y-%m-%d")
        .expect("anchor")
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
// 1. `view` en la raíz de TODA respuesta que acepta el parámetro
// ---------------------------------------------------------------------------

/// El síntoma que motiva el campo: en una instalación de un solo usuario, `?view=mine` y `?view`
/// omitido devuelven el MISMO contenido, así que sin el eco es imposible distinguir «mine coincide
/// con el hogar» de «el parámetro se ignoró». En un hogar de dos, ésa es la pregunta que decide si
/// la cifra citada es la correcta.
#[tokio::test]
async fn every_view_aware_response_echoes_the_view_it_applied() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "Bolsa", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;

    // Las ocho respuestas-objeto que aceptan `?view`. Los listados siguen siendo arrays planos en
    // HTTP a propósito (convertirlos en objeto rompería el contrato REST; el precedente literal es
    // `get_allocation_resolution`, que nació como endpoint NUEVO por esa razón): su eco de `view`
    // va en el envelope de la tool MCP.
    let endpoints = [
        "/v1/summary",
        "/v1/budget",
        "/v1/projection/series",
        "/v1/allocation-rules/resolution",
        "/v1/history/series",
        "/v1/history/cashflow",
        "/v1/transactions/summary",
        "/v1/transactions/category-series?kind=expense",
    ];

    for ep in endpoints {
        let sep = if ep.contains('?') { '&' } else { '?' };

        let household = get(&app, &owner.cookie, ep).await;
        assert_eq!(
            household["view"], "household",
            "{ep} sin ?view debe declarar household: {household}"
        );

        let explicit = get(&app, &owner.cookie, &format!("{ep}{sep}view=household")).await;
        assert_eq!(explicit["view"], "household", "{ep}?view=household: {explicit}");

        let mine = get(&app, &owner.cookie, &format!("{ep}{sep}view=mine")).await;
        assert_eq!(mine["view"], "mine", "{ep}?view=mine: {mine}");

        // La prueba de que el campo sirve para algo: con un solo usuario el resto del payload es
        // idéntico, y aun así las dos respuestas se distinguen.
        let mut sin_view = mine.clone();
        sin_view["view"] = household["view"].clone();
        assert_eq!(
            sin_view, household,
            "{ep}: con un solo usuario mine y household solo deberían diferir en `view`"
        );
        assert_ne!(mine["view"], household["view"], "{ep}");
    }
}

// ---------------------------------------------------------------------------
// 2. Plan vs real: cuatro nombres idénticos, dos significados
// ---------------------------------------------------------------------------

/// `income_monthly_equivalent`, `expense_regular_monthly_equivalent`,
/// `expense_total_monthly_equivalent` y `net_monthly_equivalent` existen con el MISMO nombre en
/// `/v1/budget.totals` y en `/v1/summary.financial_health`, y valen cosas distintas en cuanto el
/// modo deja de ser `budget`. Ninguno se renombra (rompería la SPA); lo que hacen los dos `basis`
/// es que la diferencia sea legible sin conocer el modo.
#[tokio::test]
async fn budget_and_summary_declare_whether_their_totals_are_plan_or_actual() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nómina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    for (cat, amount) in [(&cat_inc, "3000"), (&cat_exp, "1000")] {
        app.post_json_with_cookie(
            "/v1/budget/entries",
            serde_json::json!({"category_id": cat, "amount": amount}),
            &owner.cookie,
        )
        .await;
    }

    let budget = get(&app, &owner.cookie, "/v1/budget").await;
    // El presupuesto es el PLAN siempre, en los tres modos. Es una constante, y esa es la gracia.
    assert_eq!(budget["totals"]["basis"], "plan", "{budget}");

    // Modo A (default): el summary también es plan, y las cuatro cifras coinciden.
    let summary = get(&app, &owner.cookie, "/v1/summary").await;
    let fh = &summary["financial_health"];
    assert_eq!(fh["basis"], "plan", "modo budget: {fh}");
    for campo in [
        "income_monthly_equivalent",
        "expense_regular_monthly_equivalent",
        "expense_total_monthly_equivalent",
        "net_monthly_equivalent",
    ] {
        assert_eq!(
            dec(&fh[campo]),
            dec(&budget["totals"][campo]),
            "modo budget: `{campo}` debe coincidir en las dos superficies"
        );
    }

    // Modo B sin movimientos reales: el fallback por lado deja las DOS bases en presupuesto, así
    // que `basis` sigue diciendo `plan` — y eso es exactamente lo que hay que poder leer. Un
    // `savings_source` a secas diría `budget` por el fallback y no distinguiría el caso.
    let iid = app.installation_id().await;
    sqlx::query(
        "UPDATE installation SET fire_settings = jsonb_set(COALESCE(fire_settings, '{}'::jsonb), \
         '{savings_source}', '\"transactions_avg\"') WHERE id = $1",
    )
    .bind(iid)
    .execute(&app.pool)
    .await
    .expect("forzar modo B");

    let summary_b = get(&app, &owner.cookie, "/v1/summary").await;
    let fh_b = &summary_b["financial_health"];
    assert_eq!(
        fh_b["savings_income_basis"]["basis"], "budget",
        "sin meses reales el lado ingreso cae al presupuesto: {fh_b}"
    );
    assert_eq!(
        fh_b["basis"], "plan",
        "los dos lados en presupuesto ⇒ basis = plan, comparable con /v1/budget: {fh_b}"
    );
}

// ---------------------------------------------------------------------------
// 3. Próximos: totales sin ventana, pero con su horizonte declarado
// ---------------------------------------------------------------------------

/// `upcoming_inflows_total` / `upcoming_outflows_total` / `upcoming_coverage_ratio` suman TODOS los
/// Próximos del scope, sin ventana: pueden mezclar un cobro a dieciséis años con un pago del mes
/// que viene. No se les pone ventana (cambiaría una cifra de portada); se publica el horizonte que
/// están sumando, que es lo que faltaba para poder leerlas.
#[tokio::test]
async fn upcoming_totals_publish_the_horizon_they_are_summing() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Extras").await;
    let cat_exp = app.create_category(&owner, "expense", "Impuestos").await;

    let sin_proximos = get(&app, &owner.cookie, "/v1/summary").await;
    let fh0 = &sin_proximos["financial_health"];
    assert_eq!(fh0["upcoming_flows_count"], 0, "{fh0}");
    assert!(
        fh0["upcoming_last_due_date_ymd"].is_null(),
        "sin Próximos no hay horizonte: {fh0}"
    );

    // Un cobro cercano, un pago lejano y uno SIN fecha: los tres suman, y solo dos tienen fecha.
    for (cat, title, amount, due) in [
        (&cat_inc, "Paga extra", "1200", Some("2027-06-30")),
        (&cat_exp, "Derrama", "9000", Some("2041-01-15")),
        (&cat_exp, "Sin fecha", "300", None),
    ] {
        let mut body = serde_json::json!({
            "category_id": cat, "title": title, "expected_amount": amount
        });
        if let Some(d) = due {
            body["due_date"] = serde_json::json!(d);
        }
        let r = app
            .post_json_with_cookie("/v1/planning/flows", body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "planning flow: {r:?}");
    }

    let summary = get(&app, &owner.cookie, "/v1/summary").await;
    let fh = &summary["financial_health"];
    assert_eq!(dec(&fh["upcoming_inflows_total"]), 1200.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_outflows_total"]), 9300.0, "el sin-fecha suma: {fh}");
    assert_eq!(fh["upcoming_flows_count"], 3, "los tres cuentan, fechados o no: {fh}");
    assert_eq!(
        fh["upcoming_last_due_date_ymd"], "2041-01-15",
        "el horizonte es la fecha MÁS LEJANA de las que suman: {fh}"
    );
    // Y la ratio sigue siendo una fracción sobre esos mismos operandos sin ventana.
    assert!((dec(&fh["upcoming_coverage_ratio"]) - 1200.0 / 9300.0).abs() < 1e-6, "{fh}");

    // #148: un recurrente de 800 €/MES NO entra en los totales en € — mezclaría magnitudes.
    // Va aparte, en €/mes, y el count global sí lo cuenta.
    let r = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            serde_json::json!({
                "category_id": cat_exp, "title": "Alquiler", "expected_amount": "800",
                "amount_basis": "per_month", "window_start_date": "2026-09-01"
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "per_month: {r:?}");

    let summary = get(&app, &owner.cookie, "/v1/summary").await;
    let fh = &summary["financial_health"];
    assert_eq!(
        dec(&fh["upcoming_outflows_total"]),
        9300.0,
        "el €/mes no contamina el total en €: {fh}"
    );
    assert_eq!(dec(&fh["upcoming_recurring_monthly_outflow"]), 800.0, "{fh}");
    assert_eq!(dec(&fh["upcoming_recurring_monthly_inflow"]), 0.0, "{fh}");
    assert_eq!(fh["upcoming_recurring_count"], 1, "{fh}");
    assert_eq!(fh["upcoming_flows_count"], 4, "el count global cuenta TODO: {fh}");
    // La ratio conserva su base (solo puntuales) — su helpText lo declara.
    assert!((dec(&fh["upcoming_coverage_ratio"]) - 1200.0 / 9300.0).abs() < 1e-6, "{fh}");
}

// ---------------------------------------------------------------------------
// 4. `liabilities_by_type_tag`: `null` en vez de un literal español
// ---------------------------------------------------------------------------

/// La línea de «sin etiqueta» viajaba como la cadena `"(sin etiqueta)"`: texto de interfaz dentro
/// de un campo de datos, indistinguible de un `type_tag` que el usuario hubiera escrito así, y no
/// reenviable como filtro. Ahora es `null`, el mismo criterio que `category_id` para «sin
/// categoría». **Breaking** para quien leyera el literal; la SPA no lee este desglose.
#[tokio::test]
async fn untagged_liabilities_group_under_null_not_a_spanish_literal() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_liab = app.create_category(&owner, "liability", "Deudas").await;
    let cat_exp = app.create_category(&owner, "expense", "Cuotas").await;

    for (label, tag) in [("Hipoteca", Some("hipoteca")), ("Préstamo", None)] {
        let mut body = serde_json::json!({
            "category_id": cat_liab,
            "expense_category_id": cat_exp,
            "label": label,
            "principal": "100000",
        });
        if let Some(t) = tag {
            body["type_tag"] = serde_json::json!(t);
        }
        let r = app
            .post_json_with_cookie("/v1/liabilities", body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "liability: {r:?}");
    }

    let summary = get(&app, &owner.cookie, "/v1/summary").await;
    let tags = summary["liabilities_by_type_tag"].as_array().unwrap();
    assert_eq!(tags.len(), 2, "{summary}");

    let etiquetado = tags.iter().find(|l| l["type_tag"] == "hipoteca").expect("hipoteca");
    assert_eq!(dec(&etiquetado["total"]), 100_000.0);

    let sin_etiqueta = tags.iter().find(|l| l["type_tag"].is_null()).expect("línea sin etiqueta");
    assert_eq!(dec(&sin_etiqueta["total"]), 100_000.0);
    for l in tags {
        assert_ne!(
            l["type_tag"], "(sin etiqueta)",
            "ningún literal de interfaz dentro de un campo de datos: {summary}"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Serie histórica: default acotado, truncado declarado, 2 decimales
// ---------------------------------------------------------------------------

/// El peor caso del catálogo era el DEFAULT: sin `window_months` la serie salía desde el snapshot
/// más antiguo del scope. Un hogar que ancla su histórico muy atrás —hasta su fecha de
/// nacimiento— recibía cientos de puntos interpolando desde ~0 €, y nada en la respuesta decía
/// que eso era lo que estaba pasando.
#[tokio::test]
async fn history_series_default_window_is_bounded_and_says_it_truncated() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let anchor = server_anchor(&app, &owner.cookie).await;

    let item = Uuid::new_v4().to_string();
    // Ancla remota (15 años) de importe simbólico + una foto reciente: la forma exacta del
    // «snapshot en la fecha de nacimiento» que la auditoría encontró.
    let remoto = add_months_signed(anchor, -180);
    backfill(&app, &owner, "asset", remoto,
        serde_json::json!([{"item_id": item, "label": "Cash", "value": "1"}])).await;
    backfill(&app, &owner, "asset", add_months_signed(anchor, -2),
        serde_json::json!([{"item_id": item, "label": "Cash", "value": "50000"}])).await;

    // Default: 120 meses, recortado, y la respuesta lo dice y señala dónde empieza lo que hay.
    let def = get(&app, &owner.cookie, "/v1/history/series").await;
    assert_eq!(def["window_months"], 120, "default acotado: {def}");
    assert_eq!(def["window_truncated"], true, "hay histórico fuera de la ventana: {def}");
    assert_eq!(
        def["first_snapshot_date_ymd"],
        serde_json::json!(remoto.format("%Y-%m-%d").to_string()),
        "el snapshot más antiguo se declara aunque quede fuera: {def}"
    );
    assert_eq!(def["first_snapshot_month_index"], -180, "{def}");
    let def_pts = def["points"].as_array().unwrap();
    assert_eq!(def_pts.len(), 121, "meses -120..=0: {}", def_pts.len());
    // Y los markers fuera de la ventana no viajan (el ancla remota queda fuera).
    assert_eq!(def["markers"].as_array().unwrap().len(), 1, "{def}");

    // Petición explícita del histórico completo: 1200 es el máximo y significa «todo».
    let todo = get(&app, &owner.cookie, "/v1/history/series?window_months=1200").await;
    assert_eq!(todo["window_months"], 1200, "{todo}");
    assert_eq!(todo["window_truncated"], false, "ya no queda nada fuera: {todo}");
    assert_eq!(todo["points"].as_array().unwrap().len(), 181, "meses -180..=0");
    assert_eq!(todo["markers"].as_array().unwrap().len(), 2, "{todo}");
}

/// Las series de chart se publican a 2 decimales y `month_fraction` a 4. Antes viajaban con la
/// escala completa de `rust_decimal` cruzada a f64 (`78012.333333333333`), que ni el chart usa ni
/// la interpolación justifica: son tokens y precisión inventada.
#[tokio::test]
async fn history_chart_values_are_published_with_two_decimals() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let anchor = server_anchor(&app, &owner.cookie).await;

    // 100 → 400 sobre tres meses: la interpolación por días civiles cae en periódicos infinitos.
    let item = Uuid::new_v4().to_string();
    backfill(&app, &owner, "asset", add_months_signed(anchor, -4).succ_opt().unwrap(),
        serde_json::json!([{"item_id": item, "label": "Cash", "value": "100"}])).await;
    backfill(&app, &owner, "asset", add_months_signed(anchor, -1).succ_opt().unwrap(),
        serde_json::json!([{"item_id": item, "label": "Cash", "value": "400"}])).await;

    let body = get(&app, &owner.cookie, "/v1/history/series?window_months=1200").await;

    for p in body["points"].as_array().unwrap() {
        for campo in ["net_worth", "assets_total", "liabilities_total"] {
            let v = &p[campo];
            if v.is_null() {
                continue;
            }
            assert!(
                decimals_of(v) <= 2,
                "points[].{campo} debe publicarse a 2 decimales, llegó {v}"
            );
        }
    }
    for s in body["asset_series"].as_array().unwrap() {
        for v in s["values"].as_array().unwrap() {
            assert!(decimals_of(v) <= 2, "asset_series values a 2 decimales, llegó {v}");
        }
    }
    for m in body["markers"].as_array().unwrap() {
        assert!(decimals_of(&m["total"]) <= 2, "marker total: {m}");
        assert!(
            decimals_of(&m["month_fraction"]) <= 4,
            "month_fraction a 4 decimales: {m}"
        );
    }
    // Y que de verdad había algo que redondear: al menos un punto con parte decimal.
    assert!(
        body["points"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| decimals_of(&p["assets_total"]) > 0),
        "el escenario debía producir valores no enteros: {body}"
    );
}

/// Un `backfill` puede estar en cualquier fecha pasada, incluso remota; una `capture` es una foto
/// que la app tomó. Entre los markers las dos se presentaban igual, así que a «¿cuándo empecé a
/// ahorrar?» la serie contestaba con la fecha del ancla que el usuario tecleó.
#[tokio::test]
async fn markers_declare_capture_versus_backfill() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let anchor = server_anchor(&app, &owner.cookie).await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "Bolsa", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;

    backfill(&app, &owner, "asset", add_months_signed(anchor, -6),
        serde_json::json!([{"item_id": Uuid::new_v4().to_string(), "label": "Cash", "value": "1"}]))
        .await;
    let cap = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({"kinds": ["asset"]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(cap.status, http::StatusCode::OK, "capture: {cap:?}");

    let body = get(&app, &owner.cookie, "/v1/history/series?window_months=1200").await;
    let markers = body["markers"].as_array().unwrap();
    assert_eq!(markers.len(), 2, "{body}");
    let antiguo = markers.iter().min_by_key(|m| m["month_index"].as_i64().unwrap()).unwrap();
    assert_eq!(antiguo["source"], "backfill", "el ancla remota es un backfill: {antiguo}");
    // Un backfill de importe simbólico en una fecha remota se reconoce por lo que es: `source`
    // + `total` juntos. Sin `source`, ese punto se presentaba igual que una foto real.
    assert!(antiguo["total"].as_f64().unwrap() < 10.0, "{antiguo}");
    let reciente = markers.iter().max_by_key(|m| m["month_index"].as_i64().unwrap()).unwrap();
    assert_eq!(reciente["source"], "capture", "la foto de hoy es una capture: {reciente}");
}

// ---------------------------------------------------------------------------
// 6. Cash-flow: por qué falta la curva fina
// ---------------------------------------------------------------------------

/// La curva fina son ~520 puntos POR ACTIVO en el peor caso (120 meses weekly): es lo más caro del
/// catálogo. Se acota la ventana con la que se sirve, pero **sin romper la llamada** — el agregado
/// mensual sigue llegando entero y la respuesta dice por qué la curva no está. Antes, las tres
/// causas de ausencia producían exactamente el mismo JSON.
#[tokio::test]
async fn cashflow_says_why_the_fine_curve_is_missing() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // (a) Sin transacciones ligadas a un activo no hay nada que moldee la curva.
    let vacia = get(&app, &owner.cookie, "/v1/history/cashflow").await;
    assert!(vacia["fine"].is_null(), "{vacia}");
    assert_eq!(
        vacia["fine_absent_reason"], "no_asset_linked_transactions",
        "{vacia}"
    );
    assert!(
        !vacia["months"].as_array().unwrap().is_empty(),
        "el agregado mensual llega igual: {vacia}"
    );

    // (b) Ventana por encima del tope de la curva: el agregado mensual cubre los 120 meses
    //     pedidos y la curva se omite con su razón, en vez de un 400 que obligaría a reintentar.
    let ancha = get(&app, &owner.cookie, "/v1/history/cashflow?window_months=120").await;
    assert_eq!(
        ancha["fine_absent_reason"], "window_too_large_for_curve",
        "el tope de la curva se declara, no se aplica en silencio: {ancha}"
    );
    assert_eq!(
        ancha["months"].as_array().unwrap().len(),
        121,
        "la ventana mensual pedida se sirve entera: {ancha}"
    );

    // (c) Con movimientos ligados a un activo pero sin snapshots, no hay ancla para la curva.
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat_ast, "name": "Indexado", "current_value": "5000"}),
            &owner.cookie,
        )
        .await;
    let asset_id = asset.json()["id"].as_str().unwrap().to_string();
    let hoy = chrono::Utc::now().date_naive();
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            serde_json::json!({
                "op_date": hoy.format("%Y-%m-%d").to_string(),
                "concept": "Aportación",
                "amount": "-200",
                "kind": "savings",
                "linked_asset_id": asset_id,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "savings txn: {r:?}");

    let sin_ancla = get(&app, &owner.cookie, "/v1/history/cashflow").await;
    assert_eq!(sin_ancla["fine_absent_reason"], "no_snapshots_to_anchor", "{sin_ancla}");

    // (d) Con snapshot, la curva llega y la razón desaparece.
    let cap = app
        .post_json_with_cookie(
            "/v1/history/snapshots/capture",
            serde_json::json!({"kinds": ["asset"]}),
            &owner.cookie,
        )
        .await;
    assert_eq!(cap.status, http::StatusCode::OK, "capture: {cap:?}");

    let con_curva = get(&app, &owner.cookie, "/v1/history/cashflow").await;
    assert!(!con_curva["fine"].is_null(), "{con_curva}");
    assert!(
        con_curva["fine_absent_reason"].is_null(),
        "con curva no hay razón de ausencia: {con_curva}"
    );
    // Y la curva también se publica a 2 decimales / 4 en la fracción.
    for g in con_curva["fine"]["grid"].as_array().unwrap() {
        assert!(decimals_of(&g["month_fraction"]) <= 4, "{g}");
    }
    for s in con_curva["fine"]["asset_series"].as_array().unwrap() {
        for v in s["values"].as_array().unwrap() {
            assert!(decimals_of(v) <= 2, "curva fina a 2 decimales, llegó {v}");
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Snapshots: detalle suprimido ≠ snapshot vacío
// ---------------------------------------------------------------------------

/// `items: []` significaba dos cosas distintas: «este snapshot no tiene ítems» y «no te he mandado
/// el detalle». `item_count` + `items_included` las separan; sobre HTTP el detalle siempre viaja.
#[tokio::test]
async fn snapshots_distinguish_suppressed_detail_from_an_empty_snapshot() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let anchor = server_anchor(&app, &owner.cookie).await;

    backfill(&app, &owner, "asset", add_months_signed(anchor, -3),
        serde_json::json!([
            {"item_id": Uuid::new_v4().to_string(), "label": "Cash", "value": "100"},
            {"item_id": Uuid::new_v4().to_string(), "label": "Fondo", "value": "900"},
        ]))
        .await;

    let list = get(&app, &owner.cookie, "/v1/history/snapshots").await;
    let arr = list.as_array().expect("HTTP sigue devolviendo un array plano");
    assert_eq!(arr.len(), 1, "{list}");
    let s = &arr[0];
    assert_eq!(s["items_included"], true, "sobre HTTP el detalle siempre viaja: {s}");
    assert_eq!(s["item_count"], 2, "{s}");
    assert_eq!(s["items"].as_array().unwrap().len(), 2, "{s}");
    assert_eq!(dec(&s["total"]), 1000.0, "{s}");
}

// ---------------------------------------------------------------------------
// 8. Imports: el doble import que sus propios datos delataban
// ---------------------------------------------------------------------------

/// Subir el mismo extracto dos veces deja dos lotes con el mismo `original_filename`. El dato ya
/// estaba en la respuesta, pero exigía que el consumidor lo cruzara solo — y ninguno lo hacía.
#[tokio::test]
async fn import_batches_point_at_their_possible_duplicates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Dos lotes con el mismo nombre de fichero y sin cuenta vinculada, más uno distinto.
    let iid = app.installation_id().await;
    let mut ids: Vec<Uuid> = Vec::new();
    for (name, filename) in [
        ("dup-1", "enero.csv"),
        ("dup-2", "enero.csv"),
        ("otro", "febrero.csv"),
    ] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO transaction_imports (installation_id, owner_user_id, source, original_filename)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(iid)
        .bind(owner.user_id)
        .bind(name)
        .bind(filename)
        .fetch_one(&app.pool)
        .await
        .expect("insert import");
        ids.push(id);
    }

    let list = get(&app, &owner.cookie, "/v1/transactions/imports").await;
    let arr = list.as_array().expect("HTTP sigue devolviendo un array plano");
    assert_eq!(arr.len(), 3, "{list}");

    let by_id = |id: Uuid| -> serde_json::Value {
        arr.iter()
            .find(|b| b["id"] == serde_json::json!(id.to_string()))
            .unwrap_or_else(|| panic!("lote {id} en {list}"))
            .clone()
    };

    // La relación es simétrica: cada gemelo apunta al otro, así que basta con mirar una fila.
    let a = by_id(ids[0]);
    let b = by_id(ids[1]);
    assert_eq!(
        a["possible_duplicate_of"],
        serde_json::json!([ids[1].to_string()]),
        "{a}"
    );
    assert_eq!(
        b["possible_duplicate_of"],
        serde_json::json!([ids[0].to_string()]),
        "{b}"
    );
    // Y un fichero distinto no es sospechoso de nada.
    let c = by_id(ids[2]);
    assert_eq!(c["possible_duplicate_of"], serde_json::json!([]), "{c}");
}

// ---------------------------------------------------------------------------
// 9. Proyección: eventos que explican los saltos de la densidad híbrida
// ---------------------------------------------------------------------------

/// Con `density=hybrid` (la que sirve la tool MCP) entre dos puntos consecutivos caben doce meses,
/// y una caída de decenas de miles de euros entre ellos no tenía en la respuesta nada que la
/// explicara. Los eventos cuestan ~90 bytes cada uno y contestan la pregunta entera; subir la
/// densidad multiplica el payload por ~5 y sigue sin decir POR QUÉ.
#[tokio::test]
async fn projection_publishes_the_dated_planning_flows_that_move_the_curve() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_exp = app.create_category(&owner, "expense", "Grandes gastos").await;
    let cat_inc = app.create_category(&owner, "income", "Extras").await;

    let anchor = chrono::Utc::now().date_naive();
    let dentro = add_months_signed(anchor, 30); // Más allá del tramo mensual de `hybrid`.
    // 45 días atrás cae SIEMPRE antes del día 1 del mes ancla (ningún mes tiene 45 días).
    let vencido = anchor - chrono::Duration::days(45);
    let flujos = [
        (&cat_exp, "Reforma", "98000", Some(dentro)),
        (&cat_inc, "Herencia", "20000", Some(add_months_signed(anchor, 5))),
        (&cat_exp, "Sin fecha", "300", None),
        (&cat_exp, "Atrasado", "3000", Some(vencido)),
    ];
    for (cat, title, amount, due) in flujos {
        let mut body = serde_json::json!({
            "category_id": cat, "title": title, "expected_amount": amount
        });
        if let Some(d) = due {
            body["due_date"] = serde_json::json!(d.format("%Y-%m-%d").to_string());
        }
        let r = app
            .post_json_with_cookie("/v1/planning/flows", body, &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "planning flow: {r:?}");
    }

    let proj = get(&app, &owner.cookie, "/v1/projection/series?density=hybrid").await;
    let events = proj["events"].as_array().expect("events array");
    assert_eq!(proj["events_truncated"], false, "{proj}");
    // Los TRES con fecha (el vencido incluido, #126): el sin-fecha se reparte sobre 90 días y no
    // produce escalón, así que llamarlo «evento» sería señalar una rampa.
    assert_eq!(events.len(), 3, "solo los flujos con fecha: {events:?}");

    // #126: el vencido carga en el mes ancla con su fecha REAL y el flag que declara la
    // discordancia mes-señalado ≠ fecha-mostrada. Los no vencidos llevan overdue: false — el
    // campo distingue los dos casos, no decora.
    let atrasado = events.iter().find(|e| e["title"] == "Atrasado").expect("Atrasado");
    assert_eq!(atrasado["month_index"], 0, "{atrasado}");
    assert_eq!(atrasado["overdue"], true, "{atrasado}");
    assert_eq!(
        atrasado["date_ymd"],
        serde_json::json!(vencido.format("%Y-%m-%d").to_string()),
        "{atrasado}"
    );

    let reforma = events.iter().find(|e| e["title"] == "Reforma").expect("Reforma");
    assert_eq!(reforma["direction"], "outflow", "{reforma}");
    assert_eq!(reforma["overdue"], false, "{reforma}");
    assert_eq!(dec(&reforma["amount"]), 98_000.0, "magnitud ≥ 0: {reforma}");
    assert_eq!(
        reforma["date_ymd"],
        serde_json::json!(dentro.format("%Y-%m-%d").to_string()),
        "{reforma}"
    );

    let herencia = events.iter().find(|e| e["title"] == "Herencia").expect("Herencia");
    assert_eq!(herencia["direction"], "inflow", "{herencia}");

    // Orden cronológico: es lo que permite alinear un evento con el hueco entre dos puntos.
    let idx: Vec<i64> = events.iter().map(|e| e["month_index"].as_i64().unwrap()).collect();
    assert!(idx.windows(2).all(|w| w[0] <= w[1]), "orden por month_index: {idx:?}");

    // Y el mes del evento cae DENTRO de un hueco de la densidad híbrida: los puntos servidos son
    // anuales a partir del 24, así que el mes 30 no es ninguno de ellos. Ésa es la razón de ser
    // del array — sin él, ese salto no tiene explicación en la respuesta.
    let servidos: Vec<i64> = proj["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["month_index"].as_i64().unwrap())
        .collect();
    let mes_reforma = reforma["month_index"].as_i64().unwrap();
    assert!(mes_reforma >= 24, "el escenario debe caer en el tramo anual: {mes_reforma}");
    assert!(
        !servidos.contains(&mes_reforma),
        "el mes del evento NO es un punto servido en hybrid ({mes_reforma} en {servidos:?})"
    );
}

/// #178 — `drawdown_gain_basis` distingue los TRES regímenes que antes eran indistinguibles
/// (regla de la casa: un campo de contexto se prueba forzando las ramas que existe para
/// separar), y `taxable_gain_ratio_today` publica la g₀ informativa SOLO cuando hay coste
/// declarado del que derivarla.
#[tokio::test]
async fn projection_declares_what_governs_the_drawdown_taxation() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Fondos").await;

    let mk = |name: &str, value: &str| {
        serde_json::json!({ "category_id": cat, "name": name, "current_value": value,
                            "is_liquid": true })
    };
    let a = app.post_json_with_cookie("/v1/assets", mk("A", "10000"), &owner.cookie).await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
    let a_id = a.json()["id"].as_str().unwrap().to_string();
    let b = app.post_json_with_cookie("/v1/assets", mk("B", "5000"), &owner.cookie).await;
    assert_eq!(b.status, http::StatusCode::CREATED, "{b:?}");
    let b_id = b.json()["id"].as_str().unwrap().to_string();

    // Caso 1: nadie declara coste → rige el escalar, y la g₀ informativa NO se fabrica.
    let proj = get(&app, &owner.cookie, "/v1/projection/series").await;
    assert_eq!(proj["drawdown_gain_basis"], "declared_ratio", "{proj}");
    assert!(proj["taxable_gain_ratio_today"].is_null(), "{proj}");

    // Caso 2: un activo de dos declara → mixed, y la g₀ sale SOLO de lo declarado:
    // A(10.000, coste 8.000) ⇒ 2.000/10.000 = 0,2 (B no contamina la cifra).
    let p = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{a_id}"),
            serde_json::json!({ "purchase_price": "8000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    let proj = get(&app, &owner.cookie, "/v1/projection/series").await;
    assert_eq!(proj["drawdown_gain_basis"], "mixed", "{proj}");
    assert_eq!(dec(&proj["taxable_gain_ratio_today"]), 0.2, "{proj}");

    // Caso 3: todos declaran → cost_basis; g₀ agregada = (2.000 + 4.000)/15.000 = 0,4.
    let p = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{b_id}"),
            serde_json::json!({ "purchase_price": "1000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::OK, "{p:?}");
    let proj = get(&app, &owner.cookie, "/v1/projection/series").await;
    assert_eq!(proj["drawdown_gain_basis"], "cost_basis", "{proj}");
    assert_eq!(dec(&proj["taxable_gain_ratio_today"]), 0.4, "{proj}");
}
