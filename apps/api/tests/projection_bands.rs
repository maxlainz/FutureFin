//! **`GET /v1/projection/bands`** — la superficie HTTP de Monte Carlo (5.0.0, WP A6).
//!
//! Lo que estos tests compran, en orden de importancia:
//!
//! 1. **σ = 0 ⇒ la banda ES la línea determinista.** Es el único gate que ata el camino `f64` al
//!    camino `Decimal` que la app publica como dinero: si el ensamblado del endpoint tomara otro
//!    input —otro horizonte, otro scope—, la banda seguiría saliendo bonita y el error sería
//!    invisible. Aquí se compara punto a punto contra `/v1/projection/series`.
//! 2. **Sin jubilación no hay éxito que publicar** (6.0.0, D-7): las tres cifras del éxito viajan
//!    a `null` con `success_absent_reason: "no_retirement_phase"` en vez de un 100 % mecánico.
//! 3. **El vector de volatilidades sigue el orden de los activos.** Un vector descolocado produce
//!    bandas ESTRECHAS Y CREÍBLES, que es el peor fallo posible en esta superficie.
//! 4. **Reproducibilidad**: misma semilla ⇒ mismo cuerpo byte a byte; otra semilla ⇒ otro mercado.
//! 5. **El hogar no tiene bandas** (400 declarado) y el cache se invalida con las mismas
//!    mutaciones que la serie.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::state::BandsCacheKey;
use serde_json::{json, Value};
use uuid::Uuid;

/// Caminos de los tests. **Deliberadamente pocos**: lo que se comprueba aquí es el ensamblado, la
/// rejilla y el contrato, no la convergencia estadística — y en `debug` cada camino cuesta un
/// orden de magnitud más que en release (0,2 ms/camino medidos en release, §doc del módulo).
const PATHS: u32 = 24;

/// Los tests que miran la DISPERSIÓN suben a este: con 24 caminos la banda es demasiado ruidosa
/// para comparar anchos.
const PATHS_SPREAD: u32 = 120;

async fn bands(app: &TestApp, cookie: &str, q: &str) -> Value {
    let r = app
        .get_with_cookie(&format!("/v1/projection/bands{q}"), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET bands{q}: {r:?}");
    r.json()
}

/// La serie, **recomputada a propósito**.
///
/// La invalidación previa no es higiene decorativa: el warm-up que el login lanza en
/// `tokio::spawn` puede aterrizar **después** de que el test haya sembrado el hogar, repoblando
/// la cache con la proyección del hogar VACÍO —`settle_login_warmup` solo espera un segundo—. Un
/// test que compare la banda contra esa serie compara contra una línea de ceros y falla
/// culpando a las bandas. Invalidar justo antes del GET fuerza el recompute y hace la
/// comparación determinista.
async fn series(app: &TestApp, cookie: &str) -> Value {
    let iid = app.installation_id().await;
    app.state.invalidate_projection_by_installation(iid).await;
    let r = app
        .get_with_cookie("/v1/projection/series?density=hybrid", cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET series: {r:?}");
    r.json()
}

/// Presupuesto + N activos. Devuelve los ids en el mismo orden de creación, que con
/// `sort_index`/`name` iguales es el orden que ve el motor.
async fn seed(
    app: &TestApp,
    u: &LoggedInOwner,
    income: &str,
    expense: &str,
    assets: &[(&str, &str, Option<&str>)],
) -> Vec<String> {
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
    let mut ids = Vec::new();
    for (name, value, vol) in assets {
        let mut body = json!({
            "category_id": ast, "name": name, "current_value": value,
            "is_liquid": true, "expected_annual_return_percent": "5",
        });
        if let Some(v) = vol {
            body["annual_volatility_percent"] = json!(v);
        }
        let r = app
            .post_json_with_cookie("/v1/assets", body, &u.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
        ids.push(r.json()["id"].as_str().expect("asset id").to_string());
    }
    ids
}

/// La clave del cache de bandas.
fn key(iid: Uuid, user_id: Uuid, paths: u32, seed: &str) -> BandsCacheKey {
    BandsCacheKey {
        installation_id: iid,
        user_id,
        paths,
        seed: seed.parse().expect("semilla decimal"),
    }
}

fn f(v: &Value) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("se esperaba un número, llegó {v}"))
}

/// Los dos contadores de `failures_by_kind`, en su orden fijo `[F1, F2]`. (Eran tres hasta el
/// 2026-10-10: F3, el fallo de las reglas de retirada por saldo, se retiró con ellas.)
fn failures_by_kind(b: &Value) -> [u64; 2] {
    let a = b["failures_by_kind"].as_array().expect("failures_by_kind");
    assert_eq!(a.len(), 2, "el reparto tiene DOS motivos y solo dos: {b}");
    [a[0].as_u64().expect("F1"), a[1].as_u64().expect("F2")]
}

/// **Sin fase de jubilación no hay éxito que publicar** (6.0.0, D-7): el motor solo cuenta
/// fallos estando jubilado, así que el sorteo devolvería mecánicamente un `1`. Las tres cifras
/// viajan a `null` —presentes, no omitidas— con su motivo, y los contadores son cero.
fn assert_success_is_absent(b: &Value) {
    for k in ["success_of_plan", "success_wilson_low", "success_sampling_error_pp"] {
        assert!(
            b.get(k).is_some_and(Value::is_null),
            "`{k}` viaja como null explícito: {b}"
        );
    }
    assert_eq!(b["success_absent_reason"], "no_retirement_phase", "{b}");
    assert_eq!(failures_by_kind(b), [0, 0], "sin jubilación no hay fallo que contar: {b}");
}

/// **Los campos del modelo VIEJO no pueden volver por la puerta de atrás.** Un cliente que
/// todavía los leyera vería `undefined` y no un número equivocado, pero un servidor que los
/// reintrodujera publicaría dos definiciones de éxito a la vez.
fn assert_the_v1_fields_are_gone(b: &Value) {
    for dead in [
        "success_probability",
        "never_retired_probability",
        "success_given_retired",
        "retirement_month_index_percentiles",
        "underfunded_probability",
        "depletion_probability_by_age",
        "retirement_trigger",
        // 6.0.0 (D-7): retirados con el modelo de jubilación.
        "strategy",
        "success_threshold_pct",
        "success_verdict",
    ] {
        assert!(
            b.get(dead).is_none(),
            "`{dead}` se retiró y no puede volver: {b}"
        );
    }
}

/// Ancho de la banda en el ÚLTIMO punto, relativo a la mediana: la medida de dispersión que un
/// vector de volatilidades descolocado falsearía.
fn relative_spread(b: &Value) -> f64 {
    let last = b["points"].as_array().expect("puntos").last().expect("último");
    let p10 = f(&last["net_worth_p10"]);
    let p50 = f(&last["net_worth_p50"]);
    let p90 = f(&last["net_worth_p90"]);
    assert!(p50 > 0.0, "la mediana debe ser positiva para normalizar: {last}");
    (p90 - p10) / p50
}

/// El cuerpo sin `computed_in_ms` — el único campo que NO es función de la entrada (es un reloj).
fn without_timing(mut v: Value) -> Value {
    v.as_object_mut().expect("objeto").remove("computed_in_ms");
    v
}

// ---------------------------------------------------------------------------------------------
// 1. Scope y forma de la respuesta
// ---------------------------------------------------------------------------------------------

/// `mine` responde y ecoa su vista; `household` es un 400 **declarado**, no un 500 ni una banda
/// inventada sumando percentiles que no suman. De paso fija la FORMA de la respuesta v2: qué
/// campos hay, cuáles murieron y qué rejilla comparte con la serie.
#[tokio::test]
async fn bands_exist_for_mine_and_the_household_is_a_declared_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("15"))]).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["view"], "mine", "{b}");
    assert_eq!(b["paths"], PATHS, "{b}");
    assert_eq!(b["percentiles"], json!([10, 50, 90]), "{b}");
    assert_eq!(b["any_volatility_declared"], true, "{b}");
    assert_success_is_absent(&b);
    assert_the_v1_fields_are_gone(&b);
    assert!(
        b["model_note"]
            .as_str()
            .expect("nota")
            .contains("SIN JUBILACIÓN"),
        "la nota debe declarar por qué no hay éxito: {b}"
    );

    // La rejilla es la MISMA que la de la serie: mismo primer y último `month_index`.
    let s = series(&app, &owner.cookie).await;
    let bi: Vec<u64> = b["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["month_index"].as_u64().unwrap())
        .collect();
    let si: Vec<u64> = s["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["month_index"].as_u64().unwrap())
        .collect();
    assert_eq!(bi, si, "bandas y serie deben compartir rejilla punto a punto");

    let r = app
        .get_with_cookie("/v1/projection/bands?view=household", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "household_bands_unavailable", "{r:?}");
}

// ---------------------------------------------------------------------------------------------
// 2. Reproducibilidad
// ---------------------------------------------------------------------------------------------

/// Misma semilla ⇒ **el mismo cuerpo**, recomputado (se vacía la cache entre las dos llamadas para
/// que lo que se pruebe sea el determinismo del sorteo y no el del `HashMap`). Otra semilla ⇒ otro
/// mercado, y por tanto otras bandas.
#[tokio::test]
async fn the_same_seed_reproduces_the_body_and_another_seed_does_not() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("18"))]).await;
    let iid = app.installation_id().await;

    let q = format!("?paths={PATHS}&seed=424242");
    let first = without_timing(bands(&app, &owner.cookie, &q).await);
    // Vaciar el cache: el segundo GET tiene que volver a sortear.
    app.state.invalidate_projection_by_installation(iid).await;
    assert!(
        app.state.bands_cache.read().await.is_empty(),
        "la invalidación por instalación debe vaciar también las bandas"
    );
    let second = without_timing(bands(&app, &owner.cookie, &q).await);
    assert_eq!(first, second, "misma semilla, mismo resultado");
    assert_eq!(first["seed"], "424242", "la semilla se ecoa como STRING: {first}");

    app.state.invalidate_projection_by_installation(iid).await;
    let other = without_timing(
        bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=999999")).await,
    );
    assert_ne!(
        first["points"], other["points"],
        "otra semilla es otro mercado: las bandas no pueden coincidir"
    );
}

/// La semilla por defecto es **estable por usuario**: sin `?seed=`, dos ejecuciones separadas por
/// una invalidación devuelven exactamente lo mismo. Sin esto, la probabilidad de éxito bailaría a
/// cada refresco y el KPI del Resumen no valdría nada.
#[tokio::test]
async fn the_default_seed_is_stable_for_a_user() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("18"))]).await;
    let iid = app.installation_id().await;

    let q = format!("?paths={PATHS}");
    let first = without_timing(bands(&app, &owner.cookie, &q).await);
    app.state.invalidate_projection_by_installation(iid).await;
    let second = without_timing(bands(&app, &owner.cookie, &q).await);
    assert_eq!(first, second, "la semilla estable debe reproducir el sorteo");
    // Y es una semilla de 64 bits publicada como dígitos: `JSON.parse` la redondearía como número.
    let seed = first["seed"].as_str().expect("la semilla viaja como string");
    assert!(
        seed.parse::<u64>().is_ok(),
        "la semilla debe ser un u64 en dígitos decimales: {seed}"
    );
}

/// Una semilla que no es un `u64` se **rechaza**. Caer en silencio a la estable devolvería «el
/// sorteo de siempre» y sería indistinguible de haber funcionado.
#[tokio::test]
async fn a_malformed_seed_is_rejected_instead_of_falling_back() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", None)]).await;

    for raw in ["abc", "-1", "18446744073709551616"] {
        let r = app
            .get_with_cookie(
                &format!("/v1/projection/bands?paths={PATHS}&seed={raw}"),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "seed={raw}: {r:?}");
        assert_eq!(r.json()["code"], "invalid_seed", "seed={raw}: {r:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// 3. El gate: σ = 0 ⇒ la banda ES la línea
// ---------------------------------------------------------------------------------------------

/// **Sin volatilidad declarada, los tres percentiles coinciden con la serie determinista.**
///
/// Es el único punto donde el camino `f64` se mide contra el `Decimal` que la app publica como
/// dinero.
///
/// La tolerancia es RELATIVA (1e-6) porque la degeneración del camino genérico está medida en
/// ≤ 1,5e-7 € sobre patrimonios de seis cifras — un umbral absoluto en euros mentiría sobre lo
/// que se está comprobando.
#[tokio::test]
async fn zero_volatility_makes_the_band_the_deterministic_line() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(
        &app,
        &owner,
        "3000",
        "2000",
        &[("Cuenta", "5000", None), ("Indexado", "20000", Some("0"))],
    )
    .await;

    // La SERIE primero: `series()` invalida y recomputa, así que la línea contra la que se
    // compara es la de este hogar y no la que el warm-up del login dejó a medias.
    let s = series(&app, &owner.cookie).await;
    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(
        b["any_volatility_declared"], false,
        "un 0 explícito y un NULL son los dos «activo determinista»: {b}"
    );

    let bp = b["points"].as_array().expect("bandas");
    let sp = s["points"].as_array().expect("serie");
    assert_eq!(bp.len(), sp.len(), "misma rejilla");
    for (pb, ps) in bp.iter().zip(sp.iter()) {
        assert_eq!(pb["month_index"], ps["month_index"]);
        let det = f(&ps["net_worth"]);
        let liq = f(&ps["net_worth_liquid"]);
        for band in ["net_worth_p10", "net_worth_p50", "net_worth_p90"] {
            let got = f(&pb[band]);
            assert!(
                (got - det).abs() <= 1e-6 * det.abs().max(1.0),
                "mes {}: {band} = {got}, determinista = {det}",
                pb["month_index"]
            );
        }
        for band in [
            "net_worth_liquid_p10",
            "net_worth_liquid_p50",
            "net_worth_liquid_p90",
        ] {
            let got = f(&pb[band]);
            assert!(
                (got - liq).abs() <= 1e-6 * liq.abs().max(1.0),
                "mes {}: {band} = {got}, líquido determinista = {liq}",
                pb["month_index"]
            );
        }
    }

    assert_success_is_absent(&b);
}

// ---------------------------------------------------------------------------------------------
// 4. El vector de volatilidades sigue el orden de los activos
// ---------------------------------------------------------------------------------------------

/// **El fallo que este test existe para cazar es silencioso**: si el vector de volatilidades se
/// descolocara respecto de `input.assets`, las bandas seguirían saliendo —más estrechas y
/// perfectamente creíbles— y ningún otro assert protestaría.
///
/// Se prueba por COMPORTAMIENTO. Dos activos con el mismo orden estable (`sort_index`, luego
/// nombre): «Aaa» con 200.000 € y «Bbb» con 2.000 €. Con la volatilidad en el grande la banda es
/// ancha; moviéndola al pequeño se estrecha en dos órdenes de magnitud. Con el vector invertido,
/// las dos mediciones se intercambiarían y el `assert` de abajo fallaría en ambos sentidos.
#[tokio::test]
async fn the_volatility_vector_follows_the_asset_order() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ids = seed(
        &app,
        &owner,
        "3000",
        "2000",
        &[("Aaa grande", "200000", Some("30")), ("Bbb pequeno", "2000", None)],
    )
    .await;
    let iid = app.installation_id().await;

    let q = format!("?paths={PATHS_SPREAD}&seed=7");
    let wide = relative_spread(&bands(&app, &owner.cookie, &q).await);

    // Mover la volatilidad al activo PEQUEÑO. El tri-estado de `annual_volatility_percent`
    // (`null` = borrar) es lo que permite dejar el grande determinista.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{}", ids[0]),
            json!({"annual_volatility_percent": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{}", ids[1]),
            json!({"annual_volatility_percent": "30"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    app.state.invalidate_projection_by_installation(iid).await;

    let narrow = relative_spread(&bands(&app, &owner.cookie, &q).await);
    assert!(
        wide > narrow * 5.0,
        "la volatilidad en el activo GRANDE tiene que ensanchar mucho más que en el pequeño \
         (ancha {wide}, estrecha {narrow}); si se parecen, el vector de σ no está alineado con \
         `input.assets`"
    );
}

// ---------------------------------------------------------------------------------------------
// 5. Cotas de `paths`
// ---------------------------------------------------------------------------------------------

/// `paths` fuera de rango es un 400, **nunca un clamp**: servir 5.000 caminos a quien pidió
/// 50.000 es contestar otra pregunta con cara de haber contestado la suya.
///
/// El techo subió a **5.000** en 5.0.0 (el default, a 2.500), así que lo que aquí se fija es el
/// borde nuevo: 5.001 se rechaza. El borde superior VÁLIDO no se ejercita aquí —sortear 5.000
/// caminos en `debug` es la operación más cara de la suite— sino en
/// `query_param_validation.rs::the_exact_bounds_of_every_numeric_window_still_work`, sobre un
/// hogar vacío.
#[tokio::test]
async fn paths_out_of_range_is_rejected_not_clamped() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", None)]).await;

    for raw in ["0", "5001", "100000"] {
        let r = app
            .get_with_cookie(
                &format!("/v1/projection/bands?paths={raw}"),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "paths={raw}: {r:?}");
        assert_eq!(r.json()["code"], "paths_out_of_range", "paths={raw}: {r:?}");
    }
    // Lo que ANTES estaba fuera de rango ahora entra: el techo viejo era 2.000.
    let b = bands(&app, &owner.cookie, "?paths=2001").await;
    assert_eq!(b["paths"], 2001, "{b}");
    // El borde inferior SÍ es válido: un solo camino es una pregunta legítima (y barata).
    let b = bands(&app, &owner.cookie, "?paths=1").await;
    assert_eq!(b["paths"], 1, "{b}");
}

// ---------------------------------------------------------------------------------------------
// 6. Cache
// ---------------------------------------------------------------------------------------------

/// HIT/MISS con centinela (el mismo patrón que `projection_cache.rs`: se envenena la entrada, y si
/// el siguiente GET la devuelve es que salió de la cache) e invalidación por una mutación del
/// ledger.
#[tokio::test]
async fn the_bands_cache_serves_hits_and_dies_with_the_projection() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ids = seed(
        &app,
        &owner,
        "3000",
        "2000",
        &[("Indexado", "20000", Some("15"))],
    )
    .await;
    let iid = app.installation_id().await;

    let q = format!("?paths={PATHS}&seed=11");
    let first = bands(&app, &owner.cookie, &q).await;
    let k = key(iid, owner.user_id, PATHS, "11");
    assert!(
        app.state.bands_cache.read().await.contains_key(&k),
        "el primer GET debe dejar la entrada"
    );

    // Centinela: si el segundo GET lo devuelve, se sirvió de la cache.
    const SENTINEL: &str = "SENTINEL-bands-hit";
    let poisoned = {
        let cache = app.state.bands_cache.read().await;
        let mut resp = (*cache.get(&k).expect("entrada recién insertada").response).clone();
        resp.model_note = SENTINEL.to_string();
        resp
    };
    // La generación vigente: no hay cómputo lento entre leerla e insertar, así que la entrada
    // entra (5.0.0, WP A12 — la guardia se prueba aparte, en `projection_cache.rs`).
    let generation = app.state.projection_generation(iid).await;
    app.state
        .bands_cache_insert_if_current(k.clone(), generation, std::sync::Arc::new(poisoned))
        .await;
    let hit = bands(&app, &owner.cookie, &q).await;
    assert_eq!(hit["model_note"], SENTINEL, "el segundo GET debió ser un HIT");

    // Una clave distinta (otros caminos) NO es un hit: `paths` es parte de la pregunta.
    let other = bands(&app, &owner.cookie, &format!("?paths={}&seed=11", PATHS + 1)).await;
    assert_ne!(other["model_note"], SENTINEL, "otro `paths` es otra entrada");

    // Mutación del ledger: PATCH de un activo.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{}", ids[0]),
            json!({"current_value": "31000"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(
        !app.state.bands_cache.read().await.contains_key(&k),
        "un PATCH de activo debe invalidar las bandas igual que la serie"
    );
    let after = bands(&app, &owner.cookie, &q).await;
    assert_ne!(
        after["points"], first["points"],
        "la banda debe reflejar el activo nuevo"
    );
}

/// El logout borra las bandas del usuario junto a su proyección: son suyas por construcción
/// (`view=mine`).
#[tokio::test]
async fn logout_drops_the_bands_of_that_user() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", None)]).await;
    let iid = app.installation_id().await;

    bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=5")).await;
    assert!(
        app.state
            .bands_cache
            .read()
            .await
            .contains_key(&key(iid, owner.user_id, PATHS, "5")),
        "la entrada debe existir antes del logout"
    );
    app.state.invalidate_projection_by_user(owner.user_id).await;
    assert!(
        app.state.bands_cache.read().await.is_empty(),
        "el logout debe llevarse las bandas del usuario"
    );
}

// ---------------------------------------------------------------------------------------------
// 7. Tamaño del payload
// ---------------------------------------------------------------------------------------------

/// **La medida del presupuesto de contexto**, con el número impreso para que quede en el log del
/// CI en vez de en la memoria de nadie.
///
/// El tope no es estético: la respuesta entera viaja a la tool MCP `get_projection_bands`, y ahí
/// compite con el resto de la conversación. 32 KB es holgado para la densidad `hybrid` con las
/// SEIS series (las tres del patrimonio y las tres del líquido) y deja margen para un horizonte
/// de 840 meses con patrimonios de siete cifras; si algún día se rompe, la salida es quitar las
/// bandas del líquido (ya opt-in en la tool), no subir la constante.
///
/// **El payload es independiente de `paths`** salvo por el eco del propio número: los puntos son
/// percentiles, no caminos. Por eso se mide con una muestra pequeña — el default de 2.500 no
/// cambiaría ni un byte de `points[]` y cuesta cien veces más. La `model_note` y la curva de
/// fallo se imprimen aparte para que el margen esté medido y no supuesto.
#[tokio::test]
async fn the_hybrid_payload_stays_within_the_context_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(
        &app,
        &owner,
        "9000",
        "3000",
        &[("Indexado", "900000", Some("18"))],
    )
    .await;

    // Los tiempos se IMPRIMEN, no se afirman: un umbral de reloj en CI enseña a ignorar los
    // fallos (misma doctrina que `timing_mc.rs`).
    let q = format!("/v1/projection/bands?paths={PATHS_SPREAD}");
    let t0 = std::time::Instant::now();
    let r = app.get_with_cookie(&q, &owner.cookie).await;
    let miss_ms = t0.elapsed().as_millis();
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let t1 = std::time::Instant::now();
    let hit = app.get_with_cookie(&q, &owner.cookie).await;
    let hit_ms = t1.elapsed().as_millis();
    assert_eq!(hit.body, r.body, "el HIT debe devolver el mismo cuerpo");

    let bytes = r.body.len();
    let v = r.json();
    let points = v["points"].as_array().expect("puntos").len();
    let note = v["model_note"].as_str().expect("nota").len();
    let curve = v["failure_probability_by_age"].as_array().expect("curva").len();
    let sin_liquido = bytes
        - v["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                // Lo que la tool MCP ahorra sin `include_liquid_bands`: las tres claves y sus
                // valores, medidas sobre el JSON real en vez de estimadas.
                ["net_worth_liquid_p10", "net_worth_liquid_p50", "net_worth_liquid_p90"]
                    .iter()
                    .map(|k| k.len() + 4 + p[*k].to_string().len())
                    .sum::<usize>()
            })
            .sum::<usize>();
    println!(
        "[bands-payload] hybrid · {points} puntos · {bytes} bytes ({} caminos) · sin bandas de \
         líquido ≈ {sin_liquido} bytes · model_note {note} B · curva de fallo {curve} filas · \
         MISS {miss_ms} ms · HIT {hit_ms} ms · motor {} ms (perfil {})",
        v["paths"],
        v["computed_in_ms"],
        if cfg!(debug_assertions) { "debug" } else { "release" },
    );
    assert!(
        bytes <= 32_000,
        "el payload de bandas a densidad hybrid pesa {bytes} bytes ({points} puntos, nota de \
         {note} B, curva de {curve} filas) y el presupuesto es 32.000 — quita las bandas del \
         líquido antes de subir la cota"
    );
}

