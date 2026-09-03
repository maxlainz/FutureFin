//! **La tarjeta «Tu plan» del Resumen** (5.0.0 WP5-2b, D27): `/v1/summary` publica el bloque
//! `plan` con la estrategia, el disparador, el mes efectivo, el ahorro necesario, el margen y el
//! rojo de D17.
//!
//! Lo único que este bloque NO puede ser es una segunda fórmula. Sale del **mismo objeto** que
//! sirve el chart —la entrada de cache de `/v1/projection/series`—, y si no hay ninguna se
//! calcula por el camino cacheado (que además la deja caliente). Por eso el test que más importa
//! aquí es el de identidad cifra a cifra con la serie: dos superficies que responden a la misma
//! pregunta con dos números distintos es exactamente el fallo que esta casa no publica.

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

/// **El plan del Resumen ES el de la serie, cifra a cifra.** No se comprueba «que hay un número»:
/// se comprueban los seis campos contra `/v1/projection/series`, que es el objeto del que salen.
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
    assert_eq!(plan["strategy"], s["strategy"], "{plan} vs {s}");
    assert_eq!(plan["retirement_trigger"], s["retirement_trigger"], "{plan}");
    assert_eq!(
        plan["jubilacion_month_index"], s["jubilacion_month_index"],
        "{plan}"
    );
    // El nombre cambia (el Resumen habla de «ahorro necesario»), la cifra NO.
    assert_eq!(
        plan["required_savings_monthly"], s["required_contribution_monthly"],
        "{plan} vs {s}"
    );
    assert_eq!(plan["disposable_monthly"], s["disposable_monthly"], "{plan}");
    assert_eq!(plan["underfunded"], s["underfunded"], "{plan}");
    // Y con este hogar (2.400 − 1.000 = 1.400 €/mes de sobrante, objetivo 300.000 €) el plan
    // llega: el rojo de D17 está apagado y hay margen.
    assert_eq!(plan["underfunded"], false, "{plan}");
    assert!(
        plan["disposable_monthly"].as_str().expect("decimal").parse::<f64>().unwrap() > 0.0,
        "{plan}"
    );
}

/// **`asap` no responde a «cuánto tengo que ahorrar»**, y eso es `null`, no `0`. Un cero ahí
/// diría «no tienes que ahorrar nada», que es la respuesta contraria.
#[tokio::test]
async fn a_crossing_strategy_leaves_the_solve_fields_null_not_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;

    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert!(plan["absent_reason"].is_null(), "hay plan, es el de asap: {plan}");
    assert_eq!(plan["strategy"], "asap", "{plan}");
    assert_eq!(plan["retirement_trigger"], "liquid_crossing", "{plan}");
    assert!(plan["required_savings_monthly"].is_null(), "{plan}");
    assert!(plan["disposable_monthly"].is_null(), "{plan}");
    assert!(plan["underfunded"].is_null(), "{plan}");
    // Lo que sí existe siempre: cuándo se jubila.
    assert!(!plan["jubilacion_month_index"].is_null(), "{plan}");
}

/// **En `household` el plan va entero a `null` con su razón**: el agregado es la suma de N
/// simulaciones independientes, una por miembro y con la estrategia de cada uno. «El ahorro
/// necesario del hogar» no es una cifra que exista.
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
    for k in [
        "strategy",
        "retirement_trigger",
        "jubilacion_month_index",
        "required_savings_monthly",
        "disposable_monthly",
        "underfunded",
        // 5.0.0 WP6b: el KPI «Éxito del plan» cae con el resto. El hogar es la suma de N planes
        // independientes y «la probabilidad de éxito del hogar» no es una cifra que exista.
        "success_probability",
        "success_threshold_pct",
        "success_verdict",
    ] {
        assert!(plan[k].is_null(), "{k} debía ir a null en household: {plan}");
    }
    // Y `mine` sí lo tiene: la diferencia entre las dos vistas es la razón de ser del campo.
    let mine = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert!(mine["absent_reason"].is_null(), "{mine}");
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
    assert_eq!(despues["retirement_trigger"], "liquid_crossing", "{despues}");
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
// El KPI «Éxito del plan» (5.0.0 WP6b, D28)
// ---------------------------------------------------------------------------------------------

/// **El KPI del Resumen y el fan chart de Jubilación citan la MISMA ejecución.**
///
/// No es una preferencia de estilo: dos ejecuciones de Monte Carlo con semillas distintas dan dos
/// probabilidades distintas del mismo plan, y el usuario vería el tile del Resumen discrepar del
/// gráfico sin ninguna explicación posible. Se comprueba por identidad contra
/// `GET /v1/projection/bands` con los caminos y la semilla por defecto — exactamente la petición
/// que hace la sección «Riesgo».
#[tokio::test]
async fn the_success_kpi_is_the_same_run_the_risk_chart_draws() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;

    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert!(plan["absent_reason"].is_null(), "{plan}");
    assert!(plan["success_absent_reason"].is_null(), "{plan}");

    let b = app
        .get_with_cookie("/v1/projection/bands", &owner.cookie)
        .await;
    assert_eq!(b.status, http::StatusCode::OK, "{b:?}");
    let b = b.json();
    assert_eq!(
        plan["success_probability"], b["success_probability"],
        "el KPI y el chart deben ser la MISMA cifra: plan={plan} bands={b}"
    );
    assert_eq!(plan["success_verdict"], b["success_verdict"], "{plan} / {b}");
    assert_eq!(
        plan["success_threshold_pct"], b["success_threshold_pct"],
        "{plan} / {b}"
    );
    // Y el umbral es el del PERFIL, no una constante: al cambiarlo, las dos superficies lo siguen.
    patch_profile(&app, &owner, json!({"success_threshold_pct": 80})).await;
    let plan = summary(&app, &owner.cookie, "?view=mine").await["plan"].clone();
    assert_eq!(plan["success_threshold_pct"], 80, "{plan}");
}

/// Leer el Resumen deja **calientes las bandas**: el GET de `/v1/projection/bands` que la SPA
/// hace un instante después es un HIT y no vuelve a sortear. Es el mismo trato que el bloque
/// `plan` ya tenía con la serie — el coste se paga una vez, no dos.
#[tokio::test]
async fn reading_the_summary_warms_the_bands_cache_for_the_risk_section() {
    use futurefin_api::state::BandsCacheKey;

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2400", "1000").await;
    let iid = app.installation_id().await;
    assert!(
        app.state.bands_cache.read().await.is_empty(),
        "la mutación del alta debía dejar las bandas vacías"
    );

    let _ = summary(&app, &owner.cookie, "?view=mine").await;
    let cache = app.state.bands_cache.read().await;
    assert_eq!(cache.len(), 1, "el Resumen debe dejar UNA entrada de bandas");
    let k: &BandsCacheKey = cache.keys().next().expect("una clave");
    assert_eq!(k.installation_id, iid, "{k:?}");
    assert_eq!(k.user_id, owner.user_id, "{k:?}");
    assert_eq!(k.paths, 500, "los caminos por defecto: {k:?}");
}
