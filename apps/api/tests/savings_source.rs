//! Toggle `savings_source` (presupuesto vs promedio real 12m) — Fases 1-3.
//!
//! Números PREDICHOS antes de ejecutar (ver comentarios por test). El "hoy" se deriva del servidor
//! (`/v1/history/series` anchor) para no depender del reloj de la máquina. Todas las lecturas de
//! proyección usan `?months=240` para saltarse la cache (el hot-path solo cachea cuando `months`
//! está ausente) y así ver siempre el efecto del modo/settings vigentes sin condiciones de carrera.

mod common;

use chrono::{Datelike, NaiveDate};
use common::TestApp;
use serde_json::{json, Value};

fn parse_dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("expected decimal string, got {v:?}"))
        .parse::<f64>()
        .expect("parse decimal string")
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 0.5, "expected ~{b}, got {a}");
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

#[allow(clippy::too_many_arguments)]
async fn manual(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
    cat: Option<&str>,
    linked_liability: Option<&str>,
) {
    let mut body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind });
    if let Some(c) = cat {
        body["category_id"] = json!(c);
    }
    if let Some(l) = linked_liability {
        body["linked_liability_id"] = json!(l);
    }
    let r = app.post_json_with_cookie("/v1/transactions", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "manual {concept}: {r:?}");
}

async fn budget(app: &TestApp, cookie: &str, cat: &str, amount: &str) {
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({ "category_id": cat, "amount": amount }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "budget: {r:?}");
}

/// Alta con recurrencia: crea la regla + su instancia de origen (y backfillea los meses intermedios
/// hasta hoy dentro del propio commit del create). El origen y el backfill quedan con
/// `recurring_rule_id NOT NULL` → son «pseudovacíos» a efectos del promedio.
async fn recurring(
    app: &TestApp,
    cookie: &str,
    date: &str,
    concept: &str,
    amount: &str,
    kind: &str,
) {
    let body = json!({ "op_date": date, "concept": concept, "amount": amount, "kind": kind,
                       "recurrence": {} });
    let r = app.post_json_with_cookie("/v1/transactions", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "recurring {concept}: {r:?}");
}

/// PATCH mode B (mínimo: solo `savings_source`; el resto de `FireSettings` cae al default de struct).
async fn set_mode_b(app: &TestApp, cookie: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "transactions_avg" } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode B: {r:?}");
}

/// PATCH de las ventanas del promedio real (ambas en modo `calendar`).
async fn set_windows(app: &TestApp, cookie: &str, source: &str, income_m: u32, expense_m: u32) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": {
                "savings_source": source,
                "income_avg_window_months": income_m,
                "income_avg_window_mode": "calendar",
                "expense_avg_window_months": expense_m,
                "expense_avg_window_mode": "calendar",
            }}),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set windows: {r:?}");
}

/// PATCH mode C (`budget_income_real_expense`): income del presupuesto + gasto real.
async fn set_mode_c(app: &TestApp, cookie: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "budget_income_real_expense" } }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set mode C: {r:?}");
}

async fn projection_delta(app: &TestApp, cookie: &str, query: &str) -> f64 {
    let resp = app.get_with_cookie(query, cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "projection: {resp:?}");
    parse_dec(&resp.json()["monthly_delta_assumption"])
}

/// `contribution_nominal_monthly` del primer (único) activo de GET /v1/assets.
async fn asset_contribution(app: &TestApp, cookie: &str) -> f64 {
    let resp = app.get_with_cookie("/v1/assets", cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "assets: {resp:?}");
    let body = resp.json();
    let first = body.as_array().and_then(|a| a.first()).expect("un activo");
    parse_dec(&first["contribution_nominal_monthly"])
}

// ---------------------------------------------------------------------------
// Fase 1 — serde / PATCH
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fire_settings_defaults_to_budget_when_absent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let resp = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK);
    assert_eq!(
        resp.json()["installation"]["fire_settings"]["savings_source"], "budget",
        "instalación nueva → savings_source ausente en JSON → budget"
    );
}

#[tokio::test]
async fn patch_savings_source_transactions_avg_roundtrips() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "transactions_avg" } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    assert_eq!(
        patched.json()["installation"]["fire_settings"]["savings_source"],
        "transactions_avg"
    );

    // Round-trip: GET fresco lo persiste.
    let got = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(got.json()["installation"]["fire_settings"]["savings_source"], "transactions_avg");
}

#[tokio::test]
async fn patch_savings_source_budget_income_real_expense_roundtrips() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "budget_income_real_expense" } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    assert_eq!(
        patched.json()["installation"]["fire_settings"]["savings_source"],
        "budget_income_real_expense"
    );

    let got = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(
        got.json()["installation"]["fire_settings"]["savings_source"],
        "budget_income_real_expense"
    );
}

#[tokio::test]
async fn patch_savings_source_unknown_returns_422() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": "nope" } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(
        resp.status,
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "valor desconocido debe ser 422, recibido {}",
        resp.status
    );
    let body_text = String::from_utf8_lossy(&resp.body);
    assert!(
        body_text.contains("unknown variant") && body_text.contains("nope"),
        "cuerpo debe nombrar la variante desconocida: {body_text}"
    );
    // El error debe listar las 3 variantes válidas (incluida la nueva de modo C).
    for expected in ["budget", "transactions_avg", "budget_income_real_expense"] {
        assert!(
            body_text.contains(expected),
            "el error debe listar la variante válida `{expected}`: {body_text}"
        );
    }
}

// ---------------------------------------------------------------------------
// Fase 2/3 — helper de promedio vía la proyección (monthly_delta_assumption)
// ---------------------------------------------------------------------------

/// Ponderación (hueco no diluye), exclusión de `savings`, ventana [−12m, mes actual): el mes
/// parcial y el mes −13 quedan fuera. Y el modo B se impone sobre el presupuesto.
#[tokio::test]
async fn mode_b_weighted_avg_excludes_savings_and_partial_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;

    // Presupuesto bien distinto → modo A = 5000 − 3000 = 2000.
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1); // último mes completo (en ventana)
    let (y6, m6) = shift_month(today.year(), today.month(), -6);
    let (y13, m13) = shift_month(today.year(), today.month(), -13); // fuera de ventana (antes)

    // Datos solo en −1 y −6 → months_with_data = 2 (los huecos intermedios no diluyen).
    manual(&app, &owner.cookie, &date_in(y1, m1, 15), "Sueldo", "2400", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 16), "Compra", "-900", "expense", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 17), "Aporte", "-500", "savings", None, None).await;
    manual(&app, &owner.cookie, &date_in(y6, m6, 15), "Sueldo", "1200", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y6, m6, 16), "Compra", "-300", "expense", None, None).await;
    // Ruido excluido: mes actual (parcial) y mes −13 (antes de la ventana).
    manual(&app, &owner.cookie, &today.format("%Y-%m-%d").to_string(), "Hoy", "9999", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y13, m13, 15), "Viejo", "8888", "income", None, None).await;

    // Modo A (default) = presupuesto.
    let delta_a = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_a, 2000.0);

    // Modo B con ventanas SIMÉTRICAS 12/12 (el comportamiento anterior a 3.9.0, reproducible):
    // income_avg = (2400+1200)/2 = 1800; expense_avg = (900+300)/2 = 600 → delta 1200.
    set_windows(&app, &owner.cookie, "transactions_avg", 12, 12).await;
    let delta_sym = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_sym, 1200.0);

    // Ventanas ASIMÉTRICAS 3/12 (el default de 3.9.0): el ingreso solo mira los últimos 3 meses
    // civiles, así que el mes −6 sale de SU ventana pero sigue contando para el gasto.
    // income_avg = 2400 (solo −1); expense_avg = (900+300)/2 = 600 → delta 1800.
    // Este es el par discriminante: con una sola ventana los dos casos darían lo mismo.
    set_windows(&app, &owner.cookie, "transactions_avg", 3, 12).await;
    let delta_asym = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_asym, 1800.0);

    assert!((delta_a - delta_sym).abs() > 100.0, "modo B debe cambiar la pendiente frente a A");
    assert!(
        (delta_asym - delta_sym).abs() > 100.0,
        "la ventana de ingreso debe mover la cifra de forma independiente a la de gasto"
    );
}

/// `months_with_data == 0` → fallback silencioso al presupuesto (modo A efectivo).
#[tokio::test]
async fn mode_b_zero_months_falls_back_to_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "2000").await;

    // Única transacción en el mes en curso (parcial) → fuera de ventana → months_with_data = 0.
    let today = server_today(&app, &owner.cookie).await;
    manual(&app, &owner.cookie, &today.format("%Y-%m-%d").to_string(), "Hoy", "9999", "income", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    let delta = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta, 3000.0); // presupuesto 5000 − 2000, NO tocado por la txn del mes parcial
}

/// Reforma 3.4.0: el promedio de gasto se usa CRUDO — las cuotas ya viven dentro de los
/// movimientos, así que ni los vínculos (`linked_liability_id`) ni las cuotas nominales de las
/// liabilities alteran el delta mensual. (Antes: resta híbrida 450 real + 300 nominal → 4750.)
#[tokio::test]
async fn mode_b_raw_avg_ignores_liability_links() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 5, 1, 15);
    let past = date_in(today.year() - 1, 6, 15);

    // L1 activa, cuota nominal 500 (pero tiene txns vinculadas → se usa el avg real 450).
    let l1 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat, "label": "L1", "principal": "100000",
                    "payment_amount": "500", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l1.status, http::StatusCode::CREATED, "{l1:?}");
    let l1_id = l1.json()["id"].as_str().unwrap().to_string();

    // L2 activa, sin txns vinculadas → se usa la cuota nominal 300.
    let l2 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat, "label": "L2", "principal": "100000",
                    "payment_amount": "300", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l2.status, http::StatusCode::CREATED, "{l2:?}");

    // L3 terminada (payment_end pasado) → NO se resta aunque tenga cuota 700.
    let l3 = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat, "label": "L3", "principal": "100000",
                    "payment_amount": "700", "payment_frequency": "monthly", "payment_end_date": past }),
            &owner.cookie,
        )
        .await;
    assert_eq!(l3.status, http::StatusCode::CREATED, "{l3:?}");

    // Transacciones en el último mes completo: income 6000; expense total 2000 (de los cuales 450
    // vinculados a L1 — el vínculo es metadata y no altera nada) → income_avg 6000, expense_avg 2000.
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "6000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Cuota L1", "-450", "expense", None, Some(&l1_id)).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 12), "Resto", "-1550", "expense", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    // INVERTIDO en 4.8.0 (#142, opción 3 firmada): la cuota DECLARADA de los planes vivos sale
    // del promedio — gasto efectivo = 2.000 − (500 + 300) = 1.200 y delta = 6.000 − 1.200 =
    // **4.800** (la cuota vuelve al motor como servicio de deuda real, con su vencimiento).
    // Sigue siendo la cuota DECLARADA, no la vinculada (L1 gira 450 reales): estimación acotada
    // y documentada. L3 (plan vencido) sigue SIN restarse — su cuota ya no se gira.
    // (Hasta 4.7.0 el promedio iba crudo: delta 4.000 y deuda congelada para siempre.)
    let delta = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta, 4800.0);
}

/// Reajustado en 4.12.1 (la muerte de `surplus_cash`): este escenario NO tiene activos, así
/// que el neto positivo del mes (ingreso 2.000 − cuota 1.000 = 1.000) ya no se acumula en caja
/// — queda VARADO y declarado (decisión 3 del owner: el modelo no simula dinero sin sitio). El
/// patrimonio publicado es SOLO la deuda amortizándose: NW(k) = 1.000·k − 100.000 mientras el
/// plan vive. El bucle se acota a k ≤ 50 porque el fin del plan (`today + 5 años`) hace el
/// tramo posterior dependiente del reloj — la meseta se comprueba por su existencia, no por su
/// mes exacto. Historia: en 4.8.0 (#142) el pin era 2.000·k − 100.000 porque el 1.000 sobrante
/// contaba como caja dentro del NW.
#[tokio::test]
async fn mode_b_liability_static_nw_subtraction() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let today = server_today(&app, &owner.cookie).await;
    let future = date_in(today.year() + 5, 1, 15);
    // Cuota nominal 1000 (se ignora en la caja del modo real), principal 100000.
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat, "label": "L", "principal": "100000",
                    "payment_amount": "1000", "payment_frequency": "monthly", "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // Solo income → income_avg = 2000, expense_avg = 0; months_with_data = 1.
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "2000", "income", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    // delta = income_avg − expense_avg = 2000; la cuota nominal NO se cobra.
    let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
    approx(parse_dec(&body["monthly_delta_assumption"]), 2000.0);

    // Serie sin activos (4.12.1): NW(k) = −principal(k) = 1.000·k − 100.000 mientras el plan
    // vive — el 1.000/mes que antes era caja está varado y fuera del balance. Exacto y estable
    // para k ≤ 50 (el plan de today+5 años vive al menos ~52 meses desde cualquier fecha).
    let points = body["points"].as_array().unwrap();
    assert!(points.len() > 200, "serie mensual esperada, {} puntos", points.len());
    for p in points {
        let k = p["month_index"].as_u64().unwrap() as f64;
        if k > 50.0 {
            break;
        }
        let nw = p["net_worth"].as_f64().unwrap();
        let expected = 1000.0 * k - 100_000.0;
        assert!(
            (nw - expected).abs() < 0.5,
            "mes {k}: net_worth {nw}, esperado {expected} (solo amortización; el resto, varado)"
        );
    }
    // Y el varado se DECLARA — a 240 meses es ≥ 50×1.000 (la cifra exacta depende del reloj
    // por el fin del plan; el arranque no).
    let varado = parse_dec(&body["unallocated_savings_total"]);
    assert!(varado >= 50_000.0, "unallocated_savings_total: {varado}");
    assert_eq!(body["unallocated_savings_reason"], "no_assets", "{body}");
}

/// Target FIRE modo B (annual_expense) usa el promedio de gasto crudo como base:
/// target = (expense_avg×12)/SWR.
#[tokio::test]
async fn mode_b_target_annual_expense_uses_expense_avg() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Solo transacciones: expense_avg = 1000, income_avg = 3000.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;

    // FIRE: annual_expense, SWR 4%, sin impuestos → target = (1000×12)/0.04 = 300000.
    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": {
                "taxes_enabled": false,
                "tax_brackets": [],
                "savings_source": "transactions_avg"
            } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    // 5.0.0 (D13): el modo del objetivo y el SWR son del PERFIL del usuario; la fuente del
    // ahorro y la fiscalidad siguen siendo del hogar. Mismos números, dos superficies.
    let patched = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "annual_expense", "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");

    let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
    let target = parse_dec(&body["jubilacion_target_net_worth"]);
    approx(target, 300_000.0);
}

/// Scoping household vs mine: cada vista promedia solo sus propias transacciones.
#[tokio::test]
async fn mode_b_household_vs_mine_scoping() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);

    // Ambos en el mismo mes → household months_with_data = 1.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Owner in", "2000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Owner out", "-800", "expense", None, None).await;
    manual(&app, &member.cookie, &date_in(y1, m1, 12), "Member in", "1000", "income", None, None).await;
    manual(&app, &member.cookie, &date_in(y1, m1, 13), "Member out", "-400", "expense", None, None).await;

    set_mode_b(&app, &owner.cookie).await;

    // household: income 3000, expense 1200 → delta 1800.
    // `household` explícito desde 5.0.0 (R2): sin él, el GET devuelve la vista `mine`.
    let hh = projection_delta(
        &app,
        &owner.cookie,
        "/v1/projection/series?view=household&months=240",
    )
    .await;
    approx(hh, 1800.0);
    // mine (owner): income 2000, expense 800 → delta 1200.
    let mine = projection_delta(&app, &owner.cookie, "/v1/projection/series?view=mine&months=240").await;
    approx(mine, 1200.0);
}

/// Reforma 3.4.0 — pin del coste aceptado: en modo real NO hay step-up al terminar un préstamo.
/// La cuota vive dentro del promedio de gasto (se carga todo el horizonte) y el principal resta
/// constante, así que la fecha de fin del plan es irrelevante para la trayectoria: dos préstamos
/// idénticos salvo el vencimiento producen EXACTAMENTE el mismo patrimonio final. Decisión de
/// producto (owner, 2026-08-18): proyección conservadora a cambio de un modelo simple sin
/// dependencia del vínculo; la realidad entra en cada recomputación vía promedio y principal.
#[tokio::test]
async fn mode_b_no_step_up_at_liability_end() {
    async fn terminal_nw(username: &str, end_year_offset: i32) -> f64 {
        let app = TestApp::spawn().await;
        let owner = app.register_and_login_owner(username).await;
        let asset_cat = app.create_category(&owner, "asset", "Bolsa").await;
        let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

        let asset = app
            .post_json_with_cookie(
                "/v1/assets",
                json!({ "category_id": asset_cat, "name": "MSCI", "current_value": "100000",
                        "is_liquid": true, "expected_annual_return_percent": "10" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
        // #150: "MSCI" es el primer (y único) activo del owner, así que crearlo ya sembró el
        // sumidero apuntándole — no hace falta crear la regla a mano.

        let today = server_today(&app, &owner.cookie).await;
        let end = date_in(today.year() + end_year_offset, today.month(), 28);
        let liab = app
            .post_json_with_cookie(
                "/v1/liabilities",
                json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat, "label": "L", "principal": "300000",
                        "payment_amount": "1000", "payment_frequency": "monthly", "payment_end_date": end }),
                &owner.cookie,
            )
            .await;
        assert_eq!(liab.status, http::StatusCode::CREATED, "{liab:?}");

        let (y1, m1) = shift_month(today.year(), today.month(), -1);
        manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "2000", "income", None, None).await;

        set_mode_b(&app, &owner.cookie).await;
        let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
        let points = body["points"].as_array().unwrap();
        points.last().unwrap()["net_worth"].as_f64().unwrap()
    }

    // EARLY: préstamo termina ~1 año. LATE: termina ~26 años (fuera del horizonte de 240 meses).
    let early = terminal_nw("alice", 1).await;
    let late = terminal_nw("bob", 26).await;

    // INVERTIDO en 4.8.0 (#142, opción 3): hasta 4.7.0 este test pineaba «sin step-up» — el
    // plan se anulaba y la fecha de vencimiento era irrelevante, o sea, la cuota liberada
    // NUNCA volvía al ahorro. Ahora el motor cobra la cuota mientras el plan vive y la libera
    // al vencer: terminar 25 años antes deja cientos de miles más de patrimonio (la cuota de
    // 1.000 €/mes componiendo al 10 %). El paréntesis firmado del owner, hecho contrato.
    assert!(
        early > late + 300_000.0,
        "el plan que termina antes libera su cuota al ahorro; early={early}, late={late}"
    );
}

/// Fix 3 (regresión): GET /v1/assets debe seguir el modo `savings_source`. El reparto del primer mes
/// usa el ahorro mensual efectivo, así que `contribution_nominal_monthly` cambia entre modo A
/// (presupuesto) y modo B (promedio real). Antes el endpoint pasaba `fire_settings = None` al builder
/// → las aportaciones por activo salían SIEMPRE en modo presupuesto.
#[tokio::test]
async fn assets_contribution_follows_savings_source_mode() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Bolsa").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;

    // Presupuesto (modo A): surplus 5000 − 3000 = 2000.
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    // Un único activo con regla remainder → recibe TODO el ahorro mensual del primer mes.
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": asset_cat, "name": "MSCI", "current_value": "100000",
                    "is_liquid": true, "expected_annual_return_percent": "5" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    // #150: "MSCI" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.

    // Transacciones en el último mes completo (modo B): income 4000, expense 1000 → surplus 3000.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "4000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;

    // Modo A (default): contribución = surplus presupuesto = 2000.
    let contrib_a = asset_contribution(&app, &owner.cookie).await;
    approx(contrib_a, 2000.0);

    // Modo B: contribución = surplus promedio real = 3000. DEBE diferir de A (regresión del bug).
    set_mode_b(&app, &owner.cookie).await;
    let contrib_b = asset_contribution(&app, &owner.cookie).await;
    approx(contrib_b, 3000.0);

    assert!(
        (contrib_a - contrib_b).abs() > 100.0,
        "GET /v1/assets debe seguir el modo: A={contrib_a}, B={contrib_b}"
    );
}

/// Fix B4 (regresión): los caps `months_expense` / `income_multiple` de las reglas de asignación se
/// resuelven con los escalares **efectivos** del engine, no con los del presupuesto.
///
/// Dataset con presupuesto y transacciones deliberadamente distintos (sin pasivos → debt_service 0):
/// - presupuesto: income 5.000, gasto 3.000
/// - promedio real del último mes completo: income 4.000, gasto 1.000
///
/// PREDICCIÓN (a mano):
/// - modo A: `Colchón` (cap `months_expense` 6) = 6 × 3.000 = **18.000**;
///   `Bolsa` (cap `income_multiple` 2) = 2 × 5.000 = **10.000**
/// - modo B: `Colchón` = 6 × 1.000 = **6.000**; `Bolsa` = 2 × 4.000 = **8.000**
/// (antes del fix ambos modos devolvían los valores del presupuesto).
#[tokio::test]
async fn assets_cap_targets_follow_savings_source_mode() {
    /// `nombre → contribution_target_amount` de GET /v1/assets (los activos sin cap no aparecen).
    async fn targets(app: &TestApp, cookie: &str) -> std::collections::HashMap<String, f64> {
        let resp = app.get_with_cookie("/v1/assets", cookie).await;
        assert_eq!(resp.status, http::StatusCode::OK, "assets: {resp:?}");
        resp.json()
            .as_array()
            .expect("array de activos")
            .iter()
            .filter_map(|a| {
                let name = a["name"].as_str()?.to_string();
                let t = a.get("contribution_target_amount")?;
                Some((name, parse_dec(t)))
            })
            .collect()
    }

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Bolsa").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;

    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    let mut ids = Vec::new();
    for (name, value) in [("Colchón", "1000"), ("Bolsa", "1000")] {
        let a = app
            .post_json_with_cookie(
                "/v1/assets",
                json!({ "category_id": asset_cat, "name": name, "current_value": value,
                        "is_liquid": true }),
                &owner.cookie,
            )
            .await;
        assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
        ids.push(a.json()["id"].as_str().unwrap().to_string());
    }

    // Colchón: remainder con cap en meses de gasto. Bolsa: percent con cap en múltiplos de ingreso.
    let r1 = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({ "target_asset_id": ids[0], "kind": "remainder",
                    "cap_kind": "months_expense", "cap_value": "6" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r1.status, http::StatusCode::CREATED, "{r1:?}");
    let r2 = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({ "target_asset_id": ids[1], "kind": "percent", "amount": "50",
                    "cap_kind": "income_multiple", "cap_value": "2" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r2.status, http::StatusCode::CREATED, "{r2:?}");

    // Promedio real del último mes completo: income 4.000, gasto 1.000.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "4000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;

    let modo_a = targets(&app, &owner.cookie).await;
    approx(modo_a["Colchón"], 18_000.0);
    approx(modo_a["Bolsa"], 10_000.0);

    set_mode_b(&app, &owner.cookie).await;
    let modo_b = targets(&app, &owner.cookie).await;
    approx(modo_b["Colchón"], 6_000.0);
    approx(modo_b["Bolsa"], 8_000.0);

    assert!(
        (modo_a["Colchón"] - modo_b["Colchón"]).abs() > 100.0
            && (modo_a["Bolsa"] - modo_b["Bolsa"]).abs() > 100.0,
        "los caps deben cambiar con el modo: A={modo_a:?}, B={modo_b:?}"
    );
}

/// `GET /v1/projection/series` reporta la fuente **efectiva** (tras el fallback) y la PROCEDENCIA
/// de cada lado (3.9.0: `savings_income_basis` / `savings_expense_basis`), para que la web
/// etiquete la pendiente sin un fetch extra y sepa decir el rango real que usó.
///
/// PREDICCIÓN: modo A → `"budget"` y ambos lados `basis: "budget"`; modo B **sin** meses reales →
/// `"budget"` y ambos en `budget` (fallback); modo B con un mes real → `"transactions_avg"` y
/// ambos lados `basis: "average"` con `months_with_data == 1`.
#[tokio::test]
async fn projection_series_reports_effective_savings_source() {
    async fn source(app: &TestApp, cookie: &str) -> (String, u64) {
        let resp = app
            .get_with_cookie("/v1/projection/series?months=240", cookie)
            .await;
        assert_eq!(resp.status, http::StatusCode::OK, "projection: {resp:?}");
        let body = resp.json();
        // El lado GASTO gobierna el contrato del modo (es el que decide `expense_from_avg`).
        (
            body["savings_source"].as_str().expect("savings_source").to_string(),
            body["savings_expense_basis"]["avg_months"]
                .as_u64()
                .expect("expense months"),
        )
    }

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "3000").await;

    assert_eq!(source(&app, &owner.cookie).await, ("budget".into(), 0));

    // Modo B sin transacciones en la ventana (solo una en el mes en curso) → fallback a budget.
    let today = server_today(&app, &owner.cookie).await;
    manual(&app, &owner.cookie, &today.format("%Y-%m-%d").to_string(), "Hoy", "9999", "income", None, None).await;
    set_mode_b(&app, &owner.cookie).await;
    assert_eq!(
        source(&app, &owner.cookie).await,
        ("budget".into(), 0),
        "fallback ⇒ la fuente efectiva reportada es budget, no la configurada"
    );

    // Un mes real en la ventana → la fuente efectiva pasa a ser la configurada.
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "4000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;
    assert_eq!(source(&app, &owner.cookie).await, ("transactions_avg".into(), 1));
}

// ---------------------------------------------------------------------------
// Ponderación sin meses pseudovacíos (solo-recurrentes) — Bloque 1
// ---------------------------------------------------------------------------

/// Un mes «pseudovacío» (solo instancias recurrentes, `recurring_rule_id NOT NULL`) queda excluido
/// POR COMPLETO del promedio: ni denominador ni numerador. Aquí: 1 mes real (income manual 2000 en
/// M-2) + 1 mes solo-recurrente (nómina recurrente 3000 en M-1) → months_with_data = 1, income_avg =
/// 2000. (Antes del fix: months = 2, income_avg = (2000+3000)/2 = 2500.)
#[tokio::test]
async fn pseudo_empty_month_excluded_from_avg() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    // Presupuesto bien distinto (modo A = 8000 − 1000 = 7000) para ver que el modo B lo ignora.
    budget(&app, &owner.cookie, &income_cat, "8000").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y2, m2) = shift_month(today.year(), today.month(), -2); // mes real
    let (y1, m1) = shift_month(today.year(), today.month(), -1); // mes solo-recurrente

    // Mes real M-2: income manual 2000 (recurring_rule_id NULL).
    manual(&app, &owner.cookie, &date_in(y2, m2, 10), "Sueldo", "2000", "income", None, None).await;
    // Mes solo-recurrente M-1: nómina recurrente 3000 (el origen es la propia instancia de M-1; el
    // backfill no crea nada más — el mes en curso jamás se materializa).
    recurring(&app, &owner.cookie, &date_in(y1, m1, 1), "Nomina rec", "3000", "income").await;

    // Modo A (default) = presupuesto.
    let delta_a = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_a, 7000.0);

    // Modo B: solo M-2 es real → months_with_data = 1, income_avg = 2000, expense_avg = 0 → delta 2000.
    set_mode_b(&app, &owner.cookie).await;
    let delta_b = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_b, 2000.0);
}

/// Un mes real cuenta ENTERO, incluidas sus transacciones recurrentes: M-2 tiene income manual 2000 +
/// nómina recurrente 3000 → income del mes = 5000; months_with_data = 1, income_avg = 5000.
#[tokio::test]
async fn real_month_counts_recurring_too() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "9000").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y2, m2) = shift_month(today.year(), today.month(), -2);

    // M-2 real: income manual 2000 + nómina recurrente 3000 (origen día 1 de M-2). El backfill crea
    // una instancia en M-1 (solo-recurrente, excluido); el mes en curso jamás se materializa.
    manual(&app, &owner.cookie, &date_in(y2, m2, 10), "Sueldo", "2000", "income", None, None).await;
    recurring(&app, &owner.cookie, &date_in(y2, m2, 1), "Nomina rec", "3000", "income").await;

    // Modo B: months_with_data = 1 (M-2), income_avg = (2000+3000)/1 = 5000 → delta 5000. Si la
    // recurrente NO contase, income_avg sería 2000.
    set_mode_b(&app, &owner.cookie).await;
    let delta_b = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_b, 5000.0);
}

/// Caso motivador del backfill: si TODA la ventana son meses solo-recurrentes (0 meses reales) →
/// months_with_data = 0 → fallback silencioso al presupuesto (modo A). Respuesta idéntica a A.
#[tokio::test]
async fn mode_b_all_pseudo_empty_falls_back_to_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "1000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y3, m3) = shift_month(today.year(), today.month(), -3);

    // Nómina recurrente con origen 3 meses atrás → backfillea M-2 y M-1 (todos solo-recurrentes;
    // el mes en curso jamás). NINGÚN movimiento real en la ventana → months_with_data = 0.
    recurring(&app, &owner.cookie, &date_in(y3, m3, 1), "Nomina rec", "3000", "income").await;

    // Modo A (default) = presupuesto 5000 − 1000 = 4000.
    let delta_a = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_a, 4000.0);

    // Modo B: 0 meses reales → fallback → idéntico a A (4000). Sin el fix serían 3 meses solo-
    // recurrentes → income_avg = 3000 → delta 3000.
    set_mode_b(&app, &owner.cookie).await;
    let delta_b = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_b, 4000.0);
    approx(delta_b, delta_a);
}

// ---------------------------------------------------------------------------
// Modo C — income del presupuesto + gasto real 12m — Bloque 2
// ---------------------------------------------------------------------------

/// Modo C: la pendiente usa el income del PRESUPUESTO y el gasto REAL medio (mismo promedio crudo
/// que el modo B). Budget income 5000, expense 2000; real M-1: income 3000, expense 800 → delta = 5000 − 800
/// = 4200. (Modo A daría 3000; modo B daría 3000 − 800 = 2200.)
#[tokio::test]
async fn mode_c_income_budget_expense_real() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "2000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-800", "expense", None, None).await;

    // Modo A: 5000 − 2000 = 3000.
    let delta_a = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_a, 3000.0);

    // Modo C: income presupuesto 5000, expense real 800 → delta 4200.
    set_mode_c(&app, &owner.cookie).await;
    let delta_c = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta_c, 4200.0);
}

/// Target FIRE modo C (annual_expense) usa el gasto real medio como base, igual que el modo B:
/// expense_avg = 1000 → target = (1000×12)/0.04 = 300000.
#[tokio::test]
async fn mode_c_target_annual_expense_uses_expense_avg() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    // Presupuesto presente (para que exista income de presupuesto) pero el target usa el gasto real.
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "9000").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;

    // FIRE: annual_expense, SWR 4%, sin impuestos, modo C → target = (expense_avg 1000 ×12)/0.04.
    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": {
                "taxes_enabled": false,
                "tax_brackets": [],
                "savings_source": "budget_income_real_expense"
            } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    // 5.0.0 (D13): el modo del objetivo y el SWR son del PERFIL del usuario; la fuente del
    // ahorro y la fiscalidad siguen siendo del hogar. Mismos números, dos superficies.
    let patched = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "annual_expense", "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");

    let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
    let target = parse_dec(&body["jubilacion_target_net_worth"]);
    approx(target, 300_000.0);
}

/// Target FIRE modo C (current_income) usa el income del PRESUPUESTO, no el de las transacciones:
/// budget income 5000 (aunque las txns midan 3000) → target = (5000×12)/0.04 = 1_500_000. Con el
/// income de transacciones (modo B) saldría 900_000.
#[tokio::test]
async fn mode_c_target_current_income_uses_budget_income() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;

    // Mes real (income 3000) → modo C activo; su income NO se usa para el target.
    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None, None).await;

    let patched = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": {
                "taxes_enabled": false,
                "tax_brackets": [],
                "savings_source": "budget_income_real_expense"
            } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    // 5.0.0 (D13): el modo del objetivo y el SWR son del PERFIL del usuario; la fuente del
    // ahorro y la fiscalidad siguen siendo del hogar. Mismos números, dos superficies.
    let patched = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "current_income", "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");

    let body = app.get_with_cookie("/v1/projection/series?months=240", &owner.cookie).await.json();
    let target = parse_dec(&body["jubilacion_target_net_worth"]);
    approx(target, 1_500_000.0);
}

/// Modo C sin datos (`months_with_data == 0`) → fallback silencioso al presupuesto, como el modo B.
#[tokio::test]
async fn mode_c_zero_months_falls_back_to_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    budget(&app, &owner.cookie, &income_cat, "5000").await;
    budget(&app, &owner.cookie, &expense_cat, "2000").await;

    // Única transacción en el mes en curso (parcial) → fuera de ventana → months_with_data = 0.
    let today = server_today(&app, &owner.cookie).await;
    manual(&app, &owner.cookie, &today.format("%Y-%m-%d").to_string(), "Hoy", "9999", "income", None, None).await;

    set_mode_c(&app, &owner.cookie).await;
    let delta = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(delta, 3000.0); // presupuesto 5000 − 2000, sin tocar por la txn del mes parcial
}

// ---------------------------------------------------------------------------
// Conciliación de transferencias (3.5.0) — el promedio del engine excluye conciliadas
// ---------------------------------------------------------------------------

/// Modo B con un par de traspaso a CABALLO de dos meses (salida −700 a fin del mes −2, entrada
/// +700 a inicio del mes −1, Δ ≤ 5 días → auto-conciliado). El caso cruza numerador Y denominador:
///
/// - Conciliadas: el mes −2 queda solo-conciliado → NO cuenta (months_with_data = 1) y las patas
///   no suman → delta = income_avg − expense_avg = 3000 − 1000 = **2000**.
/// - Tras DESCONCILIAR: ambos meses cuentan (months = 2) y las patas vuelven →
///   delta = (3000+700)/2 − (1000+700)/2 = 1850 − 850 = **1000**.
///
/// Números predichos ANTES de ejecutar (norma del repo).
#[tokio::test]
async fn mode_b_avg_excludes_reconciled_pair() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let today = server_today(&app, &owner.cookie).await;
    let (y1, m1) = shift_month(today.year(), today.month(), -1);
    let (y2, m2) = shift_month(today.year(), today.month(), -2);

    // Mes −1: datos reales.
    manual(&app, &owner.cookie, &date_in(y1, m1, 10), "Sueldo", "3000", "income", None, None).await;
    manual(&app, &owner.cookie, &date_in(y1, m1, 11), "Gasto", "-1000", "expense", None, None).await;
    // Par de traspaso a caballo de los dos meses (últimos días de −2, primeros de −1): Δ ≤ 5 días.
    let last_day_m2 = date_in(y2, m2, 28);
    let first_day_m1 = date_in(y1, m1, 1);
    manual(&app, &owner.cookie, &last_day_m2, "Traspaso salida", "-700", "expense", None, None).await;
    manual(&app, &owner.cookie, &first_day_m1, "Traspaso entrada", "700", "income", None, None).await;

    set_mode_b(&app, &owner.cookie).await;
    // Conciliadas: el mes −2 queda solo-conciliado (no cuenta) → months=1, delta = 3000−1000.
    let d1 = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(d1, 2000.0);

    // Desconciliar el par → ambos meses cuentan: delta = (3700 − 1700) / 2 = 1000.
    let list = app
        .get_with_cookie(&format!("/v1/transactions?month={y1}-{m1:02}"), &owner.cookie)
        .await
        .json();
    let in_leg = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["concept"] == "Traspaso entrada")
        .unwrap()
        .clone();
    assert!(in_leg["transfer_counterpart_id"].is_string(), "precondición: conciliada");
    let u = app
        .delete_with_cookie(
            &format!("/v1/transactions/{}/reconcile", in_leg["id"].as_str().unwrap()),
            &owner.cookie,
        )
        .await;
    assert_eq!(u.status, http::StatusCode::OK, "unreconcile: {u:?}");
    let d2 = projection_delta(&app, &owner.cookie, "/v1/projection/series?months=240").await;
    approx(d2, 1000.0);
}

/// `contribution_nominal_monthly` vs `contribution_recurring_monthly` (3.8.0): el primero incluye
/// el tramo transitorio de los planning flows del mes en curso —y por eso baja cada día— y el
/// segundo es la misma cascada sobre el neto recurrente, estable.
///
/// PREDICCIÓN: ingreso 3000, gasto 1000 → neto recurrente 2000. Un planning flow **sin fecha** de
/// 900 € se reparte a 900/90 = 10 €/día sobre una ventana de 90 días, así que el mes en curso
/// recibe `10 × días_restantes` (incluido hoy). El sumidero se lo lleva todo, luego:
/// `nominal − recurrente == planning_component > 0`, y `recurrente == 2000` exacto.
#[tokio::test]
async fn asset_contribution_separates_recurring_from_the_planning_tranche() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &cat_inc, "3000").await;
    budget(&app, &owner.cookie, &cat_exp, "1000").await;

    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat_ast, "name": "Indexado", "current_value": "0",
                    "is_liquid": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    // #150: "Indexado" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.

    // Sin planning flows los dos campos coinciden: no hay tramo transitorio que separar.
    let a = app.get_with_cookie("/v1/assets", &owner.cookie).await.json();
    let nominal: f64 = a[0]["contribution_nominal_monthly"].as_str().unwrap().parse().unwrap();
    let recurring: f64 = a[0]["contribution_recurring_monthly"].as_str().unwrap().parse().unwrap();
    assert_eq!(recurring, 2000.0, "neto recurrente = 3000 − 1000: {a}");
    assert_eq!(nominal, recurring, "sin planning flows ambos coinciden: {a}");

    // Un planning flow SIN fecha introduce el tramo /90 en el mes en curso.
    let plan = app
        .post_json_with_cookie(
            "/v1/planning/flows",
            json!({ "category_id": cat_inc, "title": "Devolucion renta",
                    "expected_amount": "900" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(plan.status, http::StatusCode::CREATED, "{plan:?}");

    let a = app.get_with_cookie("/v1/assets", &owner.cookie).await.json();
    let nominal: f64 = a[0]["contribution_nominal_monthly"].as_str().unwrap().parse().unwrap();
    let recurring: f64 = a[0]["contribution_recurring_monthly"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        recurring, 2000.0,
        "el recurrente NO se mueve con los planning flows — ese es su valor: {a}"
    );
    assert!(
        nominal > recurring,
        "el nominal del mes 1 sí incorpora el tramo de planning: nominal={nominal} recurrente={recurring}"
    );
    // El tramo es un múltiplo exacto de 900/90 = 10 €/día (días restantes del mes, incluido hoy).
    let tranche = nominal - recurring;
    assert!(
        (tranche / 10.0).fract().abs() < 1e-9 && tranche > 0.0,
        "el tramo debe ser N × 10 €/día (900/90): {tranche}"
    );
}
