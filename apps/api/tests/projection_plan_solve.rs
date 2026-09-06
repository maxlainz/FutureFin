//! **El plan de jubilación resuelto por el sorteo**: los dos niveles de
//! `handlers/retirement_solver.rs` vistos desde fuera, por HTTP (5.0.0, modelo v2, WP A3).
//!
//! # Estos tests están escritos contra el CONTRATO, y todavía no pueden correr
//!
//! A3 construye el solver y su cache; quien los **conecta** a la respuesta es A4 (ensamblado) y
//! quien **publica** el bloque «plan» de `ProjectionSeriesResponse` es A5. Hasta que esos dos
//! aterricen, `GET /v1/projection/series` no tiene ni `retirement_date_basis` ni
//! `needed_capital_today` ni `needed_capital_curve_state`, así que todo esto fallaría por campos
//! ausentes — no por el solver.
//!
//! Por eso van con `#[ignore]` y con la misma frase en todos: **A5 tiene que quitar el `ignore`**.
//! No es un adorno: un test ignorado que nadie desmarca es peor que un test que no existe, porque
//! aparenta cobertura. Si al cerrar A5 alguno sigue ignorado, o falta el campo o falta el test.
//!
//! Compilan HOY —van por HTTP y leen `serde_json::Value`, así que ningún campo nuevo es un símbolo
//! de Rust— y eso es lo que los hace verificables ya: `cargo check --tests` los cubre.
//!
//! # Qué se mide aquí y qué no
//!
//! Aquí se miden **propiedades del contrato**: quién espera a quién, qué mueve la clave de la
//! cache, qué se publica cuando no hay respuesta. La aritmética de Wilson, la bisección y la
//! minimalidad viven en `crates/engine-stochastic/tests/` y no se re-prueban por HTTP: un test de
//! integración que reimplanta el criterio del crate es una segunda definición del criterio.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::state::Density;
use serde_json::Value;
use uuid::Uuid;

/// La razón, escrita una sola vez, para que quitarla sea un `grep` y no una búsqueda.
const PENDING_A5: &str = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore";

/// Un hogar con cartera y con margen: sin líquido no hay ni fecha ni capital que medir, y todos
/// estos tests se quedarían midiendo la ausencia.
async fn household_with_a_plan(app: &TestApp, name: &str) -> LoggedInOwner {
    let owner = app.register_and_login_owner(name).await;
    let asset_cat = app.create_category(&owner, "asset", "Fondo").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": "Indexado",
                "current_value": "400000",
                "expected_annual_return_percent": "5",
                "annual_volatility_percent": "15",
                "is_liquid": true,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "crear activo: {r:?}");

    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Vida").await;
    for (cat, amount) in [(income_cat, "3000"), (expense_cat, "1500")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget",
                serde_json::json!({"category_id": cat, "amount": amount}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "crear presupuesto: {r:?}");
    }
    owner
}

/// Fija el umbral del perfil y devuelve la fecha que el plan publica con él, en meses de la
/// REJILLA. `None` = no hay ninguna alcanzable con ese umbral, que es una respuesta.
async fn date_with_threshold(app: &TestApp, owner: &LoggedInOwner, threshold: u32) -> Option<u64> {
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            serde_json::json!({"success_threshold_pct": threshold}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "patch umbral {threshold}: {r:?}");
    let s = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(s.status, http::StatusCode::OK);
    s.json()["jubilacion_month_index"].as_u64()
}

/// Espera a que el nivel 2 aterrice, con una cota acotada por EVENTO y no por reloj: sale en
/// cuanto el estado deja de ser `computing`. El tope solo se agota si de verdad no llegó.
async fn settle_plan_extras(app: &TestApp, cookie: &str) -> Value {
    for _ in 0..600 {
        let r = app.get_with_cookie("/v1/projection/series", cookie).await;
        assert_eq!(r.status, http::StatusCode::OK);
        let body = r.json();
        if body["needed_capital_curve_state"] != "computing" {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("el nivel 2 no aterrizó dentro del margen");
}

// =================================================================================================
// Los dos niveles
// =================================================================================================

/// **La serie espera al nivel 1 y NO espera al nivel 2.**
///
/// Es la decisión de producto entera de este WP, y por eso es el primer test: la fecha, el éxito y
/// el capital necesario hoy cuestan segundos y viajan en la primera respuesta; la curva por edad
/// cuesta decenas y llega después, declarándose `computing` mientras tanto. Si algún día el nivel 2
/// se colara en el camino síncrono, este test lo vería como un GET que tarda medio minuto — pero lo
/// que afirma no es el reloj, es el CONTRATO: los campos del nivel 1 están y los del nivel 2
/// todavía no.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn the_series_waits_for_level_one_and_the_extras_arrive_later() {
    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let first = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(first.status, http::StatusCode::OK);
    let body = first.json();

    // Nivel 1: presente en la PRIMERA respuesta.
    assert!(
        body["retirement_date_basis"].is_string(),
        "la fecha es de nivel 1 y viaja en el primer GET: {body}"
    );
    assert!(
        !body["success_of_plan"].is_null(),
        "el éxito del plan es de nivel 1"
    );
    assert!(
        !body["success_sampling_error_pp"].is_null(),
        "una probabilidad sin su barra de error no se publica"
    );
    assert!(
        !body["paths_used"].is_null() && !body["seed"].is_null(),
        "sin N y sin semilla el número no es reproducible y por tanto no es un resultado"
    );

    // Nivel 2: declarado como «se está calculando», no como ausente.
    assert_eq!(
        body["needed_capital_curve_state"], "computing",
        "el nivel 2 se declara `computing`, nunca `unavailable`: «no se puede» y «todavía no» son \
         dos respuestas distintas"
    );

    // …y llega.
    let settled = settle_plan_extras(&app, &owner.cookie).await;
    assert_eq!(settled["needed_capital_curve_state"], "ready");
    assert!(
        settled["needed_capital_curve"].as_array().is_some_and(|a| !a.is_empty()),
        "la curva llega con puntos: {settled}"
    );
}

/// **Dos peticiones concurrentes resuelven la fecha UNA vez.**
///
/// El nivel 2 se deduplica por `plan_inflight`. La prueba directa no es cronometrar: es contar
/// entradas de la cache de plan — dos peticiones del mismo hogar producen la misma clave de
/// contenido, así que si la deduplicación funcionara mal habría dos tareas escribiendo la misma
/// entrada, y lo que se observa es que la entrada es una y su contenido, uno.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn two_concurrent_requests_solve_the_date_once() {
    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let (a, b) = tokio::join!(
        app.get_with_cookie("/v1/projection/series", &owner.cookie),
        app.get_with_cookie("/v1/projection/series", &owner.cookie),
    );
    assert_eq!(a.status, http::StatusCode::OK);
    assert_eq!(b.status, http::StatusCode::OK);
    // La MISMA pregunta tiene la MISMA respuesta: si cada petición hubiera sorteado por su cuenta
    // con otra muestra, estas dos cifras diferirían.
    assert_eq!(
        a.json()["success_of_plan"],
        b.json()["success_of_plan"],
        "dos peticiones del mismo plan no pueden publicar dos probabilidades"
    );
    assert_eq!(a.json()["jubilacion_month_index"], b.json()["jubilacion_month_index"]);

    settle_plan_extras(&app, &owner.cookie).await;
    assert_eq!(
        app.state.plan_cache.read().await.len(),
        1,
        "una pregunta, una entrada: la deduplicación por `plan_inflight` es lo que lo garantiza"
    );
}

// =================================================================================================
// La cache direccionada por contenido
// =================================================================================================

/// **La clave del plan es el CONTENIDO, y una mutación la mueve.**
///
/// Es la propiedad que justifica que `plan_cache` no entre en `invalidate_projection_by_*`: la
/// entrada vieja no se borra porque **no hace falta** — nadie puede volver a pedirla. Lo que se
/// comprueba es justo eso: tras mutar, la entrada del plan que sirve el GET es OTRA, y la vieja
/// sigue ahí sin que nadie pueda alcanzarla.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn the_plan_cache_is_content_addressed_and_a_mutation_moves_the_key() {
    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    settle_plan_extras(&app, &owner.cookie).await;
    let key_before = app
        .state
        .projection_cache_plan_key(&app.default_view_key(iid, owner.user_id))
        .await;
    assert!(key_before.is_some(), "la entrada cacheada lleva su clave de plan");
    let entries_before = app.state.plan_cache.read().await.len();

    // Una mutación cualquiera del ledger: otro hogar, otra entrada del motor, otra huella.
    let asset_cat = app.create_category(&owner, "asset", "Cuenta").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": "Corriente",
                "current_value": "12000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "crear activo: {r:?}");

    settle_plan_extras(&app, &owner.cookie).await;
    let key_after = app
        .state
        .projection_cache_plan_key(&app.default_view_key(iid, owner.user_id))
        .await;
    assert_ne!(
        key_before, key_after,
        "un hogar distinto es otra huella: si la clave no se moviera, el plan viejo se serviría \
         como si fuera el nuevo"
    );
    assert!(
        app.state.plan_cache.read().await.len() > entries_before,
        "la entrada vieja NO se borra: se queda inalcanzable, que es la razón por la que este \
         mapa no entra en las invalidaciones"
    );
}

// =================================================================================================
// El umbral
// =================================================================================================

/// **Subir el umbral nunca adelanta la fecha.**
///
/// El criterio de éxito (`SuccessAt::meets`) es monótono en el umbral: un mes que cumple el 100 %
/// cumple cualquier umbral menor, así que el conjunto de meses válidos solo puede encogerse al
/// subirlo. Lo que la bisección devuelve **no es el mínimo** de ese conjunto —el crate lo dice sin
/// adornos— así que la monotonía del resultado no es un teorema; lo que sí lo es es la del
/// criterio, y las dos búsquedas recorren la misma rejilla con los mismos caminos (números
/// aleatorios comunes).
///
/// La única vía por la que una fecha puede saltar hacia adelante sin que el criterio cambie son
/// los avances de la fase de confirmación (hasta doce meses), y por eso la tolerancia es
/// exactamente esa y no un margen a ojo. **Una violación mayor no es un flake: es un hallazgo**, y
/// se abre como issue.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn a_higher_threshold_never_moves_the_date_earlier() {
    use futurefin_engine_stochastic::MAX_CONFIRMATION_ADVANCES;

    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let lax = date_with_threshold(&app, &owner, 85).await;
    let strict = date_with_threshold(&app, &owner, 99).await;
    match (lax, strict) {
        (Some(lax), Some(strict)) => assert!(
            strict + u64::from(MAX_CONFIRMATION_ADVANCES) >= lax,
            "el 99 % ({strict}) no puede jubilar ANTES que el 85 % ({lax}) más allá de los \
             avances de confirmación"
        ),
        // Que el umbral estricto no tenga fecha es una respuesta legítima («no llegas al 99 %»),
        // y lo contrario —el laxo sin fecha y el estricto con ella— sería el hallazgo.
        (None, Some(_)) => panic!("el 85 % no tiene fecha y el 99 % sí: el criterio no es monótono"),
        _ => {}
    }
}

/// **Las fechas al 100 % y al 90 % encierran a la del umbral del perfil.**
///
/// Misma monotonía del criterio, aplicada a las dos fechas de referencia del nivel 2 con el umbral
/// por defecto (95 %) en medio. Misma tolerancia y por la misma razón.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn the_hundred_and_ninety_dates_bracket_the_ninety_five() {
    use futurefin_engine_stochastic::MAX_CONFIRMATION_ADVANCES;

    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let body = settle_plan_extras(&app, &owner.cookie).await;
    assert_eq!(
        body["success_threshold_pct"], 95,
        "este test asume el umbral por defecto en medio de los dos"
    );
    let tol = u64::from(MAX_CONFIRMATION_ADVANCES);
    let at_90 = body["safe_date_at_90_month_index"].as_u64();
    let at_95 = body["jubilacion_month_index"].as_u64();
    let at_100 = body["safe_date_at_100_month_index"].as_u64();

    if let (Some(a), Some(b)) = (at_90, at_95) {
        assert!(a <= b + tol, "el 90 % ({a}) no puede jubilar después del 95 % ({b})");
    }
    if let (Some(b), Some(c)) = (at_95, at_100) {
        assert!(b <= c + tol, "el 95 % ({b}) no puede jubilar después del 100 % ({c})");
    }
    // Y la ausencia también ordena: si no hay fecha al 90 %, no puede haberla al 100 %.
    if at_90.is_none() {
        assert!(
            at_100.is_none(),
            "sin fecha al 90 % no puede haberla al 100 %: el criterio es más estricto"
        );
    }
}

// =================================================================================================
// La honestidad de las cifras
// =================================================================================================

/// **La barra de error nunca es 0, tampoco con cero fallos.**
///
/// Es el sentido entero de usar el intervalo de Wilson en vez de la aproximación normal: con
/// `p̂ = 1` la normal da una barra EXACTAMENTE cero y la app declararía «100 % seguro» con una
/// muestra finita, que es la clase de número que esta casa no publica. Con 0 fallos de 2.500 la
/// barra vale 0,1534 pp — pequeña, pero nunca nula.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn the_sampling_error_is_not_zero_with_zero_failures() {
    let app = TestApp::spawn().await;
    // Cartera enorme y gasto pequeño: ningún camino falla, que es justo el caso que importa.
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Fondo").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": "Indexado",
                "current_value": "5000000",
                "expected_annual_return_percent": "5",
                "annual_volatility_percent": "10",
                "is_liquid": true,
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "crear activo: {r:?}");
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let body = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    let error_pp: f64 = body["success_sampling_error_pp"]
        .as_str()
        .expect("la barra viaja como decimal-string")
        .parse()
        .expect("decimal");
    assert!(
        error_pp > 0.0,
        "una barra de error de 0 pp diría «100 % seguro» con una muestra finita: {body}"
    );
    let low: f64 = body["success_wilson_low"]
        .as_str()
        .expect("la cota de Wilson viaja como decimal-string")
        .parse()
        .expect("decimal");
    assert!(low < 1.0, "la cota inferior de Wilson nunca llega a 1: {low}");
}

/// **Un plan sin fecha alcanzable publica su mejor intento, no una fecha.**
///
/// «No llegas» y «llegas en el mes 0» son respuestas opuestas, y la segunda es la que sale sola si
/// alguien rellena la ausencia con el valor que más se le parece. Lo que se publica es
/// `not_reachable` con el `best_effort` al lado — «lo más cerca que llegas es el 78 % a los 67».
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn not_reachable_publishes_best_effort_not_a_date() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Gasto por encima del ingreso y sin cartera: no hay ningún mes del horizonte en que este
    // hogar pueda jubilarse.
    let income_cat = app.create_category(&owner, "income", "Nómina").await;
    let expense_cat = app.create_category(&owner, "expense", "Vida").await;
    for (cat, amount) in [(income_cat, "1200"), (expense_cat, "1800")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget",
                serde_json::json!({"category_id": cat, "amount": amount}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "crear presupuesto: {r:?}");
    }
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let body = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_eq!(body["retirement_date_basis"], "not_reachable", "{body}");
    assert!(
        body["jubilacion_month_index"].is_null(),
        "sin fecha se publica null, JAMÁS un 0 — un 0 se lee como «ya puedes»: {body}"
    );
    assert!(
        !body["success_of_plan"].is_null(),
        "sin fecha sigue habiendo una medición: la del mejor intento"
    );
    // Y el capital necesario dice por qué falta en vez de publicar 0 €.
    assert!(
        body["needed_capital_today"].is_null(),
        "un hogar sin líquido no necesita 0 €: {body}"
    );
    assert!(
        body["needed_capital_absent_reason"].is_string(),
        "la ausencia se nombra: {body}"
    );
}

// =================================================================================================
// Cuándo NO hay plan
// =================================================================================================

/// **Un horizonte a medida publica la línea, no la fecha del sorteo.**
///
/// `?months=` salta la cache por diseño (D7), y con ella salta el plan: resolver una fecha por
/// bisección en cada petición con horizonte arbitrario pondría veinticinco segundos de CPU detrás
/// de un parámetro de query — el agujero exacto que `heavy.rs` documenta. La serie sale igual, con
/// `plan_absent_reason` diciendo por qué.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn a_months_override_publishes_the_line_without_the_stochastic_date() {
    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let r = app
        .get_with_cookie("/v1/projection/series?months=240", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK);
    let body = r.json();
    assert!(
        body["points"].as_array().is_some_and(|p| !p.is_empty()),
        "la serie determinista se publica igual"
    );
    assert_eq!(body["plan_absent_reason"], "months_override", "{body}");
    assert!(body["jubilacion_month_index"].is_null(), "{body}");
    assert!(body["needed_capital_today"].is_null(), "{body}");
    // Y no deja rastro en la cache de plan: un horizonte a medida no puebla nada.
    assert!(
        app.state.plan_cache.read().await.is_empty(),
        "un `?months=` no puede sembrar la cache de plan"
    );
}

/// **Sin fecha de nacimiento no hay plan, y se dice por qué.**
///
/// Sin ella no hay edad que convertir en mes: ni horizonte por esperanza de vida, ni edad
/// objetivo, ni eje de edades. La respuesta correcta es la ausencia CON su razón — nunca un 500
/// por un campo opcional del perfil (§A del plan de #207), y nunca una fecha inventada a partir de
/// la edad de otro miembro del hogar.
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn without_birth_date_the_plan_is_absent_with_its_reason() {
    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            serde_json::json!({"birth_date": Value::Null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "borrar la fecha de nacimiento: {r:?}");
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let body = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_eq!(body["plan_absent_reason"], "birth_date_missing", "{body}");
    assert!(
        body["retirement_date_basis"].is_null(),
        "sin plan no hay base de fecha que declarar: {body}"
    );
    assert!(
        body["points"].as_array().is_some_and(|p| !p.is_empty()),
        "la serie sigue publicándose: un campo opcional del perfil no tumba una lectura"
    );
}

// =================================================================================================
// La densidad no cambia el plan
// =================================================================================================

/// El plan es del HOGAR, no de la rejilla con la que se dibuja: las dos densidades comparten
/// entrada de plan y publican exactamente las mismas cifras.
///
/// La densidad decima `points[]`; no toca ni el horizonte simulado ni el sorteo. Dos fechas
/// distintas en `?density=hybrid` y `?density=monthly` serían dos respuestas a la misma pregunta
/// en la misma pantalla — la SPA pide las dos a la vez (two-phase loading).
#[tokio::test]
#[ignore = "el bloque «plan» de la serie lo publica A5; A5 debe QUITAR este ignore"]
async fn both_densities_share_one_plan() {
    let app = TestApp::spawn().await;
    let owner = household_with_a_plan(&app, "alice").await;
    let iid = app.installation_id().await;
    app.settle_login_warmup(iid).await;

    let hybrid = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();
    let monthly = app
        .get_with_cookie("/v1/projection/series?density=monthly", &owner.cookie)
        .await
        .json();
    assert_eq!(hybrid["jubilacion_month_index"], monthly["jubilacion_month_index"]);
    assert_eq!(hybrid["success_of_plan"], monthly["success_of_plan"]);
    assert_eq!(hybrid["needed_capital_today"], monthly["needed_capital_today"]);

    let key_hybrid = app
        .state
        .projection_cache_plan_key(&plan_view_key(iid, owner.user_id, Density::Hybrid))
        .await;
    let key_monthly = app
        .state
        .projection_cache_plan_key(&plan_view_key(iid, owner.user_id, Density::Monthly))
        .await;
    assert_eq!(
        key_hybrid, key_monthly,
        "la densidad no entra en la huella del plan: es una decisión de SERIALIZACIÓN"
    );
}

/// La clave de la serie de `view=mine` en una densidad concreta.
fn plan_view_key(
    installation_id: Uuid,
    user_id: Uuid,
    density: Density,
) -> futurefin_api::state::ProjectionCacheKey {
    futurefin_api::state::ProjectionCacheKey {
        installation_id,
        view: futurefin_api::handlers::person_view::LedgerView::Mine,
        owner_user_id: Some(user_id),
        density,
    }
}

/// Nota para quien venga a quitar los `#[ignore]`: el literal está arriba, en [`PENDING_A5`], y
/// esta función existe para que un `cargo check` avise si alguien lo borra sin quitar los atributos
/// (que llevan el mismo texto escrito a mano, porque `#[ignore = ...]` no admite una constante).
#[test]
fn the_pending_reason_is_written_once() {
    assert!(PENDING_A5.contains("A5"), "la razón tiene que nombrar al WP que la cierra");
}
