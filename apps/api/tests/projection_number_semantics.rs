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
/// 7 % y una regla sumidero que encamina TODO el sobrante a ese activo (sin ella, desde 4.12.1,
/// el sobrante queda VARADO fuera del balance y el cruce se va fuera del horizonte).
///
/// PREDICCIÓN del objetivo base (modo `annual_expense`, SWR 3,5 %, tramos ES por defecto):
/// neto anual = 1.000 × 12 = 12.000. Gross-up: tramo 1 (19 %, techo 6.000) da 12.000/0,81 =
/// 14.814,81 > 6.000 → K = 1.140; tramo 2 (21 %, techo 50.000): (12.000 + 1.140 − 0,21×6.000)/0,79
/// = 11.880/0,79 = **15.037,9746835443…**. Objetivo = 15.037,97…/0,035 = **429.656,4195 €**.
fn es_brackets() -> Vec<futurefin_engine::TaxBracket> {
    [
        (Some(6_000u32), 19u32),
        (Some(50_000), 21),
        (Some(200_000), 23),
        (Some(300_000), 27),
        (None, 30),
    ]
    .into_iter()
    .map(|(up, pct)| futurefin_engine::TaxBracket {
        up_to: up.map(rust_decimal::Decimal::from),
        pct: rust_decimal::Decimal::from(pct),
    })
    .collect()
}

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
    // #150: "Indexado" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.
    r.json()["id"].as_str().unwrap().to_string()
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
        // Desde la Ola 6 (#170) el objetivo se evalúa mes a mes sobre la necesidad REAL:
        // nominal = gross_up(12.000·1,03^(k/12))/0,035 — el gross-up de la necesidad inflada,
        // NO la base inflada (gross_up es afín: fiscal drag, los tramos son nominales). El
        // oráculo es el helper del motor con los MISMOS ingredientes del hogar semilla — que es
        // exactamente lo que este assert vigila: que el campo sale del motor evaluado en k, no
        // interpolado de la serie.
        let nominal = dec(&body["jubilacion_target_net_worth_nominal"]);
        let ft = futurefin_engine::FireTarget {
            need: futurefin_engine::FireNeed::ExpenseMinusPension {
                expense_monthly: rust_decimal::Decimal::from(1_000),
                pension_monthly: rust_decimal::Decimal::ZERO,
            },
            swr_pct: "3.5".parse().unwrap(),
            tax_brackets: es_brackets(),
            taxes_enabled: true,
            taxable_gain_ratio: rust_decimal::Decimal::ONE,
            annual_inflation_percent: rust_decimal::Decimal::from(3),
            debt_payments_remaining: Vec::new(),
        };
        let esperado: f64 =
            futurefin_engine::fire_target_at_month_index(Some(&ft), k as u32)
                .unwrap()
                .round_dp(4)
                .to_string()
                .parse()
                .unwrap();
        assert!(
            (nominal - esperado).abs() / esperado < 1e-9,
            "{density}: objetivo nominal {nominal}, predicho gross_up(need·f({k}))/swr = {esperado}"
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

/// 5.0.0 (§B.8) — **las tres series de retirada son FLUJOS del mes, no acumulados**, y los dos
/// nombres del mes efectivo apuntan al mismo punto de la serie.
///
/// Publicar `withdrawal` como acumulado sería el mismo fallo de familia que este fichero
/// documenta: una cifra sintácticamente válida que significa otra cosa. Se distingue con una
/// prueba directa —la suma de los flujos hasta el mes k es MAYOR que el flujo de k, y el flujo no
/// es monótono— sin depender de ningún número concreto del modelo.
#[tokio::test]
async fn the_withdrawal_series_are_monthly_flows_and_the_positions_index_the_arrays() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;

    let r = app
        .get_with_cookie("/v1/projection/series?months=600", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let s = r.json();

    let k = s["jubilacion_month_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("este escenario debe cruzar: {s}"));
    assert_eq!(
        s["retirement_month_index"], s["jubilacion_month_index"],
        "R8: los dos nombres son el mismo mes: {s}"
    );
    assert_eq!(
        s["retirement_series_position"], s["jubilacion_series_position"],
        "y las dos posiciones, la misma: {s}"
    );

    let pts = s["points"].as_array().expect("points");
    // La posición INDEXA el array (esa es toda su razón de ser) y apunta al último punto servido
    // que no pasa del mes publicado.
    let pos = s["retirement_series_position"].as_u64().expect("posición") as usize;
    assert!(pos < pts.len(), "la posición debe caer dentro de points: {pos} / {}", pts.len());
    assert!(
        pts[pos]["month_index"].as_u64().unwrap() <= k,
        "convención «el punto anterior o igual»: {}",
        pts[pos]
    );

    // El flujo del mes: 0 mientras se acumula, positivo una vez jubilado.
    let w_at = |m: u64| -> f64 {
        pts.iter()
            .find(|p| p["month_index"] == m)
            .unwrap_or_else(|| panic!("sin punto {m}"))["withdrawal"]
            .as_f64()
            .unwrap()
    };
    assert_eq!(w_at(0), 0.0, "el mes 0 no es un mes simulado: {s}");
    assert_eq!(w_at(k.saturating_sub(1)), 0.0, "el mes anterior aún no drena");
    let despues = w_at(k + 12);
    assert!(despues > 0.0, "jubilado: el mes {} retira: {despues}", k + 12);

    // Y NO es un acumulado: la suma de los flujos posteriores al cruce supera con creces
    // cualquier flujo individual, y el flujo individual no crece sin parar de mes en mes.
    let suma: f64 = pts
        .iter()
        .filter(|p| p["month_index"].as_u64().unwrap() > k)
        .map(|p| p["withdrawal"].as_f64().unwrap())
        .sum();
    assert!(
        suma > despues * 2.0,
        "si `withdrawal` fuese acumulado, un solo punto ya valdría casi la suma entera: \
         suma {suma}, punto {despues}"
    );
}

/// **`unmet_need`: la tercera magnitud del mes** (pase de correcciones de la revisión
/// adversarial, hallazgo B2/#4).
///
/// `withdrawal` es lo que se obtuvo, `withdrawal_shortfall` lo que la REGLA rechazó y
/// `unmet_need` lo que la CARTERA no pudo dar. Confundir las dos últimas es el fallo que este
/// campo existe para hacer imposible: con la regla por defecto (`fixed_real`, sin techo) el
/// recorte es **cero por construcción**, así que un hogar que se queda sin cartera publicaba
/// ceros en todas las columnas de retirada y su único rastro era un escalar al final.
///
/// El hogar: 500 € de ingreso contra 2.500 € de gasto y una hucha de 3.000 € al 0 %. El
/// descubierto empieza a los dos meses y no para.
///
/// Lo que se pinea:
/// 1. `points[0].unmet_need == 0` — el mes 0 es el estado de hoy, no un mes simulado.
/// 2. **Nunca negativo y a la escala monetaria**: el motor conserva en el acumulador el operando
///    literal de 4.15.0 y su serie llega con una polvareda de ±1e-25 € incluso en un hogar
///    solvente (medido: 7 de 66 puntos en el arnés de `mcp_simulate`). Lo que se publica va
///    clampado Y redondeado a 4 decimales — es la única columna del punto cuyo signo se lee como
///    un veredicto, y servir la polvareda encendería meses en rojo al azar.
/// 3. **Σ mensual = `uncovered_deficit_total`**: la serie es la descomposición del escalar, no
///    otra cuenta. Sin esta identidad las dos cifras podrían derivar sin que nada avisara.
/// 4. `withdrawal_shortfall` es **todo ceros** en el mismo hogar: son magnitudes distintas.
#[tokio::test]
async fn unmet_need_is_non_negative_starts_at_zero_and_adds_up_to_the_uncovered_total() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    budget(&app, &owner.cookie, &inc, "500").await;
    budget(&app, &owner.cookie, &exp, "2500").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": ast, "name": "Hucha", "current_value": "3000",
                   "is_liquid": true, "expected_annual_return_percent": "0"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let r = app
        .get_with_cookie(
            "/v1/projection/series?months=24&density=monthly",
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let s = r.json();
    let pts = s["points"].as_array().expect("points");
    assert_eq!(pts.len(), 25, "densidad mensual: 0..=24 ({s})");

    assert_eq!(
        pts[0]["unmet_need"].as_f64(),
        Some(0.0),
        "el mes 0 es el estado de hoy, no un mes simulado: {}",
        pts[0]
    );
    let mut suma = 0.0f64;
    let mut algun_positivo = false;
    for p in pts {
        let u = p["unmet_need"]
            .as_f64()
            .unwrap_or_else(|| panic!("`unmet_need` debe ser un número: {p}"));
        assert!(
            u >= 0.0,
            "la serie publicada sale clampada; un negativo aquí es la cola de 1e-24 sin clampar: {p}"
        );
        algun_positivo |= u > 0.0;
        suma += u;
        assert_eq!(
            p["withdrawal_shortfall"].as_f64(),
            Some(0.0),
            "con `fixed_real` la REGLA no recorta nunca — si esto deja de ser cero, las dos \
             magnitudes se han vuelto a mezclar: {p}"
        );
    }
    assert!(
        algun_positivo,
        "este hogar se queda sin cartera al segundo mes: el descubierto tiene que verse: {s}"
    );
    assert_ne!(
        s["assets_depleted_month_index"],
        Value::Null,
        "y la cartera se agota de verdad: {s}"
    );

    // La serie ES la descomposición del escalar. Tolerancia absoluta de un céntimo: el total lo
    // publica el motor con su propia escala y la suma se hace aquí en `f64`.
    let total = dec(&s["uncovered_deficit_total"]);
    assert!(
        (suma - total).abs() < 0.01,
        "Σ points[].unmet_need = {suma} debe ser `uncovered_deficit_total` = {total}: {s}"
    );
}

/// El espejo: un hogar que ahorra no tiene descubierto **en ningún mes**, y el escalar y la serie
/// dicen lo mismo. Un cero de verdad, no un hueco — y **exactamente** cero, que es lo que compra
/// el redondeo de publicación: sin él, aquí saldrían unos cuantos `1e-25`.
#[tokio::test]
async fn a_solvent_household_publishes_unmet_need_zero_everywhere() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_crossing_household(&app, &owner).await;

    let r = app
        .get_with_cookie(
            "/v1/projection/series?months=120&density=monthly",
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let s = r.json();
    for p in s["points"].as_array().expect("points") {
        assert_eq!(
            p["unmet_need"].as_f64(),
            Some(0.0),
            "sin descubierto la columna es cero, no falta: {p}"
        );
    }
    assert_eq!(
        s["uncovered_deficit_total"], "0.0000",
        "y el escalar dice lo mismo: {s}"
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

        // INVERTIDO en 4.8.0 (#142, opción 3): la cuota es servicio de deuda REAL en los TRES
        // modos y en las DOS superficies (sim + resolution) — en B/C el gasto efectivo ya la
        // restó del promedio, así que publicarla es contarla UNA vez. El literal
        // `included_in_real_expense` se retiró con su modo.
        assert_eq!(dec(&k["debt_service_monthly"]), 400.0, "modo {mode} (sim): {k}");
        assert!(k["debt_service_absent_reason"].is_null(), "modo {mode} (sim): {k}");
        assert_eq!(dec(&res["debt_service"]), 400.0, "modo {mode} (resolution): {res}");
        assert!(
            res["debt_service_absent_reason"].is_null(),
            "modo {mode} (resolution): {res}"
        );
        {
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

/// #124 (4.8.0), escenario del issue a mano: partida de 500 € cuyo `expense_end_date` venció
/// hace 6 meses + gasto vivo de 1.500 €, ingreso 3.000 €, SWR 4 %, modo A, sin impuestos.
/// Hasta 4.7.0 la partida vencida vivía dos vidas: sumaba en el presupuesto (gasto 2.000,
/// ahorro publicado 1.000, objetivo 600.000) mientras el motor la cancelaba mes a mes (ahorro
/// real 1.500) — «ahorras 1.000» junto a una serie que ahorra 1.500. Ahora las tres superficies
/// dicen lo mismo: gasto **1.500**, delta **1.500**, objetivo **450.000** (= 1.500×12/0,04).
/// La FILA sigue visible en `entries` (no se purga: reads never mutate).
#[tokio::test]
async fn an_expired_budget_entry_stops_counting_everywhere_at_once() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    budget(&app, &owner.cookie, &cat_inc, "3000").await;
    budget(&app, &owner.cookie, &cat_exp, "1500").await;

    // La partida que venció: se crea con fin futuro (la API valida) y se vence por SQL.
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({ "category_id": cat_exp, "amount": "500", "expense_end_date": "2090-01-01" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let entry_id = r.json()["id"].as_str().unwrap().to_string();
    sqlx::query("UPDATE budget_entries SET expense_end_date = DATE '2020-01-31' WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&entry_id).unwrap())
        .execute(&app.pool)
        .await
        .expect("vencer la partida");

    // SWR 4 % (por defecto es 3,5): objetivo redondo del escenario.
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "fire_settings": { "taxes_enabled": false, "tax_brackets": [] } }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    // 5.0.0 (D13): modo del objetivo y SWR son del perfil del usuario, no del hogar.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "annual_expense", "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    // 1) Presupuesto: la vencida no suma; la fila sigue visible.
    let b = app.get_with_cookie("/v1/budget", &owner.cookie).await.json();
    let expense: f64 = b["totals"]["expense_regular_monthly_equivalent"].as_str().unwrap().parse().unwrap();
    assert!((expense - 1_500.0).abs() < 0.001, "gasto sin la vencida: {expense}");
    let visible = b["entries"].as_array().unwrap().iter().any(|e| e["id"] == entry_id.as_str());
    assert!(visible, "la fila vencida sigue en entries (no se purga)");

    // 2) Proyección: delta y objetivo alineados — y SIN caja fantasma (la entrada compensadora
    //    del motor se retiró junto con la partida; con solo medio arreglo el delta sería 2.000).
    let p = app.get_with_cookie("/v1/projection/series?months=120", &owner.cookie).await.json();
    let delta: f64 = p["monthly_delta_assumption"].as_str().unwrap().parse().unwrap();
    assert!((delta - 1_500.0).abs() < 0.001, "delta sin la vencida y sin fantasma: {delta}");
    let target: f64 = p["jubilacion_target_net_worth"].as_str().unwrap().parse().unwrap();
    assert!((target - 450_000.0).abs() < 0.001, "objetivo 1.500×12/0,04: {target}");
}

/// #127 (4.8.0), a mano: ingreso 3.000, gasto 1.800, y un pasivo sin intereses con cuota
/// NOMINAL 900 pero solo 300 € de saldo — la caja real del mes 1 paga min(900, 300) = 300.
/// Hasta 4.7.0 `sim_kpis` recalculaba con la cuota nominal (net_cash 300) mientras el motor
/// repartía 900: dos «cajas del mes» para la misma pregunta. Ahora ambas cifras salen del MISMO
/// primer paso del motor: **900** en las dos superficies.
#[tokio::test]
async fn sim_kpis_cash_converges_to_the_engines_first_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let token = create_token(&app, &owner).await;
    let cat_inc = app.create_category(&owner, "income", "Nomina").await;
    let cat_exp = app.create_category(&owner, "expense", "Vida").await;
    let cat_liab = app.create_category(&owner, "liability", "Restos").await;
    let cat_cuota = app.create_category(&owner, "expense", "Cuotas").await;
    budget(&app, &owner.cookie, &cat_inc, "3000").await;
    budget(&app, &owner.cookie, &cat_exp, "1800").await;
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": cat_liab, "expense_category_id": cat_cuota,
                    "label": "Resto", "principal": "300",
                    "payment_amount": "900", "payment_frequency": "monthly" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    let sim = tool_json(&mcp_post(&app, &token, tool_call("simulate_projection", json!({}))).await);
    let k = &sim["baseline"];
    assert!(
        (dec(&k["net_recurring_monthly"]) - 900.0).abs() < 0.001,
        "recurring = 3.000 − 1.800 − min(900, 300): {k}"
    );
    assert!(
        (dec(&k["net_cash_monthly"]) - 900.0).abs() < 0.001,
        "la caja del mes ES la del primer paso del motor: {k}"
    );
    // Y la resolución de la cascada (que YA llamaba al motor) dice lo mismo.
    let res: Value = app
        .get_with_cookie("/v1/allocation-rules/resolution", &owner.cookie)
        .await
        .json();
    assert!(
        (dec(&res["recurring_net"]) - 900.0).abs() < 0.001,
        "las dos superficies convergen: {res}"
    );
}
