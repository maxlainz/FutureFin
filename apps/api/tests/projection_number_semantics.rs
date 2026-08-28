//! Cifras de proyección que el servidor devolvía **mal o de forma no interpretable** (issue #82,
//! fase 1 de la revisión adversarial del MCP). Los tres casos comparten una misma familia de fallo:
//! el número viajaba, era sintácticamente válido, y *significaba otra cosa*.
//!
//! 1. `jubilacion_month_index` no indexa ninguna serie devuelta — es un número de MES, y con
//!    `density=hybrid` (la que fuerza la tool MCP `get_projection`) los arrays llevan ~42 puntos
//!    para 361 meses. Se añaden `jubilacion_series_position` (posición real) y
//!    `jubilacion_target_net_worth_nominal` (el objetivo del mes del cruce, que hasta ahora era
//!    **inobtenible**: solo se publicaba la base en euros de hoy).
//! 2. `final_net_worth_real_delta` restaba dos cifras deflactadas con inflaciones distintas cuando
//!    el eje simulado ES la inflación → `null` + `real_delta_absent_reason`.
//! 3. `debt_service_monthly: "0"` con un préstamo vivo → `null` + `debt_service_absent_reason`.

mod common;

use chrono::{Datelike, NaiveDate};
use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

const PROTOCOL: &str = "2026-07-28";

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba string decimal, llegó {v:?}"))
        .parse::<f64>()
        .expect("decimal")
}

// ---------------------------------------------------------------------------
// Andamiaje MCP (mismo patrón que `mcp_simulate.rs`)
// ---------------------------------------------------------------------------

async fn mcp_post(app: &TestApp, bearer: &str, body: Value) -> Value {
    let mut builder = http::Request::builder()
        .method(http::Method::POST)
        .uri("/mcp")
        .header(http::header::HOST, "futurefin.test")
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL)
        .header(http::header::AUTHORIZATION, format!("Bearer {bearer}"));
    if let Some(method) = body["method"].as_str() {
        builder = builder.header("Mcp-Method", method);
    }
    if let Some(name) = body["params"]["name"].as_str() {
        builder = builder.header("Mcp-Name", name);
    }
    let resp = app
        .request(
            builder
                .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                .expect("build MCP request"),
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "MCP POST falló: {resp:?}");
    let text = String::from_utf8(resp.body.clone()).expect("utf8");
    let content_type = resp
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.starts_with("application/json") {
        return serde_json::from_str(&text).expect("json");
    }
    let mut last = None;
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                last = Some(v);
            }
        }
    }
    last.unwrap_or_else(|| panic!("sin frame JSON en la respuesta SSE:\n{text}"))
}

fn tool_call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL,
                "io.modelcontextprotocol/clientCapabilities": {},
            }
        }
    })
}

fn tool_json(envelope: &Value) -> Value {
    let result = &envelope["result"];
    assert_ne!(result["isError"], true, "tool devolvió error: {envelope}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).expect("json")
}

async fn create_token(app: &TestApp, owner: &LoggedInOwner) -> String {
    let created = app
        .post_json_with_cookie("/v1/api-tokens", json!({"label": "issue 82"}), &owner.cookie)
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    created.json()["token"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Semilla
// ---------------------------------------------------------------------------

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

/// Hogar que **sí** cruza su número FIRE dentro del horizonte por defecto (360 meses, sin fecha de
/// nacimiento): ingreso 3.000, gasto 1.000 → 2.000 €/mes de sobrante, un activo líquido de 50.000 al
/// 7 % y una regla sumidero que encamina TODO el sobrante a ese activo (sin ella el sobrante se
/// queda en `surplus_cash`, que no capitaliza, y el cruce se va fuera del horizonte con inflación).
///
/// PREDICCIÓN del objetivo base (modo `annual_expense`, SWR 3,5 %, tramos ES por defecto):
/// neto anual = 1.000 × 12 = 12.000. Gross-up: tramo 1 (19 %, techo 6.000) da 12.000/0,81 =
/// 14.814,81 > 6.000 → K = 1.140; tramo 2 (21 %, techo 50.000): (12.000 + 1.140 − 0,21×6.000)/0,79
/// = 11.880/0,79 = **15.037,9746835443…**. Objetivo = 15.037,97…/0,035 = **429.656,4195 €**.
async fn seed_crossing_household(app: &TestApp, owner: &LoggedInOwner) -> String {
    let cat_inc = app.create_category(owner, "income", "Nomina").await;
    let cat_exp = app.create_category(owner, "expense", "Vida").await;
    let cat_ast = app.create_category(owner, "asset", "Fondos").await;
    budget(app, &owner.cookie, &cat_inc, "3000").await;
    budget(app, &owner.cookie, &cat_exp, "1000").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({
                "category_id": cat_ast,
                "name": "Indexado",
                "current_value": "50000",
                "is_liquid": true,
                "expected_annual_return_percent": "7",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset: {r:?}");
    let asset_id = r.json()["id"].as_str().unwrap().to_string();

    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({ "target_asset_id": asset_id, "kind": "remainder", "priority": 100 }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "sumidero: {r:?}");
    asset_id
}

async fn set_inflation(app: &TestApp, owner: &LoggedInOwner, pct: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "annual_inflation_assumption_percent": pct }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "inflación: {r:?}");
}

// ---------------------------------------------------------------------------
// 1. `jubilacion_series_position` + `jubilacion_target_net_worth_nominal`
// ---------------------------------------------------------------------------

/// El fallo: `jubilacion_month_index` se documentaba como «para indexar las series» y no indexa
/// ninguna. Con `density=hybrid` los arrays tienen ~42 posiciones y el mes del cruce ronda el 160:
/// indexar con él se sale del array, y caer en `[0]` presenta el objetivo de HOY como si fuera el de
/// dentro de trece años (aquí, ~1,5×).
///
/// PREDICCIONES antes de ejecutar:
/// - `jubilacion_target_net_worth` = **429.656,4195 €** (derivación en `seed_crossing_household`).
/// - `jubilacion_target_net_worth_nominal` = base × 1,03^(k/12) **exacto**, con k el mes del cruce.
/// - `density=monthly` ⇒ `jubilacion_series_position == jubilacion_month_index` (los índices
///   coinciden); `density=hybrid` ⇒ la posición es la del ÚLTIMO punto servido con
///   `month_index <= k`, y `fire_target_series.len()` es MUY menor que k (el bug original).
#[tokio::test]
async fn jubilacion_series_position_indexes_the_arrays_and_the_nominal_target_is_exact() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;
    set_inflation(&app, &owner, "3").await;

    for density in ["monthly", "hybrid"] {
        let body: Value = app
            .get_with_cookie(
                &format!("/v1/projection/series?density={density}&months=360"),
                &owner.cookie,
            )
            .await
            .json();

        let base = dec(&body["jubilacion_target_net_worth"]);
        assert!(
            (base - 429_656.4195).abs() < 0.001,
            "{density}: objetivo base predicho 429.656,4195, llegó {base}"
        );

        let k = body["jubilacion_month_index"]
            .as_u64()
            .unwrap_or_else(|| panic!("{density}: el hogar semilla tiene que cruzar: {body}"))
            as usize;

        // --- La posición existe, indexa de verdad, y respeta la convención documentada ---------
        let pos = body["jubilacion_series_position"]
            .as_u64()
            .unwrap_or_else(|| panic!("{density}: hay cruce, debe haber posición: {body}"))
            as usize;
        let points = body["points"].as_array().expect("points");
        let fire = body["fire_target_series"].as_array().expect("serie FIRE");
        assert!(
            pos < points.len() && pos < fire.len(),
            "{density}: la posición {pos} tiene que caber en los arrays ({} puntos)",
            points.len()
        );
        let mi_pos = points[pos]["month_index"].as_u64().unwrap() as usize;
        assert!(
            mi_pos <= k,
            "{density}: la convención es el punto anterior o igual — {mi_pos} > {k}"
        );
        if pos + 1 < points.len() {
            let mi_next = points[pos + 1]["month_index"].as_u64().unwrap() as usize;
            assert!(
                mi_next > k,
                "{density}: {pos} no es el ÚLTIMO punto <= {k} (el siguiente es {mi_next})"
            );
        }

        if density == "monthly" {
            assert_eq!(pos, k, "monthly: mes y posición coinciden punto por punto");
        } else {
            // El bug, hecho test: el índice de mes NO indexa el array de la densidad híbrida.
            assert!(
                fire.len() <= k,
                "hybrid: con {} puntos y cruce en el mes {k}, indexar por mes se sale del array \
                 — si esto deja de cumplirse, el test ya no prueba lo que dice",
                fire.len()
            );
        }

        // --- El objetivo NOMINAL del mes del cruce, exacto ------------------------------------
        let nominal = dec(&body["jubilacion_target_net_worth_nominal"]);
        let esperado = base * 1.03_f64.powf(k as f64 / 12.0);
        assert!(
            (nominal - esperado).abs() / esperado < 1e-9,
            "{density}: objetivo nominal {nominal}, predicho base × 1,03^({k}/12) = {esperado}"
        );
        // Y es materialmente distinto de la base: ese es el motivo de que exista el campo.
        assert!(
            nominal > base * 1.3,
            "{density}: con inflación 3 % y {k} meses el objetivo nominal tiene que despegar de la \
             base ({nominal} vs {base})"
        );
        // Lo que un consumidor obtenía indexando la serie (o cayendo en `[0]`). El valor del punto
        // servido nunca pasa del objetivo del mes del cruce — con `monthly` coincide (el punto ES
        // el mes; la holgura es solo la escala: la serie va en f64 crudo y el escalar redondeado a
        // 4 decimales), con `hybrid` va por detrás.
        let en_pos = fire[pos].as_f64().unwrap();
        assert!(
            en_pos <= nominal * (1.0 + 1e-9),
            "{density}: el target del punto anterior ({en_pos}) no puede superar al del mes del \
             cruce ({nominal})"
        );
        if density == "monthly" {
            assert!(
                (en_pos - nominal).abs() / nominal < 1e-9,
                "monthly: la posición ES el mes, la serie y el escalar tienen que coincidir \
                 ({en_pos} vs {nominal})"
            );
        }
        println!(
            "[issue #82] density={density} cruce mes {k}, posición {pos} (mes {mi_pos}), \
             puntos {}, base {base}, nominal {nominal} (×{:.4})",
            points.len(),
            nominal / base
        );
        assert!(
            fire[0].as_f64().unwrap() < nominal * 0.8,
            "{density}: `[0]` es el objetivo de hoy, muy por debajo del del cruce"
        );
    }
}

/// Sin cruce, los dos campos nuevos viajan como `null` explícito — coherentes con los
/// `jubilacion_*` que ya lo hacían (auditoría MCP §8: desaparecer no es lo mismo que no alcanzarse).
#[tokio::test]
async fn the_new_jubilacion_fields_are_explicit_null_when_there_is_no_crossing() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    // Horizonte corto: el objetivo no se alcanza.
    let short = tool_json(
        &mcp_post(&app, &token, tool_call("get_projection", json!({"months": 12}))).await,
    );
    for field in [
        "jubilacion_month_index",
        "jubilacion_series_position",
        "jubilacion_target_net_worth_nominal",
    ] {
        assert!(
            short.get(field).is_some(),
            "`{field}` debe viajar aunque no haya cruce: {short}"
        );
        assert!(short[field].is_null(), "`{field}` debería ser null: {short}");
    }
    // Y el objetivo base SÍ existe: no hay cruce, pero hay configuración FIRE.
    assert!(
        !short["jubilacion_target_net_worth"].is_null(),
        "el objetivo base no depende del cruce: {short}"
    );
}

// ---------------------------------------------------------------------------
// 2. `final_net_worth_real_delta` con deflactores incomparables
// ---------------------------------------------------------------------------

/// El fallo: cada lado se deflacta «con la inflación efectiva de su lado». Cuando el eje simulado ES
/// la inflación, eso deja de comparar nada — con la instalación al 3 % y el escenario a 0, el delta
/// nominal salía muy negativo y el real muy positivo, misma magnitud y **signos opuestos**.
///
/// PREDICCIÓN: bajar la inflación baja el objetivo FIRE ⇒ el hogar se jubila antes ⇒ acumula menos
/// ⇒ `final_net_worth_delta < 0`; y el escenario, con deflactor 1, tendría un `*_real` enorme frente
/// al baseline deflactado 30 años al 3 % ⇒ el real saldría **positivo**. Ahora: `null` + razón.
#[tokio::test]
async fn real_delta_is_absent_when_the_two_sides_deflate_with_different_inflations() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;
    set_inflation(&app, &owner, "3").await;
    let token = create_token(&app, &owner).await;

    let sim = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"annual_inflation_percent": "0"})),
        )
        .await,
    );
    let d = &sim["deltas"];

    assert_eq!(dec(&sim["baseline"]["annual_inflation_percent"]), 3.0);
    assert_eq!(dec(&sim["scenario"]["annual_inflation_percent"]), 0.0);
    assert!(
        d["final_net_worth_real_delta"].is_null(),
        "deflactores distintos ⇒ el delta real no se puede publicar: {d}"
    );
    assert_eq!(
        d["real_delta_absent_reason"], "incomparable_deflators",
        "y hay que decir por qué: {d}"
    );
    // El delta NOMINAL sigue siendo comparable (los dos lados están en euros del mismo momento) y
    // no es cero: el caso es real, no un escenario que no mueve nada.
    assert!(
        dec(&d["final_net_worth_delta"]).abs() > 1.0,
        "el escenario tiene que mover el patrimonio final o el test no prueba nada: {d}"
    );

    // Control: con la MISMA inflación en los dos lados el delta real viaja y la razón es null.
    let mismo = tool_json(
        &mcp_post(
            &app,
            &token,
            tool_call("simulate_projection", json!({"extra_monthly_expense": "-200"})),
        )
        .await,
    );
    let d2 = &mismo["deltas"];
    assert!(
        !d2["final_net_worth_real_delta"].is_null(),
        "misma inflación en los dos lados ⇒ el delta real es comparable: {d2}"
    );
    assert!(d2["real_delta_absent_reason"].is_null(), "{d2}");
    // Y con deflactor común, el signo del real coincide con el del nominal: eso es lo que la
    // versión rota no garantizaba.
    assert_eq!(
        dec(&d2["final_net_worth_delta"]) > 0.0,
        dec(&d2["final_net_worth_real_delta"]) > 0.0,
        "con un deflactor común los dos deltas tienen el mismo signo: {d2}"
    );
}

// ---------------------------------------------------------------------------
// 3. `debt_service` = `null` + razón cuando la cuota ya vive en el gasto real
// ---------------------------------------------------------------------------

fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero = (year as i64) * 12 + (month as i64 - 1) + delta as i64;
    ((zero.div_euclid(12)) as i32, (zero.rem_euclid(12) + 1) as u32)
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

/// Movimiento manual en el mes `hoy − delta` (siempre un mes CERRADO: el mes en curso queda fuera
/// de la ventana del promedio por diseño).
async fn manual_months_ago(
    app: &TestApp,
    owner: &LoggedInOwner,
    delta: i32,
    concept: &str,
    amount: &str,
    kind: &str,
    cat: &str,
) {
    let today = server_today(app, &owner.cookie).await;
    let (y, m) = shift_month(today.year(), today.month(), -delta);
    let date = NaiveDate::from_ymd_opt(y, m, 10).unwrap().format("%Y-%m-%d").to_string();
    let r = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({ "op_date": date, "concept": concept, "amount": amount,
                    "kind": kind, "category_id": cat }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "manual {concept}: {r:?}");
}

async fn set_savings_source(app: &TestApp, owner: &LoggedInOwner, mode: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "savings_source": mode } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "savings_source {mode}: {r:?}");
}

/// El fallo: en los modos B y C el servicio de deuda sale 0 porque la cuota ya está dentro del gasto
/// real. Es **correcto**, y lo único que lo explicaba era un `$defs` anidado dentro de otra tool. Un
/// cliente leía «no pagas servicio de deuda» de un usuario con un préstamo vivo de 400 €/mes.
///
/// PREDICCIÓN: modo A ⇒ `"400.0000"` y razón `null` en las DOS superficies
/// (`simulate_projection` y `/v1/allocation-rules/resolution`); modos B y C ⇒ `null` +
/// `included_in_real_expense` en las dos.
#[tokio::test]
async fn debt_service_is_null_with_a_reason_when_the_cuota_lives_inside_the_real_expense() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let liab_cat = app.create_category(&owner, "liability", "Deuda").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;
    let created = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat,
                    "label": "Hipoteca", "principal": "100000", "payment_amount": "400",
                    "payment_frequency": "monthly", "payment_end_date": "2060-01-01" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");

    // Un mes real cerrado con ingreso y gasto: sin él, B y C caen al presupuesto (modo A efectivo)
    // y la cuota volvería a ser medible — el test pasaría sin ejercitar nada.
    let exp_cat = app.create_category(&owner, "expense", "Super").await;
    let inc_cat = app.create_category(&owner, "income", "Sueldo").await;
    manual_months_ago(&app, &owner, 1, "COMPRA SUPER", "-800", "expense", &exp_cat).await;
    manual_months_ago(&app, &owner, 1, "NOMINA", "2500", "income", &inc_cat).await;

    for mode in ["budget", "transactions_avg", "budget_income_real_expense"] {
        set_savings_source(&app, &owner, mode).await;

        let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
        let k = &sim["baseline"];
        let res: Value = app
            .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
            .await
            .json();

        if mode == "budget" {
            assert_eq!(dec(&k["debt_service_monthly"]), 400.0, "modo A (sim): {k}");
            assert!(k["debt_service_absent_reason"].is_null(), "modo A (sim): {k}");
            assert_eq!(dec(&res["debt_service"]), 400.0, "modo A (resolution): {res}");
            assert!(
                res["debt_service_absent_reason"].is_null(),
                "modo A (resolution): {res}"
            );
        } else {
            assert!(
                k["debt_service_monthly"].is_null(),
                "modo {mode} (sim): un 0 no puede significar «no aplica»: {k}"
            );
            assert_eq!(
                k["debt_service_absent_reason"], "included_in_real_expense",
                "modo {mode} (sim): {k}"
            );
            assert!(
                res["debt_service"].is_null(),
                "modo {mode} (resolution): la misma cifra, la misma ausencia: {res}"
            );
            assert_eq!(
                res["debt_service_absent_reason"], "included_in_real_expense",
                "modo {mode} (resolution): {res}"
            );
            // El pasivo SIGUE vivo: la ausencia no es «no hay deuda».
            let liabs: Value = app
                .get_with_cookie("/v1/liabilities", &owner.cookie)
                .await
                .json();
            assert!(
                !liabs.as_array().expect("liabilities").is_empty(),
                "modo {mode}: el préstamo tiene que seguir ahí, o la ausencia sería trivial"
            );
        }
    }
}

/// El gate NO es el modo, es **de dónde salió la base de GASTO**. El fallback del promedio es por
/// lado (`resolve_effective_savings_inputs`), así que un modo B con datos de ingreso y sin datos de
/// gasto publica `savings_source: "transactions_avg"` y sin embargo **sí** cobra la cuota: los
/// pasivos no se anulan, `debt_service_monthly` es un número real y decir
/// `included_in_real_expense` sería mentira.
///
/// Montaje: ventana de ingreso 12 meses de calendario, ventana de gasto 3, y el único mes real a 6
/// meses vista → entra en la ventana de ingreso y no en la de gasto.
/// PREDICCIÓN: `savings_source = "transactions_avg"`, `savings_expense_basis.basis = "budget"`,
/// `debt_service_monthly = "400.0000"` y `debt_service_absent_reason = null`.
#[tokio::test]
async fn debt_service_stays_a_number_when_only_the_income_side_averages() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;
    let token = create_token(&app, &owner).await;

    let liab_cat = app.create_category(&owner, "liability", "Deuda").await;
    let liab_exp_cat = app.create_category(&owner, "expense", "Cuotas").await;
    let created = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": liab_exp_cat,
                    "label": "Hipoteca", "principal": "100000", "payment_amount": "400",
                    "payment_frequency": "monthly", "payment_end_date": "2060-01-01" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");

    let inc_cat = app.create_category(&owner, "income", "Sueldo").await;
    manual_months_ago(&app, &owner, 6, "NOMINA", "2500", "income", &inc_cat).await;

    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": {
                "savings_source": "transactions_avg",
                "income_avg_window_months": 12,
                "income_avg_window_mode": "calendar",
                "expense_avg_window_months": 3,
                "expense_avg_window_mode": "calendar",
            }}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    let k = &sim["baseline"];
    assert_eq!(
        k["savings_source"], "transactions_avg",
        "el lado ingreso promedió, la fuente efectiva es B: {k}"
    );
    assert_eq!(k["savings_income_basis"]["basis"], "average", "{k}");
    assert_eq!(
        k["savings_expense_basis"]["basis"], "budget",
        "el lado gasto cayó al presupuesto — es lo que hace de este caso el discriminante: {k}"
    );
    assert_eq!(
        dec(&k["debt_service_monthly"]),
        400.0,
        "con la base de gasto en el presupuesto la cuota SÍ se cobra aparte: {k}"
    );
    assert!(
        k["debt_service_absent_reason"].is_null(),
        "y por tanto no hay nada que excusar: {k}"
    );

    let res: Value = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await
        .json();
    assert_eq!(dec(&res["debt_service"]), 400.0, "misma respuesta en la otra superficie: {res}");
    assert!(res["debt_service_absent_reason"].is_null(), "{res}");
}
