//! Cuotas de pasivo dentro de `GET /v1/budget` — cobertura dedicada.
//!
//! **3.7.0**: la cuota dejó de ser un bloque aparte (`derived_from_liabilities`, sumado por
//! debajo del presupuesto) y pasó a ser **una partida más de `entries`**, marcada
//! `source = "liability"` y atribuida a la categoría de GASTO que declara el pasivo
//! (`expense_category_id`) — la misma con la que la comparativa de Movimientos empareja los
//! recibos reales. Consecuencias que estos tests clavan:
//!
//! - `totals.expense_regular_monthly_equivalent` incluye las cuotas y es exactamente la suma de
//!   los `entries` de scope `expense`; `expense_total_*` vale lo mismo.
//! - `totals.expense_derived_monthly_equivalent` y `derived_from_liabilities` **ya no existen**.
//! - `expense_retirement_monthly_equivalent` NO recibe la cuota (termina con su plan de pago) —
//!   es el campo que alimenta la previa FIRE, con incidente propio (v1.3.0, divergencia 2–3×).
//! - La base de gasto del **engine** sigue siendo solo lo persistido: el engine cobra la cuota
//!   por su lado (`ProjectionLiabilityInput::monthly_payment`), así que fundirla también ahí la
//!   contaría dos veces en toda la proyección del modo A.
//!
//! Contrato del predicado «pasivo activo», unificado en 3.4.0 con el resto del sistema:
//! `payment_amount IS NOT NULL AND payment_frequency IS NOT NULL AND
//!  (payment_end_date IS NULL OR payment_end_date >= today)`.
//! Antes esta query era el único outlier (exigía fecha fin NOT NULL y `>` estricto): un pasivo sin
//! fecha fin no generaba línea derivada aunque el engine sí cobrara su cuota en modo A.

mod common;

use chrono::{Duration, NaiveDate};
use common::TestApp;
use serde_json::{json, Value};

fn parse_dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("expected decimal string, got {v:?}"))
        .parse::<f64>()
        .expect("parse decimal string")
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 0.01, "expected ~{b}, got {a}");
}

/// «Hoy» del SERVIDOR (ancla de /v1/history/series, en el calendar_tz de la instalación) — no el
/// del reloj UTC de la máquina, para que el test del borde `>=` no flaquee cerca de medianoche.
async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

async fn create_liability(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    exp_cat: &str,
    label: &str,
    body_extra: Value,
) {
    let mut body = json!({ "category_id": cat, "expense_category_id": exp_cat, "label": label,
                           "principal": "10000" });
    for (k, v) in body_extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let r = app.post_json_with_cookie("/v1/liabilities", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability {label}: {r:?}");
}

async fn budget_entry(app: &TestApp, cookie: &str, cat: &str, amount: &str, extra: Value) {
    let mut body = json!({ "category_id": cat, "amount": amount });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let r = app.post_json_with_cookie("/v1/budget/entries", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "budget entry: {r:?}");
}

async fn budget_snapshot(app: &TestApp, cookie: &str, query: &str) -> Value {
    let resp = app.get_with_cookie(query, cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "budget: {resp:?}");
    resp.json()
}

/// Partidas de gasto marcadas como cuota de pasivo, en el orden en que las sirve el servidor.
fn quota_entries(body: &Value) -> Vec<&Value> {
    body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source"] == "liability")
        .collect()
}

fn quota_labels(body: &Value) -> Vec<&str> {
    quota_entries(body)
        .iter()
        .map(|e| e["label"].as_str().unwrap())
        .collect()
}

fn expense_entries_sum(body: &Value) -> f64 {
    body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["scope"] == "expense")
        .map(|e| parse_dec(&e["amount"]))
        .sum()
}

// ---------------------------------------------------------------------------
// La fusión: forma de la respuesta y totales
// ---------------------------------------------------------------------------

/// La cuota es una partida de gasto normal: vive en `entries`, cuelga de la categoría de GASTO del
/// pasivo, se marca `source = "liability"` y suma dentro de `expense_regular_*`. El bloque
/// derivado y su total desaparecieron del contrato.
#[tokio::test]
async fn quota_is_a_regular_expense_entry_in_its_category() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Hogar").await;

    // Partida manual en la MISMA categoría que la cuota: deben convivir como dos filas, no
    // fundirse en una — el desfase entre plan y cuota es información, no ruido a sumar.
    budget_entry(&app, &owner.cookie, &exp_cat, "100", json!({})).await;
    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "IKEA",
        json!({ "payment_amount": "274", "payment_frequency": "monthly" }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;

    assert!(body.get("derived_from_liabilities").is_none(),
        "el bloque derivado ya no existe en el contrato: {body:?}");
    assert!(body["totals"].get("expense_derived_monthly_equivalent").is_none(),
        "el total derivado ya no existe en el contrato: {body:?}");

    let quotas = quota_entries(&body);
    assert_eq!(quotas.len(), 1, "una cuota: {body:?}");
    let q = quotas[0];
    assert_eq!(q["category_id"].as_str().unwrap(), exp_cat,
        "la cuota cuelga de la categoría de GASTO del pasivo");
    assert_eq!(q["scope"], "expense");
    assert_eq!(q["label"], "IKEA");
    assert_eq!(q["liability_id"], q["id"], "id de la partida = id del pasivo que la genera");
    approx(parse_dec(&q["amount"]), 274.0);

    // Dos filas en la misma categoría, no una fila de 374.
    let in_cat: Vec<&Value> = body["entries"].as_array().unwrap().iter()
        .filter(|e| e["category_id"].as_str() == Some(exp_cat.as_str()))
        .collect();
    assert_eq!(in_cat.len(), 2, "manual + cuota conviven como dos partidas: {body:?}");

    let totals = &body["totals"];
    approx(parse_dec(&totals["expense_regular_monthly_equivalent"]), 374.0);
    approx(parse_dec(&totals["expense_total_monthly_equivalent"]), 374.0);
    approx(expense_entries_sum(&body), 374.0);
    approx(parse_dec(&totals["net_monthly_equivalent"]), -374.0);
}

/// Las partidas persistidas siguen marcándose `manual` y conservan su id, categoría y banderas.
#[tokio::test]
async fn manual_entries_keep_their_shape() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let exp_cat = app.create_category(&owner, "expense", "Alquiler").await;
    budget_entry(&app, &owner.cookie, &exp_cat, "860", json!({})).await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["source"], "manual");
    assert!(entries[0].get("liability_id").is_none(), "una partida manual no lleva pasivo");
    assert!(entries[0].get("label").is_none());
    assert_eq!(entries[0]["category_id"].as_str().unwrap(), exp_cat);
}

/// La cuota NO entra en `expense_retirement_monthly_equivalent`: termina con su plan de pago, así
/// que no es gasto post-jubilación. Ese campo alimenta la previa FIRE de `RetirementView`.
#[tokio::test]
async fn quota_stays_out_of_the_retirement_expense_base() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Hogar").await;

    budget_entry(&app, &owner.cookie, &exp_cat, "1000", json!({})).await;
    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Cuota",
        json!({ "payment_amount": "200", "payment_frequency": "monthly" }),
    )
    .await;

    let totals = budget_snapshot(&app, &owner.cookie, "/v1/budget").await["totals"].clone();
    approx(parse_dec(&totals["expense_regular_monthly_equivalent"]), 1200.0);
    approx(parse_dec(&totals["expense_retirement_monthly_equivalent"]), 1000.0);
}

/// **Regresión del doble conteo en la proyección.** `monthly_delta_assumption` es
/// «ingresos regulares − gastos regulares» tal y como los recibe el engine: la cuota NO puede
/// estar ahí, porque el engine ya la cobra aparte vía `ProjectionLiabilityInput::monthly_payment`
/// (con amortización y fecha de fin de plan). Números a mano: presupuesto 3.000 − 1.000 = **2.000**.
/// Si alguien fundiera la cuota también en la base del engine, este valor caería a 1.800 y la
/// proyección del modo A restaría 200 €/mes dos veces, en silencio.
#[tokio::test]
async fn liability_quota_stays_out_of_the_engine_expense_base() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let inc_cat = app.create_category(&owner, "income", "Nómina").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Hogar").await;

    budget_entry(&app, &owner.cookie, &inc_cat, "3000", json!({})).await;
    budget_entry(&app, &owner.cookie, &exp_cat, "1000", json!({})).await;
    // Principal alto y sin fecha fin: la cuota nunca amortiza a cero dentro del tramo medido, así
    // que el patrimonio es lineal y se puede predecir en cerrado.
    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Préstamo",
        json!({ "principal": "100000", "payment_amount": "200", "payment_frequency": "monthly" }),
    )
    .await;

    // El presupuesto SÍ la cuenta: 1.000 + 200.
    let totals = budget_snapshot(&app, &owner.cookie, "/v1/budget").await["totals"].clone();
    approx(parse_dec(&totals["expense_regular_monthly_equivalent"]), 1200.0);

    let resp = app
        .get_with_cookie("/v1/projection/series?months=240", &owner.cookie)
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "projection: {resp:?}");
    let proj = resp.json();

    approx(
        parse_dec(&proj["monthly_delta_assumption"]),
        2000.0,
    );

    // Y el engine sigue cobrándola exactamente UNA vez. Caja del mes = 3.000 − 1.000 − 200 =
    // 1.800 — pero desde 4.12.1 este escenario SIN ACTIVOS no acumula ese sobrante (decisión 3:
    // varado y declarado): el patrimonio publicado es solo el principal amortizándose,
    // NW(k) = 200k − 100.000. La prueba de «la cuota se cobra UNA vez» sobrevive intacta: si se
    // cobrara dos, la amortización sería 400/mes y NW(12) sería −95.200, no −97.600.
    approx(parse_dec(&proj["starting_net_worth"]), -100_000.0);
    let points = proj["points"].as_array().unwrap();
    let nw_12 = points[12]["net_worth"].as_f64().unwrap();
    assert!(
        (nw_12 - (-97_600.0)).abs() < 1.0,
        "NW(12) = 200·12 − 100.000 = −97.600 (cuota contada una vez; sobrante varado); got {nw_12}"
    );
    let varado = parse_dec(&proj["unallocated_savings_total"]);
    assert!(varado > 0.0, "el 1.800/mes varado se declara: {varado}");
}

// ---------------------------------------------------------------------------
// Predicado «pasivo activo» (contrato de 3.4.0, intacto tras la fusión)
// ---------------------------------------------------------------------------

/// Fecha fin NULL = plan indefinido → SÍ genera partida (regresión de la reforma 3.4.0; antes
/// quedaba excluida y el modo A no contaba la cuota «una vez» de forma consistente).
#[tokio::test]
async fn quota_includes_null_end_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Sin fecha fin",
        json!({ "payment_amount": "500", "payment_frequency": "monthly" }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    let quotas = quota_entries(&body);
    assert_eq!(quotas.len(), 1, "el pasivo sin fecha fin debe dar partida: {body:?}");
    approx(parse_dec(&quotas[0]["amount"]), 500.0);
    assert_eq!(quotas[0]["category_id"].as_str().unwrap(), exp_cat);
    assert!(quotas[0]["expense_end_date"].is_null(), "plan indefinido → sin fecha de fin");
    approx(parse_dec(&body["totals"]["expense_regular_monthly_equivalent"]), 500.0);
    approx(parse_dec(&body["totals"]["expense_total_monthly_equivalent"]), 500.0);
}

/// Pasivo vencido (fecha fin pasada) → sin partida; borde `>=`: el día EXACTO de fin aún cuenta
/// (mismo criterio que /v1/liabilities y /v1/summary), y se publica como `expense_end_date`.
#[tokio::test]
async fn quota_excludes_expired_but_includes_end_today() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = server_today(&app, &owner.cookie).await;
    let past = (today - Duration::days(5)).format("%Y-%m-%d").to_string();
    let today_s = today.format("%Y-%m-%d").to_string();

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Vencido",
        json!({ "payment_amount": "700", "payment_frequency": "monthly", "payment_end_date": past }),
    )
    .await;
    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Termina hoy",
        json!({ "payment_amount": "200", "payment_frequency": "monthly", "payment_end_date": today_s }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    assert_eq!(quota_labels(&body), vec!["Termina hoy"], "solo el que termina hoy: {body:?}");
    assert_eq!(quota_entries(&body)[0]["expense_end_date"].as_str().unwrap(), today_s,
        "la partida publica el fin del plan de pago");
    approx(parse_dec(&body["totals"]["expense_regular_monthly_equivalent"]), 200.0);
}

/// Sin plan de pago (payment_amount/frequency NULL) → sin partida aunque el pasivo exista.
#[tokio::test]
async fn quota_requires_payment_plan() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat, "Solo principal", json!({})).await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    assert!(quota_entries(&body).is_empty(), "sin plan de pago no hay partida: {body:?}");
    approx(parse_dec(&body["totals"]["expense_regular_monthly_equivalent"]), 0.0);
}

/// Cuota semanal → equivalente mensual ×52/12 (70 €/semana ≈ 303,33 €/mes). El presupuesto es
/// mensual: la partida publica ya el equivalente, no el importe crudo del plan.
#[tokio::test]
async fn quota_weekly_monthly_equivalent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Semanal",
        json!({ "payment_amount": "70", "payment_frequency": "weekly" }),
    )
    .await;

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    approx(parse_dec(&quota_entries(&body)[0]["amount"]), 70.0 * 52.0 / 12.0);
    approx(
        parse_dec(&body["totals"]["expense_regular_monthly_equivalent"]),
        70.0 * 52.0 / 12.0,
    );
}

/// Scoping: `?view=mine` deja solo los pasivos del solicitante; household, los de todos.
#[tokio::test]
async fn quota_household_vs_mine_scoping() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Del owner",
        json!({ "payment_amount": "500", "payment_frequency": "monthly" }),
    )
    .await;
    create_liability(&app, &member.cookie, &cat, &exp_cat,
        "Del member",
        json!({ "payment_amount": "300", "payment_frequency": "monthly" }),
    )
    .await;

    let hh = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    approx(parse_dec(&hh["totals"]["expense_regular_monthly_equivalent"]), 800.0);

    let mine = budget_snapshot(&app, &owner.cookie, "/v1/budget?view=mine").await;
    approx(parse_dec(&mine["totals"]["expense_regular_monthly_equivalent"]), 500.0);
    assert_eq!(quota_labels(&mine), vec!["Del owner"]);
}

/// Pasivo SIN categoría de gasto asignada (estado de los anteriores a 3.4.0, y de los importados
/// de un `.ffbackup` viejo): la partida sigue existiendo y **sigue sumando** en los totales, pero
/// omite `category_id`. Descartarla bajaría el gasto presupuestado en silencio, que es justo el
/// modo de fallo que esta reforma quiere evitar.
#[tokio::test]
async fn quota_without_expense_category_still_counts() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    create_liability(&app, &owner.cookie, &cat, &exp_cat,
        "Heredado",
        json!({ "payment_amount": "150", "payment_frequency": "monthly" }),
    )
    .await;
    // Estado pre-3.4.0: la columna es nullable y su FK es ON DELETE SET NULL.
    sqlx::query("UPDATE liabilities SET expense_category_id = NULL")
        .execute(&app.pool)
        .await
        .expect("desasignar la categoría de gasto");

    let body = budget_snapshot(&app, &owner.cookie, "/v1/budget").await;
    let quotas = quota_entries(&body);
    assert_eq!(quotas.len(), 1, "la cuota sin categoría sigue existiendo: {body:?}");
    assert!(quotas[0].get("category_id").is_none(),
        "sin categoría asignada el campo se omite: {body:?}");
    assert_eq!(quotas[0]["label"], "Heredado");
    approx(parse_dec(&body["totals"]["expense_regular_monthly_equivalent"]), 150.0);
    approx(expense_entries_sum(&body), 150.0);
}
