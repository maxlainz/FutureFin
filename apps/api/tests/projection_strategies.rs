//! **Las tres estrategias que WP5-2b conectó al motor**: `pension_bridge` (objetivo puente con
//! pensión con fecha), `coast` (dejar de aportar y llegar igual) y `partial` (media jornada).
//!
//! El motor ya sabía hacer las tres desde WP3 y tiene sus propios tests de aritmética. Lo que se
//! pinea AQUÍ es el **mapeo y la publicación**: que el perfil de un usuario se convierta en el
//! `PhasePlan` correcto (edades → meses, base del objetivo, tasa de descuento del puente) y que
//! lo que sale por el wire sea la cifra que el motor calculó, en su rejilla y con su unidad.
//!
//! Cada número está PREDICHO a mano en el comentario del test antes de correrlo. Con inflación 0
//! y sin impuestos, las tres fórmulas de §B.3 son aritmética de servilleta:
//! perpetuidad `12·need/SWR`, hueco de media jornada `12·gap/SWR`, puente
//! `Σ 2000·(1+d)^(−m/12) + perp·(1+d)^(−P/12)`.

mod common;

use chrono::{Datelike, NaiveDate};
use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn series(app: &TestApp, cookie: &str, q: &str) -> Value {
    let r = app
        .get_with_cookie(&format!("/v1/projection/series{q}"), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET series{q}: {r:?}");
    r.json()
}

async fn patch_profile(app: &TestApp, u: &LoggedInOwner, body: Value) {
    let r = app
        .patch_json_with_cookie("/v1/auth/me/retirement-profile", body, &u.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil: {r:?}");
}

/// Los impuestos van FUERA en todos los tests de este fichero: el ejemplo del issue («2.000 €/mes,
/// 4 %, sin impuestos ⇒ 600.000 / 270.000») está escrito así, y con el gross-up de tramos
/// españoles encima ninguna de las cifras sería comprobable a mano — que es justo lo que estos
/// tests compran.
async fn taxes_off(app: &TestApp, u: &LoggedInOwner) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxes_enabled": false}}),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "taxes off: {r:?}");
}

/// Presupuesto + un activo líquido. `expected_annual_return_percent` es además la tasa de
/// descuento del puente por defecto (`bridge_discount_basis: expected_return` con una cartera
/// líquida de un solo activo).
async fn seed(app: &TestApp, u: &LoggedInOwner, income: &str, expense: &str, asset_return: &str) {
    let inc = app.create_category(u, "income", "Nómina").await;
    let exp = app.create_category(u, "expense", "Vida").await;
    let ast = app.create_category(u, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, income), (&exp, expense)] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({"category_id": cat, "amount": amount, "ends_at_retirement": false}),
                &u.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": ast, "name": "Indexado", "current_value": "20000",
                   "is_liquid": true, "expected_annual_return_percent": asset_return}),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
}

/// Meses de la rejilla hasta el mes en que el usuario cumple `age`, con la MISMA aritmética civil
/// que publica la respuesta. Se recalcula en el test —y no se importa del handler— para que lo
/// que se compruebe sea la DEFINICIÓN y no la implementación.
fn months_until_age(anchor: NaiveDate, birth: NaiveDate, age: u32) -> u32 {
    let completed = |at: NaiveDate| -> i32 {
        let mut y = at.year() - birth.year();
        if (at.month(), at.day()) < (birth.month(), birth.day()) {
            y -= 1;
        }
        y
    };
    let add = |m: u32| -> NaiveDate {
        anchor
            .checked_add_months(chrono::Months::new(m))
            .expect("dentro de rango")
    };
    (0..=1200u32)
        .find(|&m| completed(add(m)) >= age as i32)
        .expect("la edad se alcanza dentro de 100 años")
}

fn anchor_of(s: &Value) -> NaiveDate {
    NaiveDate::parse_from_str(s["anchor_date_ymd"].as_str().expect("ancla"), "%Y-%m-%d")
        .expect("ancla parseable")
}

const OWNER_BIRTH: (i32, u32, u32) = (1990, 1, 1);
fn owner_birth() -> NaiveDate {
    NaiveDate::from_ymd_opt(OWNER_BIRTH.0, OWNER_BIRTH.1, OWNER_BIRTH.2).unwrap()
}

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un string decimal, llegó {v}"))
        .parse()
        .expect("decimal parseable")
}

// ---------------------------------------------------------------------------------------------
// partial — el ejemplo del issue #207
// ---------------------------------------------------------------------------------------------

/// **El caso literal del issue**: gasto 2.000 €/mes, SWR 4 %, sin impuestos, media jornada de
/// 1.100 €/mes ⇒ el hueco que la media jornada deja abierto cuesta **270.000 €** a perpetuidad.
///
/// Predicho a mano antes de correrlo, con inflación 0 (el arnés la normaliza) y `expense_basis`
/// por defecto = gasto de JUBILACIÓN, que aquí son los mismos 2.000 (ninguna partida termina al
/// jubilarse):
///
/// ```text
/// gap_m  = max(0, 2000·f(X−1) − 1100 − 0)   = 900 €/mes
/// target = gross_up(12·900) / (4/100)       = 10.800 / 0,04 = 270.000 €
/// ```
///
/// Y la perpetuidad del gasto ENTERO, la otra cifra del issue, sale del mismo sitio:
/// `12·2000/0,04 = 600.000 €` — es `fire_target_series[0]`, porque no hay deuda.
#[tokio::test]
async fn partial_publishes_the_270k_gap_of_the_issue_example() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "partial", "swr_pct": "4",
               "partial_retirement": {"starts_at_age": 50, "income_monthly_today": "1100"}}),
    )
    .await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    let anchor = anchor_of(&s);
    let x = months_until_age(anchor, owner_birth(), 50);

    assert_eq!(s["strategy"], "partial", "{s}");
    assert_eq!(
        s["partial_gap_target"], "270000.0000",
        "12·(2000−1100)/0,04 = 270.000: {s}"
    );
    assert_eq!(
        s["partial_retirement_month_index"], x,
        "la media jornada empieza el mes en que cumple 50: {s}"
    );
    // La otra cifra del issue: la perpetuidad del gasto entero, en el mes 0 de la serie.
    let target_hoy = s["fire_target_series"].as_array().expect("serie")[0]
        .as_f64()
        .expect("f64");
    assert!(
        (target_hoy - 600_000.0).abs() < 1.0,
        "12·2000/0,04 = 600.000, llegó {target_hoy}: {s}"
    );
    // Sin edad de fin declarada, `partial` sigue disparándose por CRUCE.
    assert_eq!(s["retirement_trigger"], "liquid_crossing", "{s}");
    // Hubo fase parcial ⇒ la lectura es un bool de verdad, no el `null` de «no hubo».
    assert!(
        s["partial_phase_capital_growing"].is_boolean(),
        "hubo media jornada: la lectura existe ({s})"
    );
}

/// **La media jornada declarada que NUNCA se vive no publica su hueco** (pase de correcciones de
/// la revisión adversarial).
///
/// `partial_gap_target` se calculaba de la fase DECLARADA en el perfil, no de la fase que el
/// hogar atravesó: un plan que cruza su número FIRE a los 40 y se jubila del todo publicaba
/// «capital del hueco de la media jornada: 270.000 €» para una media jornada de los 60 que el
/// cruce había dejado décadas atrás. Ahora va atado a `partial_retirement_month_index`, igual que
/// `partial_phase_capital_growing`: los tres describen la MISMA fase y no pueden usar criterios
/// distintos para existir.
///
/// El hogar: 900.000 € líquidos contra un objetivo de `12·1.500/0,04 = 450.000 €`. Cruza el
/// primer mes, así que la media jornada de los 60 no llega a empezar nunca.
#[tokio::test]
async fn a_partial_phase_that_never_happens_publishes_no_gap_target() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "6000"), (&exp, "1500")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({"category_id": cat, "amount": amount, "ends_at_retirement": false}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": ast, "name": "Indexado", "current_value": "900000",
                   "is_liquid": true, "expected_annual_return_percent": "5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "partial", "swr_pct": "4",
               "partial_retirement": {"starts_at_age": 60, "income_monthly_today": "1100"}}),
    )
    .await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(s["strategy"], "partial", "la fase SÍ está declarada: {s}");
    let k = s["jubilacion_month_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("con 900.000 contra 450.000 el cruce es inmediato: {s}"));
    let x = months_until_age(anchor_of(&s), owner_birth(), 60);
    assert!(
        k < u64::from(x),
        "el cruce ({k}) tiene que ir MUY por delante de los 60 (mes {x}) para que la fase no \
         llegue a vivirse: {s}"
    );
    assert!(
        s["partial_retirement_month_index"].is_null(),
        "la fase declarada no se vivió: {s}"
    );
    assert!(
        s["partial_gap_target"].is_null(),
        "un hueco de una fase que nunca ocurrió es una cifra inventada: {s}"
    );
    assert!(
        s["partial_phase_capital_growing"].is_null(),
        "y su lectura hermana también: los tres describen la misma fase: {s}"
    );
}

/// **Sin fase parcial, `partial_phase_capital_growing` es `null`, no `false`.** El motor publica
/// un `bool` porque es una función pura y debe definir el estado; el wire no puede permitirse que
/// «no hay media jornada» y «la hay y se come el capital» compartan valor.
#[tokio::test]
async fn without_a_partial_phase_the_capital_reading_is_null_not_false() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", "5").await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(s["strategy"], "asap", "{s}");
    assert!(s["partial_phase_capital_growing"].is_null(), "{s}");
    assert!(s["partial_gap_target"].is_null(), "{s}");
    assert!(s["partial_retirement_month_index"].is_null(), "{s}");
}

// ---------------------------------------------------------------------------------------------
// pension_bridge
// ---------------------------------------------------------------------------------------------

/// **El puente vale MENOS que la perpetuidad que ignora la pensión** — y esa es toda la razón de
/// ser de `pension_bridge` (P2): no hace falta capital para vivir del patrimonio *para siempre*,
/// solo hasta que llega la pensión, más lo que la pensión no cubra.
///
/// Predicho a mano, mismo hogar y misma pensión en los dos lados, inflación 0 y sin impuestos.
/// Ancla 2026-09-03, nacimiento 1990-01-01 ⇒ los 67 se cumplen en 2057-01, o sea `P = 364` meses.
/// La tasa de descuento es la rentabilidad esperada ponderada de la cartera LÍQUIDA, que aquí es
/// un solo activo al 5 %:
///
/// ```text
/// r      = 1,05^(−1/12) = 0,99594241
/// puente = 2000·(1 − r^364)/(1 − r) = 2000 · 190,35 ≈ 380.700 €
/// perp   = 12·(2000 − 1200)/0,04 = 240.000, descontada ×r^364 = 0,22763 ≈ 54.630 €
/// T(0)   ≈ 435.300 €        frente a la perpetuidad íntegra 12·2000/0,04 = 600.000 €
/// ```
#[tokio::test]
async fn the_bridge_target_is_lower_than_the_perpetuity_that_ignores_the_pension() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;

    // Lado A — la pensión declarada pero el objetivo dimensionado a perpetuidad (R6: la opción
    // explícita «no cuento con ella», la conservadora).
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "asap", "swr_pct": "4", "target_basis": "perpetuity",
               "pension": {"monthly_amount_today": "1200", "starts_at_age": 67}}),
    )
    .await;
    let perp = series(&app, &owner.cookie, "?months=600").await;
    let perp_hoy = perp["fire_target_series"].as_array().expect("serie")[0]
        .as_f64()
        .expect("f64");
    assert!(
        (perp_hoy - 600_000.0).abs() < 1.0,
        "la perpetuidad ignora la pensión hasta P: 12·2000/0,04 = 600.000, llegó {perp_hoy}"
    );
    assert!(
        perp["bridge_discount_annual_pct"].is_null(),
        "sin base puente no hay tasa que publicar: {perp}"
    );

    // Lado B — la misma pensión, con el objetivo PUENTE (que `pension_bridge` fuerza).
    patch_profile(&app, &owner, json!({"strategy": "pension_bridge"})).await;
    let bridge = series(&app, &owner.cookie, "?months=600").await;
    let anchor = anchor_of(&bridge);
    let p = months_until_age(anchor, owner_birth(), 67);

    let bridge_hoy = bridge["fire_target_series"].as_array().expect("serie")[0]
        .as_f64()
        .expect("f64");
    assert!(
        bridge_hoy < perp_hoy,
        "el puente ({bridge_hoy}) tiene que ser MENOR que la perpetuidad ({perp_hoy})"
    );
    assert!(
        (430_000.0..441_000.0).contains(&bridge_hoy),
        "predicho ≈435.300 €, llegó {bridge_hoy}: {bridge}"
    );
    // La tasa que de verdad se usó, publicada: la rentabilidad ponderada del líquido (5 %).
    assert_eq!(bridge["bridge_discount_annual_pct"], "5.0000", "{bridge}");
    assert_eq!(
        bridge["pension_start_month_index"], p,
        "la pensión empieza el mes en que cumple 67: {bridge}"
    );
    // FRACCIÓN, no porcentaje (la regla de oro de los sufijos): 1.200 / 2.000 = 0,6.
    assert_eq!(bridge["pension_coverage_ratio"], "0.6000", "{bridge}");
    // Vivir del patrimonio hasta la pensión exige sacar el gasto ENTERO, y eso es una tasa por
    // encima del SWR — legítimamente, porque dura pocos años. Es lo que este KPI existe para
    // hacer visible.
    let eff = dec(&bridge["bridge_effective_withdrawal_pct"]);
    assert!(
        eff > 4.0,
        "el puente retira por encima del SWR del 4 %, llegó {eff}: {bridge}"
    );
}

/// **La tasa de descuento del puente sin un euro líquido del que sacarla**: cae a 0 (puente sin
/// descontar, la lectura conservadora) y lo DICE. Un objetivo puente sin descuento es
/// sensiblemente mayor, y sin el aviso esa diferencia no se explicaría por ningún campo.
#[tokio::test]
async fn a_bridge_without_liquid_assets_warns_instead_of_discounting_at_zero_in_silence() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Ladrillo").await;
    for (cat, amount) in [(&inc, "3000"), (&exp, "2000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({"category_id": cat, "amount": amount, "ends_at_retirement": false}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    // Un activo NO líquido: existe, crece, y no sirve para descontar un puente que se paga
    // vendiendo cartera.
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": ast, "name": "Vivienda", "current_value": "200000",
                   "is_liquid": false, "expected_annual_return_percent": "3"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    patch_profile(
        &app,
        &owner,
        json!({"strategy": "pension_bridge", "swr_pct": "4",
               "bridge_discount_basis": "expected_return",
               "pension": {"monthly_amount_today": "1200", "starts_at_age": 67}}),
    )
    .await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(s["bridge_discount_annual_pct"], "0.0000", "{s}");
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    assert!(
        warnings.contains(&"bridge_discount_no_liquid_assets"),
        "el puente cayó a 0 % de descuento y hay que decirlo: {warnings:?} en {s}"
    );
}

/// **`bridge_discount_basis: swr` usa el SWR del perfil**, y `none` no descuenta nada. Los tres
/// ejes de D7 se comprueban por su efecto sobre el objetivo, no por el nombre: sin descuento el
/// puente es la suma llana de sus flujos y por tanto el objetivo MÁS caro de los tres.
#[tokio::test]
async fn the_three_bridge_discount_bases_produce_three_ordered_targets() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;

    let target_now = |s: &Value| -> f64 {
        s["fire_target_series"].as_array().expect("serie")[0]
            .as_f64()
            .expect("f64")
    };

    patch_profile(
        &app,
        &owner,
        json!({"strategy": "pension_bridge", "swr_pct": "4",
               "bridge_discount_basis": "none",
               "pension": {"monthly_amount_today": "1200", "starts_at_age": 67}}),
    )
    .await;
    let sin = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(sin["bridge_discount_annual_pct"], "0.0000", "{sin}");
    // Sin descuento: 364 meses × 2.000 + 240.000 = 968.000 €.
    let sin_hoy = target_now(&sin);
    assert!(
        (960_000.0..975_000.0).contains(&sin_hoy),
        "predicho 968.000 €, llegó {sin_hoy}: {sin}"
    );

    patch_profile(&app, &owner, json!({"bridge_discount_basis": "swr"})).await;
    let swr = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(swr["bridge_discount_annual_pct"], "4.0000", "{swr}");
    let swr_hoy = target_now(&swr);

    patch_profile(&app, &owner, json!({"bridge_discount_basis": "expected_return"})).await;
    let ret = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(ret["bridge_discount_annual_pct"], "5.0000", "{ret}");
    let ret_hoy = target_now(&ret);

    // A mayor tasa de descuento, menor objetivo: 0 % > 4 % > 5 %.
    assert!(
        sin_hoy > swr_hoy && swr_hoy > ret_hoy,
        "0 % ({sin_hoy}) > 4 % ({swr_hoy}) > 5 % ({ret_hoy})"
    );
}

// ---------------------------------------------------------------------------------------------
// coast
// ---------------------------------------------------------------------------------------------

/// **`coast`**: el mes a partir del cual se puede dejar de aportar y llegar igual, con su número
/// y su serie discontinua.
///
/// Predicho a mano. Ingreso 3.000, gasto 1.000 ⇒ sobrante 2.000 €/mes; activo 20.000 al 5 %;
/// SWR 4 % sin impuestos ⇒ objetivo `12·1000/0,04 = 300.000 €`. Los 60 se cumplen en 2050-01,
/// que desde el ancla 2026-09-03 son **280 meses** de rejilla (`R` del bucle = 281, y el criterio
/// se evalúa en el índice 280).
///
/// Con `m = 1,05^(1/12) − 1 = 0,00407412` y aportando `n` meses antes de parar:
///
/// ```text
/// L(280) = (1+m)^280 · [20.000 + (2000/m)·(1 − (1+m)^(−n))]
///        = 3,1220 · [20.000 + 490.903·(1 − (1+m)^(−n))]  ≥ 300.000
///  ⇒ (1 − (1+m)^(−n)) ≥ 0,15501  ⇒  n ≈ 41,4  ⇒  mes coast ≈ 42 de la rejilla
/// ```
///
/// La cifra exacta la decide la cascada del motor (que además cobra el gasto del mes), así que el
/// test la acota en una banda alrededor de la predicción en vez de clavarla: lo que se pinea es
/// que el mes existe, que cae MUY por debajo de la jubilación y que la serie coast **de verdad
/// alcanza el objetivo** en el mes de la jubilación — que es la definición.
#[tokio::test]
async fn coast_publishes_the_month_the_number_and_a_path_that_reaches_the_target() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "1000", "5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "coast", "target_retirement_age": 60, "swr_pct": "4"}),
    )
    .await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    let anchor = anchor_of(&s);
    let r_grid = months_until_age(anchor, owner_birth(), 60);

    assert_eq!(s["strategy"], "coast", "{s}");
    assert_eq!(s["retirement_trigger"], "target_age", "{s}");
    assert_eq!(s["jubilacion_month_index"], r_grid, "{s}");

    let coast = s["coast_fire_month_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("coast alcanzable en este hogar: {s}")) as u32;
    assert!(
        coast < r_grid,
        "el mes coast ({coast}) cae ANTES de la jubilación ({r_grid}): {s}"
    );
    assert!(
        (20..90).contains(&coast),
        "predicho ≈42 meses, llegó {coast}: {s}"
    );
    // El número coast: el líquido con el que se ENTRA en ese mes. Positivo y por debajo del
    // objetivo (si ya lo superara, el coast sería hoy).
    let numero = dec(&s["coast_number"]);
    assert!(numero > 20_000.0, "el número coast crece sobre los 20.000 de partida: {s}");

    // **La definición, comprobada sobre la serie**: parando en el mes coast, el líquido del mes
    // de la jubilación alcanza el objetivo de ese mes. `?months` sin `density` sirve la densidad
    // mensual, así que la posición del array ES el mes.
    let path = s["coast_path"].as_array().expect("coast_path");
    let target = s["fire_target_series"].as_array().expect("fire_target_series");
    assert_eq!(path.len(), target.len(), "series paralelas: {}", path.len());
    let l = path[r_grid as usize].as_f64().expect("f64");
    let t = target[r_grid as usize].as_f64().expect("f64");
    assert!(
        l >= t,
        "parando en el mes coast se llega igual: líquido {l} vs objetivo {t} en el mes {r_grid}"
    );
    // Y no llega de sobra por accidente: parar un año antes NO llegaría. Se comprueba con el
    // margen, que es 0 antes del mes coast por definición.
    let margen = s["disposable_capital"].as_array().expect("disposable_capital");
    assert_eq!(
        margen[(coast - 1) as usize].as_f64().expect("f64"),
        0.0,
        "antes del mes coast no hay margen: cada euro que dejes de aportar retrasa la fecha"
    );
    // Y hoy tampoco hay margen mensual: el mes coast todavía no ha llegado.
    assert_eq!(s["disposable_monthly"], "0.0000", "{s}");
}

/// **`coast` que no llega ni aportando siempre**: `coast_fire_month_index` es `null` y viaja
/// `coast_not_reachable`. No es un error — la simulación existe y se publica entera.
#[tokio::test]
async fn a_coast_that_never_reaches_the_target_says_so() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    // Gasto 2.500 sobre un ingreso de 2.600: objetivo 750.000 € y 100 €/mes de sobrante.
    seed(&app, &owner, "2600", "2500", "5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "coast", "target_retirement_age": 55, "swr_pct": "4"}),
    )
    .await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    assert!(s["coast_fire_month_index"].is_null(), "{s}");
    assert!(s["coast_number"].is_null(), "{s}");
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    assert!(
        warnings.contains(&"coast_not_reachable"),
        "{warnings:?} en {s}"
    );
    // La serie sigue viajando: es la mejor que el plan da (aportando todos los meses).
    assert!(
        !s["coast_path"].as_array().expect("coast_path").is_empty(),
        "{s}"
    );
}


// ---------------------------------------------------------------------------------------------
// El aterrizaje exacto (pase de correcciones de la revisión adversarial)
// ---------------------------------------------------------------------------------------------

/// **La cartera que se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto
/// NO está agotada.**
///
/// Es el hallazgo #2 de la segunda revisión adversarial. Hasta el pase de correcciones,
/// `assets_depleted_month_index` lo decidía un solo predicado —«venta bruta ≥ drenable»— evaluado
/// ANTES de vender, así que un plan perfecto (24 meses de puente pagados al céntimo con una
/// pensión detrás que cubre el 125 % del gasto) se publicaba como «cartera agotada en el mes
/// 311» con `uncovered_deficit_total = 0`: dos cifras de la misma respuesta contándose la una a
/// la otra que mentían. Hoy hacen falta DOS condiciones —la venta dejó la cartera a cero **Y**
/// alguna venta posterior se quedó sin fundar—, y aquí la segunda no se cumple.
///
/// **El puente se mide, no se supone.** Una primera pasada con una cartera holgada dice cuánto
/// cuesta el puente entero (`Σ points[].withdrawal`: después de la pensión no se retira nada, así
/// que la suma del horizonte ES el coste del puente); la segunda pone en el activo exactamente
/// esa cifra. Sin este rodeo el test dependería de que 24 × 2.000 sea la cuenta correcta, que es
/// justo lo que no puede darse por hecho en un test de aterrizajes exactos.
#[tokio::test]
async fn an_exact_landing_on_a_fully_covering_pension_is_not_a_depleted_portfolio() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    // El ingreso TERMINA al jubilarse y el gasto no: durante la acumulación la caja es 0 exacta
    // (2.000 − 2.000), así que el activo no crece ni mengua y el puente empieza con el saldo que
    // este test le ponga.
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": inc, "amount": "2000", "ends_at_retirement": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": exp, "amount": "2000", "ends_at_retirement": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    // 0 % de rentabilidad: el saldo solo cambia por lo que se vende, y la aritmética del
    // aterrizaje es exacta en `Decimal`.
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": ast, "name": "Puente", "current_value": "500000",
                   "is_liquid": true, "expected_annual_return_percent": "0"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let asset_id = r.json()["id"].as_str().expect("asset id").to_string();

    // Jubilación por EDAD a los 60 (mes conocido, sin depender del cruce) y pensión CON FECHA a
    // los 62 que cubre el 125 % del gasto: 24 meses de puente y ni un euro de venta después.
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 60, "swr_pct": "4",
               "target_basis": "bridge_to_pension",
               "pension": {"monthly_amount_today": "2500", "starts_at_age": 62}}),
    )
    .await;

    // --- Pasada 1: cartera holgada. Se MIDE el coste del puente. -------------------------------
    let holgado = series(&app, &owner.cookie, "?months=600&density=monthly").await;
    assert_eq!(holgado["retirement_trigger"], "target_age", "{holgado}");
    let r_grid = holgado["jubilacion_month_index"].as_u64().expect("jubilación");
    let p_grid = holgado["pension_start_month_index"]
        .as_u64()
        .expect("pensión con fecha");
    assert_eq!(
        p_grid - r_grid,
        24,
        "60 → 62 son 24 meses de puente: {holgado}"
    );
    let puntos = holgado["points"].as_array().expect("points");
    let coste_puente: f64 = puntos.iter().map(|p| p["withdrawal"].as_f64().unwrap()).sum();
    assert!(
        (coste_puente - 48_000.0).abs() < 1.0,
        "24 meses × 2.000 € = 48.000 €, medidos {coste_puente}: {holgado}"
    );
    // Después de la pensión no se vende nada: la suma de arriba es el puente ENTERO y no una
    // parte de un drenaje que sigue.
    for p in puntos.iter().filter(|p| p["month_index"].as_u64().unwrap() > p_grid) {
        assert_eq!(
            p["withdrawal"].as_f64(),
            Some(0.0),
            "la pensión cubre el gasto: no hay venta después: {p}"
        );
    }

    // --- Pasada 2: el aterrizaje EXACTO. -------------------------------------------------------
    let exacto = format!("{coste_puente:.4}");
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{asset_id}"),
            json!({"current_value": exacto}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = series(&app, &owner.cookie, "?months=600&density=monthly").await;
    assert_eq!(
        s["assets_depleted_month_index"],
        Value::Null,
        "la cartera se vacía al céntimo el último mes del puente, y NADIE se queda sin cobrar: \
         eso no es una ruina, es un plan perfecto ({s})"
    );
    assert_eq!(
        s["uncovered_deficit_total"], "0.0000",
        "y el escalar hermano lo confirma — las dos cifras ya no pueden contradecirse: {s}"
    );
    let pts = s["points"].as_array().expect("points");
    for p in pts {
        assert_eq!(
            p["unmet_need"].as_f64(),
            Some(0.0),
            "ninguna venta se quedó sin fundar: {p}"
        );
    }
    // El aterrizaje es REAL: el líquido del cierre del último mes del puente es cero al céntimo.
    let liquido = |m: u64| -> f64 {
        pts.iter()
            .find(|p| p["month_index"].as_u64() == Some(m))
            .unwrap_or_else(|| panic!("sin punto {m}"))["net_worth_liquid"]
            .as_f64()
            .unwrap()
    };
    assert!(
        liquido(p_grid).abs() < 0.01,
        "cierre del último mes del puente: {} (debería ser 0)",
        liquido(p_grid)
    );
    assert!(
        liquido(p_grid - 1) > 0.0,
        "y el mes anterior todavía tenía saldo: {}",
        liquido(p_grid - 1)
    );

    // --- Control negativo: un mes de puente MENOS y el veredicto cambia. ------------------------
    // Sin esto, el test pasaría igual con un handler que publicara `null` siempre.
    let corto = format!("{:.4}", coste_puente - 2_000.0);
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{asset_id}"),
            json!({"current_value": corto}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let corta = series(&app, &owner.cookie, "?months=600&density=monthly").await;
    assert_ne!(
        corta["assets_depleted_month_index"],
        Value::Null,
        "faltando 2.000 € el último mes del puente SÍ se queda sin fundar: {corta}"
    );
    assert!(
        dec(&corta["uncovered_deficit_total"]) > 0.0,
        "y el descubierto ya no es cero: {corta}"
    );
}

// ---------------------------------------------------------------------------------------------
// El descuento del puente no puede ser negativo
// ---------------------------------------------------------------------------------------------

/// **Una cartera líquida con pérdidas esperadas NO produce un descuento negativo**: se sube a 0 y
/// se dice con `bridge_discount_clamped`.
///
/// Descontar es responder «cuánto capital necesito HOY para pagar un flujo futuro». Con `d < 0` el
/// factor `(1+d/100)^{j/12}` es menor que 1 y cada euro futuro cuesta MÁS de un euro hoy: el
/// objetivo puente saldría por encima de la suma llana de sus flujos, que es lo contrario de lo
/// que descontar significa. Y suficientemente negativo (−53,8 % a 840 meses de puente, −41,8 % a
/// 1200) la tabla desborda `Decimal` y el motor devuelve `BridgeDiscountOverflow` — que la API
/// mapea a 422 `bridge_discount_out_of_range`, hoy inalcanzable por este camino precisamente por
/// este clamp.
///
/// Se comprueba por su EFECTO y no solo por el nombre: el objetivo con la tasa clampada tiene que
/// ser exactamente el mismo que con `bridge_discount_basis: none`.
#[tokio::test]
async fn a_negative_expected_return_clamps_the_bridge_discount_to_zero_and_says_so() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    // La cartera líquida espera PERDER un 5 % anual: la base `expected_return` daría −5 %.
    seed(&app, &owner, "3000", "2000", "-5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "pension_bridge", "swr_pct": "4",
               "bridge_discount_basis": "expected_return",
               "pension": {"monthly_amount_today": "1200", "starts_at_age": 67}}),
    )
    .await;

    let s = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(
        s["bridge_discount_annual_pct"], "0.0000",
        "el descuento se sube a 0, no se pasa negativo al motor: {s}"
    );
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    assert!(
        warnings.contains(&"bridge_discount_clamped"),
        "un objetivo que deja de descontarse cambia de tamaño y hay que decirlo: {warnings:?} en {s}"
    );
    assert!(
        !warnings.contains(&"bridge_discount_no_liquid_assets"),
        "hay activo líquido de sobra: el motivo es otro y no puede confundirse: {warnings:?}"
    );
    let clampado = s["fire_target_series"].as_array().expect("serie")[0]
        .as_f64()
        .expect("f64");

    // El mismo hogar declarando explícitamente «no descuentes»: el objetivo debe coincidir.
    patch_profile(&app, &owner, json!({"bridge_discount_basis": "none"})).await;
    let sin = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(sin["bridge_discount_annual_pct"], "0.0000", "{sin}");
    let sin_hoy = sin["fire_target_series"].as_array().expect("serie")[0]
        .as_f64()
        .expect("f64");
    assert!(
        (clampado - sin_hoy).abs() < 1.0,
        "clampar a 0 tiene que ser exactamente «no descontar»: {clampado} vs {sin_hoy}"
    );
    // Y el aviso desaparece cuando la base ya no es la rentabilidad esperada: el clamp solo
    // existe donde puede pasar.
    let sin_w: Vec<&str> = sin["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    assert!(
        !sin_w.contains(&"bridge_discount_clamped"),
        "con base `none` no hay nada que clampar: {sin_w:?}"
    );
}
