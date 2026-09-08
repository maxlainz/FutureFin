//! **La tarjeta «Tu plan» del Resumen** — modelo v2 («el éxito define la fecha», WP A7 de 5.0.0).
//!
//! Lo único que este bloque NO puede ser es una segunda fórmula. Sale del **mismo objeto** que
//! sirve el chart —la entrada de cache de `/v1/projection/series`—, y si no hay ninguna se
//! calcula por el camino cacheado (que además la deja caliente). El bloque del ÉXITO
//! (`success_of_plan`, `success_wilson_low`, `safe_date_month_index`, `needed_capital_today`) YA
//! NO es una segunda lectura tampoco: es NIVEL 1 del mismo solve, resuelto síncrono dentro del
//! mismo miss que resuelve la fecha — `attach_success`, que hacía una llamada aparte a
//! `projection_bands_cached`, se retiró entero con WP A7. Por eso el test que más importa aquí
//! sigue siendo el de identidad cifra a cifra con la serie (y, para el éxito, también con las
//! bandas): dos superficies que responden a la misma pregunta con dos números distintos es
//! exactamente el fallo que esta casa no publica.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn summary(app: &TestApp, cookie: &str, q: &str) -> Value {
    let r = app.get_with_cookie(&format!("/v1/summary{q}"), cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "GET summary{q}: {r:?}");
    r.json()
}

async fn series(app: &TestApp, cookie: &str) -> Value {
    let r = app
        .get_with_cookie("/v1/projection/series", cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET series: {r:?}");
    r.json()
}

async fn seed(app: &TestApp, u: &LoggedInOwner, income: &str, expense: &str) {
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
                   "is_liquid": true, "expected_annual_return_percent": "5"}),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
}

async fn patch_profile(app: &TestApp, u: &LoggedInOwner, body: Value) {
    let r = app
        .patch_json_with_cookie("/v1/auth/me/retirement-profile", body, &u.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil: {r:?}");
}

/// Borra la fecha de nacimiento del solicitante (`register_and_login_owner` la deja puesta por
/// defecto). Es la única forma reproducible de forzar `plan_absent_reason: "birth_date_missing"`
/// en un test de integración: desde el modelo v2 (C5) NINGUNA estrategia —ni `asap`— resuelve
/// plan sin ella, así que basta con quitarla para pasar de `ready` a `absent`.
async fn clear_birth_date(app: &TestApp, u: &LoggedInOwner) {
    let r = app
        .patch_json_with_cookie("/v1/auth/me", json!({"birth_date": null}), &u.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "clear birth_date: {r:?}");
}

/// `plan.success_of_plan`/`success_wilson_low` viajan como número JSON (`f64`, per
/// `SummaryPlanApi`); `series`/`bands` los publican como STRING decimal (`rust_decimal`). Compara
/// las dos representaciones de la misma cifra sin asumir cuál es cuál.
fn as_f64(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or_else(|| panic!("no es un número ni un string numérico: {v}"))
}

/// **El plan del Resumen ES el de la serie, cifra a cifra** — incluido el bloque de éxito, que ya
/// no sale de una segunda lectura. No se comprueba «que hay un número»: se comprueban los campos
/// contra `/v1/projection/series`, que es el objeto del que salen.
#[tokio::test]
async fn the_summary_plan_is_the_same_object_the_chart_shows() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 60, "swr_pct": "4"}),
    )
    .await;

    let s = series(&app, &owner.cookie).await;
    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();

    assert!(plan["absent_reason"].is_null(), "{plan}");
    assert_eq!(plan["plan_state"], "ready", "{plan}");
    assert_eq!(plan["strategy"], s["strategy"], "{plan} vs {s}");
    assert_eq!(
        plan["jubilacion_month_index"], s["jubilacion_month_index"],
        "{plan}"
    );
    // `safe_date_month_index` es el MISMO mes bajo el nombre del modelo v2 — en las dos
    // superficies, no solo en una.
    assert_eq!(
        plan["safe_date_month_index"], s["safe_date_month_index"],
        "{plan} vs {s}"
    );
    assert_eq!(
        plan["jubilacion_month_index"], plan["safe_date_month_index"],
        "los dos nombres del mismo mes tienen que coincidir dentro del propio plan: {plan}"
    );
    // El nombre cambia (el Resumen habla de «ahorro necesario»), la cifra NO.
    assert_eq!(
        plan["required_savings_monthly"], s["contribution_required_monthly"],
        "{plan} vs {s}"
    );
    // Retirado del modelo v2: la SPA todavía lo declara, así que viaja, pero siempre a `null`.
    assert_eq!(plan["underfunded"], s["contribution_underfunded"], "{plan}");
    assert_eq!(
        plan["needed_capital_today"], s["needed_capital_today"],
        "misma cifra, mismo formato decimal-string: {plan} vs {s}"
    );
    assert_eq!(
        plan["success_threshold_pct"], s["success_threshold_pct"],
        "{plan} vs {s}"
    );
    assert!(
        (as_f64(&plan["success_of_plan"]) - as_f64(&s["success_of_plan"])).abs() < 1e-9,
        "{plan} vs {s}"
    );
    assert!(
        (as_f64(&plan["success_wilson_low"]) - as_f64(&s["success_wilson_low"])).abs() < 1e-9,
        "{plan} vs {s}"
    );
    // Y con este hogar (2.400 − 1.000 = 1.400 €/mes de sobrante, jubilándose a los 60) el plan
    // llega: el infra-financiado está apagado.
    assert_eq!(plan["underfunded"], false, "{plan}");
}

/// **Dos estados en los que las cifras van a `null`, no a `0`.**
///
/// (a) Con `asap` (fecha decidida por el UMBRAL, no por una edad) el plan sigue `ready` — hay
/// fecha, hay éxito, hay capital necesario hoy — pero `required_savings_monthly`/`underfunded`
/// no responden a una pregunta que no se hizo: no hay edad contra la que resolver nada, y un cero
/// ahí diría «no tienes que ahorrar nada», la respuesta contraria.
///
/// (b) Sin fecha de nacimiento el plan entero es `absent` (C5 del modelo v2: NINGUNA estrategia
/// resuelve plan sin ella, ni `asap`) y **todos** los campos —los del plan y los del éxito— van a
/// `null` a la vez.
///
/// `plan_state: "pending"` (la tercera rama) no se ejercita aquí: solo lo produce
/// `summary_plan` ante `ApiError::Unavailable` (semáforo de simulaciones cerrado), una condición
/// del propio proceso que un test de integración no puede provocar sin apagar el servidor a
/// medias — ver el doc de `SummaryPlan::plan_state` en `apps/api/src/handlers/summary.rs`.
#[tokio::test]
async fn a_pending_or_absent_plan_leaves_the_fields_null_not_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;

    // (a) asap: ready, pero con los campos de la pregunta «¿cuánto para esa edad?» en null.
    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert!(plan["absent_reason"].is_null(), "hay plan, es el de asap: {plan}");
    assert_eq!(plan["plan_state"], "ready", "{plan}");
    assert_eq!(plan["strategy"], "asap", "{plan}");
    assert!(plan["required_savings_monthly"].is_null(), "{plan}");
    assert!(plan["underfunded"].is_null(), "{plan}");
    // Lo que sí existe siempre con un plan `ready`: cuándo se jubila, el éxito de esa fecha y el
    // capital que la sostendría hoy.
    assert!(!plan["jubilacion_month_index"].is_null(), "{plan}");
    assert!(!plan["success_of_plan"].is_null(), "{plan}");
    assert!(!plan["needed_capital_today"].is_null(), "{plan}");

    // (b) sin fecha de nacimiento: absent, y AHORA sí todo a null, incluido lo que en (a) existía.
    clear_birth_date(&app, &owner).await;
    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert_eq!(plan["plan_state"], "absent", "{plan}");
    assert_eq!(plan["absent_reason"], "birth_date_missing", "{plan}");
    for k in [
        "strategy",
        "jubilacion_month_index",
        "required_savings_monthly",
        "underfunded",
        "success_of_plan",
        "success_threshold_pct",
        "success_wilson_low",
        "safe_date_month_index",
        "needed_capital_today",
        "success_verdict",
        "success_absent_reason",
    ] {
        assert!(plan[k].is_null(), "{k} debía ir a null sin plan: {plan}");
    }
}

/// **En `household` el plan va entero a `null` con su razón**: el agregado es la suma de N
/// simulaciones independientes, una por miembro y con la estrategia de cada uno. «El ahorro
/// necesario del hogar» no es una cifra que exista — ni tampoco «la probabilidad de éxito del
/// hogar».
#[tokio::test]
async fn the_household_view_has_no_plan_and_says_why() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 60}),
    )
    .await;

    let plan = summary(&app, &owner.cookie, "?view=household").await["plan"].clone();
    assert_eq!(plan["absent_reason"], "household_aggregate", "{plan}");
    assert_eq!(plan["plan_state"], "absent", "{plan}");
    for k in [
        "strategy",
        "jubilacion_month_index",
        "required_savings_monthly",
        "underfunded",
        "success_of_plan",
        "success_threshold_pct",
        "success_wilson_low",
        "safe_date_month_index",
        "needed_capital_today",
        "success_verdict",
        "success_absent_reason",
    ] {
        assert!(plan[k].is_null(), "{k} debía ir a null en household: {plan}");
    }
    // Y `mine` sí lo tiene: la diferencia entre las dos vistas es la razón de ser del campo.
    let mine = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert!(mine["absent_reason"].is_null(), "{mine}");
    assert_eq!(mine["plan_state"], "ready", "{mine}");
    assert_eq!(mine["strategy"], "retire_at_age", "{mine}");
}

/// **Un PATCH del perfil se ve en el Resumen inmediatamente.** El plan sale de la cache de
/// proyección, así que este test es la prueba de que la invalidación llega hasta aquí: si el
/// Resumen siguiera leyendo la entrada vieja, publicaría el ahorro necesario de una estrategia
/// que el usuario acaba de abandonar.
#[tokio::test]
async fn changing_the_strategy_changes_the_summary_plan_on_the_next_read() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 60, "swr_pct": "4"}),
    )
    .await;
    let antes = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert_eq!(antes["strategy"], "retire_at_age", "{antes}");
    assert!(!antes["required_savings_monthly"].is_null(), "{antes}");

    patch_profile(&app, &owner, json!({"strategy": "asap"})).await;

    let despues = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert_eq!(despues["strategy"], "asap", "{despues}");
    assert_eq!(despues["plan_state"], "ready", "{despues}");
    assert!(
        despues["required_savings_monthly"].is_null(),
        "el solve de la estrategia vieja no puede sobrevivir al cambio: {despues}"
    );
}

/// **El Resumen deja la cache de proyección CALIENTE.** No es un detalle de implementación: es
/// lo que hace que el coste del plan no sea coste nuevo. La SPA pide el Resumen y el chart casi a
/// la vez; con esto, el segundo paga cero.
#[tokio::test]
async fn reading_the_summary_warms_the_projection_cache_for_the_chart() {
    use futurefin_api::handlers::person_view::LedgerView;
    use futurefin_api::state::{Density, ProjectionCacheKey};

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    // El alta del activo invalidó la cache que el login había calentado.
    let iid = app.installation_id().await;
    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Mine,
        owner_user_id: Some(owner.user_id),
        density: Density::Hybrid,
    };
    assert!(
        !app.cache_contains(&key).await,
        "la mutación debía dejar la cache vacía antes de empezar"
    );

    let _ = summary(&app, &owner.cookie, "?view=mine").await;
    assert!(
        app.cache_contains(&key).await,
        "el Resumen calcula por el camino cacheado: el chart que viene detrás es un HIT"
    );
}

// ---------------------------------------------------------------------------------------------
// El KPI «Éxito del plan» — modelo v2: NIVEL 1 del mismo solve, no una segunda lectura
// ---------------------------------------------------------------------------------------------

/// **El KPI del Resumen sale del PLAN, no de un segundo sorteo.**
///
/// Hasta WP A7 el bloque del éxito llegaba por `attach_success`, que hacía una llamada aparte a
/// `projection_bands_cached` — dos ejecuciones de Monte Carlo con semillas distintas habrían dado
/// dos probabilidades distintas del mismo plan. Ya no existe esa llamada: `success_of_plan` es
/// NIVEL 1, se resuelve dentro del mismo solve que decide la fecha y viaja copiado del MISMO
/// objeto que `jubilacion_month_index`. Este test comprueba la identidad de todas formas contra
/// `GET /v1/projection/bands` (con el sorteo por defecto) porque las dos superficies SIGUEN
/// describiendo el mismo plan — y deben seguir citando la misma ejecución, se lea el número de
/// donde se lea.
#[tokio::test]
async fn the_success_kpi_comes_from_the_plan_not_from_a_second_draw() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    patch_profile(&app, &owner, json!({"success_threshold_pct": 90})).await;

    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert!(plan["absent_reason"].is_null(), "{plan}");
    assert!(plan["success_absent_reason"].is_null(), "{plan}");
    // El umbral del perfil se ecoa tal cual — ya NO es un ajuste ignorado (eso era V7; el modelo
    // v2 lo devolvió al perfil como la restricción que decide la fecha, C3).
    assert_eq!(plan["success_threshold_pct"], 90, "{plan}");

    let b = app
        .get_with_cookie("/v1/projection/bands", &owner.cookie)
        .await;
    assert_eq!(b.status, http::StatusCode::OK, "{b:?}");
    let b = b.json();
    assert!(
        (as_f64(&plan["success_of_plan"]) - as_f64(&b["success_of_plan"])).abs() < 1e-9,
        "el KPI y el chart deben ser la MISMA cifra: plan={plan} bands={b}"
    );
    assert!(
        (as_f64(&plan["success_wilson_low"]) - as_f64(&b["success_wilson_low"])).abs() < 1e-9,
        "{plan} / {b}"
    );
    assert_eq!(
        plan["success_threshold_pct"], b["success_threshold_pct"],
        "{plan} / {b}"
    );
    assert_eq!(plan["success_verdict"], b["success_verdict"], "{plan} / {b}");
}

/// **Leer el Resumen NO dispara un sorteo de bandas.** Antes de WP A7, `attach_success` llamaba a
/// `projection_bands_cached` y dejaba caliente una entrada en `bands_cache`; ahora el éxito sale
/// del mismo objeto que la fecha y esa llamada no existe. Si esta prueba fallara —si volviera a
/// aparecer una entrada— sería la señal de que alguien reintrodujo la segunda lectura que WP A7
/// vino a quitar.
#[tokio::test]
async fn reading_the_summary_does_not_draw_bands() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    assert!(
        app.state.bands_cache.read().await.is_empty(),
        "la mutación del alta debía dejar las bandas vacías"
    );

    let _ = summary(&app, &owner.cookie, "?view=mine").await;
    assert!(
        app.state.bands_cache.read().await.is_empty(),
        "el Resumen no debe sortear bandas: el éxito ya viaja en el plan de NIVEL 1"
    );
}

/// **El Resumen publica el capital necesario hoy y la fecha válida**, las dos cifras nuevas del
/// modelo v2 que sustituyen al objetivo FIRE determinista como referencia de portada.
#[tokio::test]
async fn the_summary_publishes_the_needed_capital_today_and_the_safe_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 60, "swr_pct": "4"}),
    )
    .await;

    let s = series(&app, &owner.cookie).await;
    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();

    assert_eq!(plan["plan_state"], "ready", "{plan}");
    // Misma cifra que la serie, mismo formato decimal-string (money: nunca f64).
    assert_eq!(
        plan["needed_capital_today"], s["needed_capital_today"],
        "{plan} vs {s}"
    );
    let needed: f64 = plan["needed_capital_today"]
        .as_str()
        .expect("decimal-string")
        .parse()
        .expect("número");
    assert!(needed > 0.0, "{plan}");
    // Redondeado a CIENTOS hacia arriba (documentado en el campo): nunca un resto de céntimos.
    assert!(
        (needed % 100.0).abs() < 1e-6,
        "needed_capital_today debe ser múltiplo de 100: {needed} ({plan})"
    );

    // `safe_date_month_index` es el mismo mes que `jubilacion_month_index`, en las dos superficies.
    assert_eq!(
        plan["safe_date_month_index"], plan["jubilacion_month_index"],
        "{plan}"
    );
    assert_eq!(
        plan["safe_date_month_index"], s["safe_date_month_index"],
        "{plan} vs {s}"
    );
    assert!(!plan["safe_date_month_index"].is_null(), "{plan}");
}
