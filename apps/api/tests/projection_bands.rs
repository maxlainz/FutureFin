//! **`GET /v1/projection/bands`** — la superficie HTTP de Monte Carlo (5.0.0, WP6b).
//!
//! Lo que estos tests compran, en orden de importancia:
//!
//! 1. **σ = 0 ⇒ la banda ES la línea determinista.** Es el único gate que ata el camino `f64` al
//!    camino `Decimal` que la app publica como dinero: si el ensamblado del endpoint tomara otro
//!    input —otro perfil, otro horizonte, otro scope—, la banda seguiría saliendo bonita y el
//!    error sería invisible. Aquí se compara punto a punto contra `/v1/projection/series`.
//! 2. **El vector de volatilidades sigue el orden de los activos.** Un vector descolocado produce
//!    bandas ESTRECHAS Y CREÍBLES, que es el peor fallo posible en esta superficie. Se prueba por
//!    comportamiento: con la volatilidad en el activo grande la banda es ancha, y moviéndola al
//!    pequeño se estrecha — con el vector invertido las dos mediciones se intercambiarían.
//! 3. **Reproducibilidad**: misma semilla ⇒ mismo cuerpo byte a byte; otra semilla ⇒ otro mercado.
//! 4. **El hogar no tiene bandas** (400 declarado) y el cache se invalida con las mismas
//!    mutaciones que la serie.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::state::BandsCacheKey;
use serde_json::{json, Value};
use uuid::Uuid;

/// Caminos de los tests. **Deliberadamente pocos**: lo que se comprueba aquí es el ensamblado, la
/// rejilla y el contrato, no la convergencia estadística — y en `debug` cada camino cuesta un
/// orden de magnitud más que en release (0,2 ms/camino medidos en release, §doc del módulo). Los
/// tests que miran la DISPERSIÓN suben a `PATHS_SPREAD`, que sigue siendo barato.
const PATHS: u32 = 24;
const PATHS_SPREAD: u32 = 120;

async fn bands(app: &TestApp, cookie: &str, q: &str) -> Value {
    let r = app
        .get_with_cookie(&format!("/v1/projection/bands{q}"), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET bands{q}: {r:?}");
    r.json()
}

async fn patch_profile(app: &TestApp, u: &LoggedInOwner, body: Value) {
    let r = app
        .patch_json_with_cookie("/v1/auth/me/retirement-profile", body, &u.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil: {r:?}");
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

/// Una probabilidad publicada: string decimal → `f64`. Va por `as_str` a propósito: si algún día
/// una de estas cifras dejara de viajar como cadena, el test debe caerse aquí y no comparar
/// silenciosamente contra un `null` convertido en 0.
fn prob(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un string decimal, llegó {v}"))
        .parse()
        .expect("probabilidad parseable")
}

/// **La identidad del éxito**: un camino que no se jubila no puede contar como éxito, así que
/// `success_probability ≤ 1 − never_retired_probability`. Con trigger por edad
/// `never_retired_probability` es 0 y la cota es trivial; con trigger por cruce es la que ata las
/// dos cifras y la que la definición vieja violaba (publicaba 0,96 con el 33,1 % sin jubilarse).
///
/// La tolerancia es el redondeo de publicación: las dos salen redondeadas a 6 decimales.
fn assert_success_identity(b: &Value) {
    let success = prob(&b["success_probability"]);
    let never = prob(&b["never_retired_probability"]);
    assert!(
        success <= 1.0 - never + 1e-6,
        "success_probability ({success}) > 1 − never_retired_probability ({never}): un camino \
         que no se jubila no puede ser un éxito — {b}"
    );
    // Y el condicional, cuando existe, es el mismo numerador sobre el denominador correcto:
    // `success · 1 = given_retired · (1 − never)` con trigger por CRUCE. Con trigger por EDAD el
    // numerador de `success` incluye los caminos sin jubilación, así que la identidad no aplica.
    if b["retirement_trigger"] == "liquid_crossing" {
        if let Some(g) = b["success_given_retired"].as_str() {
            let given: f64 = g.parse().expect("probabilidad parseable");
            assert!(
                (success - given * (1.0 - never)).abs() <= 1e-5,
                "success ({success}) debe ser given_retired ({given}) · (1 − never) ({}): {b}",
                1.0 - never
            );
        }
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
// 1. Scope
// ---------------------------------------------------------------------------------------------

/// `mine` responde y ecoa su vista; `household` es un 400 **declarado**, no un 500 ni una banda
/// inventada sumando percentiles que no suman.
#[tokio::test]
async fn bands_exist_for_mine_and_the_household_is_a_declared_400() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("15"))]).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["view"], "mine", "{b}");
    assert_eq!(b["paths"], PATHS, "{b}");
    assert_eq!(b["percentiles"], json!([10, 50, 90]), "{b}");
    assert_eq!(b["strategy"], "asap", "{b}");
    assert_eq!(b["retirement_trigger"], "liquid_crossing", "{b}");
    // 5.0.0 V7: el umbral configurable se retiró del perfil Y de la respuesta. El veredicto tiene
    // corte fijo, así que no hay nada que ecoar para poder auditarlo.
    assert_eq!(
        b["success_threshold_pct"],
        Value::Null,
        "el umbral se retiró en 5.0.0 y no puede volver por la puerta de atrás: {b}"
    );
    assert_eq!(b["any_volatility_declared"], true, "{b}");
    // P4/V6: el único activo es un fondo con σ = 15 %, así que no hay LÍQUIDO SIN RIESGO donde
    // alojar el colchón — y ése es el motivo que se publica. Desde que el colchón se DERIVA del
    // tope de la regla de ahorro, `not_requested` ya no existe: si no hay colchón es porque falló
    // una condición de la derivación, y ésa es la que hay que poder leer. Los dos contadores van
    // a `null` — «no se midió», que no es «cero rellenos».
    assert_eq!(b["buffer_active"], false, "{b}");
    assert_eq!(b["buffer_source"], "none", "{b}");
    assert_eq!(
        b["buffer_inactive_reason"], "no_safe_liquid_asset",
        "un colchón apagado sin motivo se lee como un fallo: {b}"
    );
    assert!(b["buffer_refills_p50"].is_null(), "{b}");
    assert!(b["buffer_refill_net_total_p50"].is_null(), "{b}");
    // Las tres cifras del éxito viajan JUNTAS: la probabilidad sola no dice si el plan ocurre.
    assert!(
        b["never_retired_probability"].is_string(),
        "la fracción de caminos que no se jubilan es el denominador escondido del éxito: {b}"
    );
    assert!(
        b["success_given_retired"].is_string() || b["success_given_retired"].is_null(),
        "el condicional viaja o es null (ningún camino se jubila), nunca ausente: {b}"
    );
    assert_success_identity(&b);
    assert!(
        b["model_note"].as_str().expect("nota").contains("no se agota"),
        "la nota debe declarar qué significa ÉXITO: {b}"
    );
    // La rejilla es la MISMA que la de la serie: mismo primer y último `month_index`.
    let s = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();
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
/// dinero. La tolerancia es RELATIVA (1e-6) porque la degeneración del camino genérico está
/// medida en ≤ 1,5e-7 € sobre patrimonios de seis cifras — un umbral absoluto en euros mentiría
/// sobre lo que se está comprobando.
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

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(
        b["any_volatility_declared"], false,
        "un 0 explícito y un NULL son los dos «activo determinista»: {b}"
    );
    let s = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();

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

    // Con σ = 0 todos los caminos son EL camino: el éxito solo puede ser 1 o 0, y aquí —una
    // cartera que crece y un plan que se jubila al cruzar— es exactamente 1.
    assert_eq!(
        b["success_probability"], "1",
        "sin dispersión el éxito es binario: {b}"
    );
    assert_eq!(b["success_verdict"], "green", "{b}");
    assert_eq!(
        s["assets_depleted_month_index"],
        Value::Null,
        "el camino determinista no se agota, así que el éxito debe ser 1: {s}"
    );
}

/// El espejo del anterior: un plan que **sí** se agota en el camino determinista da éxito `0`
/// exacto y veredicto rojo. Un plan sin activos que se jubila hoy y gasta se queda sin nada.
#[tokio::test]
async fn a_plan_that_depletes_deterministically_scores_zero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Gasto > ingreso y una hucha pequeña sin rentabilidad: la cartera se vacía y no vuelve.
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "500"), (&exp, "2500")] {
        app.post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": cat, "amount": amount, "ends_at_retirement": false}),
            &owner.cookie,
        )
        .await;
    }
    app.post_json_with_cookie(
        "/v1/assets",
        json!({"category_id": ast, "name": "Hucha", "current_value": "3000",
               "is_liquid": true, "expected_annual_return_percent": "0"}),
        &owner.cookie,
    )
    .await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["success_probability"], "0", "{b}");
    assert_eq!(b["success_verdict"], "red", "{b}");
    let s = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();
    assert_ne!(
        s["assets_depleted_month_index"],
        Value::Null,
        "el camino determinista debe agotarse para que el 0 signifique algo: {s}"
    );
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

/// `paths` fuera de rango es un 400, **nunca un clamp**: servir 2.000 caminos a quien pidió
/// 10.000 es contestar otra pregunta con cara de haber contestado la suya.
#[tokio::test]
async fn paths_out_of_range_is_rejected_not_clamped() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", None)]).await;

    for raw in ["0", "2001", "100000"] {
        let r = app
            .get_with_cookie(
                &format!("/v1/projection/bands?paths={raw}"),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "paths={raw}: {r:?}");
        assert_eq!(r.json()["code"], "paths_out_of_range", "paths={raw}: {r:?}");
    }
    // El borde inferior SÍ es válido: un solo camino es una pregunta legítima (y barata).
    let b = bands(&app, &owner.cookie, "?paths=1").await;
    assert_eq!(b["paths"], 1, "{b}");
}

// ---------------------------------------------------------------------------------------------
// 6. Cache
// ---------------------------------------------------------------------------------------------

/// HIT/MISS con centinela (el mismo patrón que `projection_cache.rs`: se envenena la entrada, y si
/// el siguiente GET la devuelve es que salió de la cache) e invalidación por los DOS caminos que
/// mueven la simulación: una mutación del ledger y un PATCH del perfil de jubilación.
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
    app.state
        .bands_cache_insert(k.clone(), std::sync::Arc::new(poisoned))
        .await;
    let hit = bands(&app, &owner.cookie, &q).await;
    assert_eq!(hit["model_note"], SENTINEL, "el segundo GET debió ser un HIT");

    // Una clave distinta (otros caminos) NO es un hit: `paths` es parte de la pregunta.
    let other = bands(&app, &owner.cookie, &format!("?paths={}&seed=11", PATHS + 1)).await;
    assert_ne!(other["model_note"], SENTINEL, "otro `paths` es otra entrada");

    // 1) Mutación del ledger: PATCH de un activo.
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

    // 2) PATCH del perfil de jubilación: cambia el plan entero, así que cambia el sorteo.
    patch_profile(&app, &owner, json!({"swr_pct": "3"})).await;
    assert!(
        !app.state.bands_cache.read().await.contains_key(&k),
        "un PATCH del perfil debe invalidar las bandas"
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
// 7. Veredicto y umbral
// ---------------------------------------------------------------------------------------------

/// **El corte del veredicto es FIJO al 100 %** (5.0.0, V7) y el umbral del perfil ya no existe:
/// un `PATCH` que lo mande se acepta y se descarta, sin error y sin efecto.
///
/// Con σ = 0 el éxito es exactamente 1, así que el verde de aquí es el verde estricto. Los tres
/// bordes del semáforo los fija el test unitario `el_verde_exige_todos_los_caminos` de
/// `handlers/projection_bands.rs`: allí la probabilidad se elige, aquí sale de un sorteo.
#[tokio::test]
async fn the_verdict_has_a_fixed_cut_and_the_threshold_is_accepted_and_ignored() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", None)]).await;

    // Se acepta (no rompe a ningún cliente que lo siga mandando) y no cambia nada.
    patch_profile(&app, &owner, json!({"success_threshold_pct": 60})).await;
    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["success_threshold_pct"], Value::Null, "{b}");
    assert_eq!(b["success_probability"], "1", "{b}");
    assert_eq!(b["success_verdict"], "green", "sin ningún camino agotado: {b}");
}

// ---------------------------------------------------------------------------------------------
// 8. Trigger por edad
// ---------------------------------------------------------------------------------------------

/// Con una estrategia por EDAD, `retirement_month_index_percentiles` es `null` (el mes es un dato
/// del plan, no una distribución) y aparece `underfunded_probability`. Son excluyentes por
/// construcción y `retirement_trigger` explica cuál toca.
#[tokio::test]
async fn an_age_trigger_swaps_the_retirement_percentiles_for_the_underfunded_probability() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("12"))]).await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 55}),
    )
    .await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["strategy"], "retire_at_age", "{b}");
    assert_eq!(b["retirement_trigger"], "target_age", "{b}");
    assert_eq!(
        b["retirement_month_index_percentiles"],
        Value::Null,
        "con la edad al mando el mes no es una distribución: {b}"
    );
    assert!(
        b["underfunded_probability"].is_string(),
        "el rojo de D17 en versión probabilística debe viajar: {b}"
    );
    // Y la tabla de agotamiento se ancla en la jubilación efectiva, con edades resueltas.
    let table = b["depletion_probability_by_age"].as_array().expect("tabla");
    for row in table {
        assert!(row["month_index"].is_u64(), "{row}");
        assert!(row["age"].is_u64(), "la DOB existe, así que la edad también: {row}");
        assert!(row["probability"].is_string(), "{row}");
    }
}

/// Con una estrategia por CRUCE es al revés: hay percentiles del mes de jubilación y no hay
/// probabilidad de infra-financiación.
#[tokio::test]
async fn a_crossing_trigger_publishes_the_retirement_month_percentiles() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "4000", "1500", &[("Indexado", "200000", Some("12"))]).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["retirement_trigger"], "liquid_crossing", "{b}");
    assert_eq!(
        b["underfunded_probability"],
        Value::Null,
        "sin trigger por edad la pregunta no existe: {b}"
    );
    let p = &b["retirement_month_index_percentiles"];
    assert!(p.is_object(), "{b}");
    // p10 ≤ p50 ≤ p90 (los caminos que no se jubilan ordenan los últimos y salen `null`).
    let ord = |k: &str| p[k].as_u64();
    if let (Some(a), Some(c)) = (ord("p10"), ord("p50")) {
        assert!(a <= c, "p10 ≤ p50: {p}");
    }
    if let (Some(c), Some(d)) = (ord("p50"), ord("p90")) {
        assert!(c <= d, "p50 ≤ p90: {p}");
    }
}

// ---------------------------------------------------------------------------------------------
// 9. Tamaño del payload
// ---------------------------------------------------------------------------------------------

/// **La medida del presupuesto de contexto**, con el número impreso para que quede en el log del
/// CI en vez de en la memoria de nadie.
///
/// El tope no es estético: la respuesta entera viaja a la tool MCP `get_projection_bands`, y ahí
/// compite con el resto de la conversación. 32 KB es holgado para la densidad `hybrid` con las
/// SEIS series (las tres del patrimonio y las tres del líquido) y deja margen para un horizonte
/// de 840 meses con patrimonios de siete cifras; si algún día se rompe, la salida es quitar las
/// bandas del líquido (ya opt-in en la tool), no subir la constante.
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

    // Con los caminos POR DEFECTO: es el payload que sirve la SPA y la tool MCP, y de paso deja
    // medido lo que cuesta un MISS frente a un HIT. Los tiempos se IMPRIMEN, no se afirman: un
    // umbral de reloj en CI enseña a ignorar los fallos (misma doctrina que `timing_mc.rs`).
    let t0 = std::time::Instant::now();
    let r = app
        .get_with_cookie("/v1/projection/bands", &owner.cookie)
        .await;
    let miss_ms = t0.elapsed().as_millis();
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let t1 = std::time::Instant::now();
    let hit = app
        .get_with_cookie("/v1/projection/bands", &owner.cookie)
        .await;
    let hit_ms = t1.elapsed().as_millis();
    assert_eq!(hit.body, r.body, "el HIT debe devolver el mismo cuerpo");

    let bytes = r.body.len();
    let v = r.json();
    let points = v["points"].as_array().expect("puntos").len();
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
         líquido ≈ {sin_liquido} bytes · MISS {miss_ms} ms · HIT {hit_ms} ms · motor \
         {} ms (perfil {})",
        v["paths"],
        v["computed_in_ms"],
        if cfg!(debug_assertions) { "debug" } else { "release" },
    );
    assert!(
        bytes <= 32_000,
        "el payload de bandas a densidad hybrid pesa {bytes} bytes ({points} puntos) y el \
         presupuesto es 32.000 — quita las bandas del líquido antes de subir la cota"
    );
}

// ---------------------------------------------------------------------------------------------
// 10. El colchón de caja (P4)
// ---------------------------------------------------------------------------------------------

/// **El colchón se simula SOLO en Monte Carlo, y solo cuando puede significar algo.**
///
/// Las tres condiciones son acumulativas: `cash_buffer_months` en el perfil, un activo líquido que
/// lo albergue y volatilidad declarada de la que protegerse. Con la última apagada, `buffer_active`
/// es `false` **y no es un fallo**: sin dispersión no hay mes bueno ni malo que distinguir, así que
/// el trasvase no tendría criterio y el resultado sería idéntico al de no pedirlo. Decirlo es lo
/// que impide leer «no pasó nada» como «no funcionó».
#[tokio::test]
async fn the_cash_buffer_is_simulated_only_when_it_can_mean_something() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Dos líquidos: el colchón se instala en el de menor rentabilidad (el primero del orden de
    // drenaje) y se rellena vendiendo del otro.
    seed(
        &app,
        &owner,
        "2500",
        "2000",
        &[("Aaa cuenta", "20000", Some("0")), ("Bbb bolsa", "300000", Some("25"))],
    )
    .await;
    patch_profile(&app, &owner, json!({"cash_buffer_months": 12})).await;
    let iid = app.installation_id().await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=13")).await;
    assert_eq!(b["buffer_active"], true, "colchón + líquido + σ > 0: {b}");
    assert!(
        b["buffer_inactive_reason"].is_null(),
        "`null` ⟺ se simuló: {b}"
    );
    assert!(
        b["buffer_refills_p50"].is_u64(),
        "con el colchón vivo el CONTADOR de rellenos viaja: {b}"
    );
    assert!(
        b["buffer_refill_net_total_p50"].is_string(),
        "y su total, como string decimal: {b}"
    );

    // Quitar la volatilidad: el colchón deja de tener sentido y se declara apagado, con sus dos
    // lecturas en `null` (no en 0).
    let list = app.get_with_cookie("/v1/assets", &owner.cookie).await.json();
    for a in list.as_array().or(list["assets"].as_array()).expect("activos") {
        let r = app
            .patch_json_with_cookie(
                &format!("/v1/assets/{}", a["id"].as_str().expect("id")),
                json!({"annual_volatility_percent": null}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    }
    app.state.invalidate_projection_by_installation(iid).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=13")).await;
    assert_eq!(b["any_volatility_declared"], false, "{b}");
    assert_eq!(
        b["buffer_active"], false,
        "sin volatilidad el colchón no protege de nada: {b}"
    );
    assert_eq!(
        b["buffer_inactive_reason"], "no_volatility",
        "el motivo distingue «no lo pediste» de «lo pediste y no cabía»: {b}"
    );
    assert!(b["buffer_refills_p50"].is_null(), "{b}");
    assert!(b["buffer_refill_net_total_p50"].is_null(), "{b}");
}

/// **El tercer motivo: colchón pedido, volatilidad declarada y NINGÚN sitio seguro donde
/// alojarlo.**
///
/// `cash_buffer_index` sale del orden de drenaje, que no sabe de volatilidad: en una cartera de
/// pura renta variable elegía la propia RV como colchón, y un colchón con σ = 25 % no es un
/// colchón, es la misma cartera con más impuestos. Desde el pase de correcciones de la revisión
/// adversarial, si no hay un líquido con σ = 0 el colchón **no se instala** y se dice por qué.
#[tokio::test]
async fn without_a_risk_free_liquid_asset_the_buffer_says_why_it_did_not_run() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Un único líquido, y VOLÁTIL: hay riesgo de secuencia (σ > 0) pero no hay refugio (σ = 0).
    seed(&app, &owner, "2500", "2000", &[("Bolsa", "300000", Some("25"))]).await;
    patch_profile(&app, &owner, json!({"cash_buffer_months": 12})).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=13")).await;
    assert_eq!(b["any_volatility_declared"], true, "{b}");
    assert_eq!(
        b["buffer_active"], false,
        "no hay ningún activo líquido con σ = 0 donde ponerlo: {b}"
    );
    assert_eq!(
        b["buffer_inactive_reason"], "no_safe_liquid_asset",
        "y el motivo no puede confundirse con `no_volatility`, que aquí sería falso: {b}"
    );
    assert!(b["buffer_refills_p50"].is_null(), "{b}");
    assert!(b["buffer_refill_net_total_p50"].is_null(), "{b}");
}

// ---------------------------------------------------------------------------------------------
// 11. La definición de éxito (pase de correcciones de la revisión adversarial)
// ---------------------------------------------------------------------------------------------

/// **El hogar que no se jubila JAMÁS ya no cuenta como éxito.**
///
/// Es la regresión exacta del hallazgo #7. Con la definición vieja —«la cartera no se agota
/// nunca»— un plan por CRUCE que no llega al objetivo en todo el horizonte nunca drena, y por
/// tanto nunca se agota: se publicaba `success_probability = 1` sobre un plan que no ocurre.
///
/// El hogar de este test ahorra 50 €/mes contra un objetivo de seis cifras: no cruza ni en el
/// último mes del horizonte. σ = 0 hace el resultado BINARIO y por tanto exacto — sin sorteo que
/// interpretar, las tres cifras son `0`, `1` y `null`.
#[tokio::test]
async fn a_plan_that_never_retires_is_not_a_success_anymore() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2000", "1950", &[("Hucha", "1000", Some("0"))]).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["retirement_trigger"], "liquid_crossing", "{b}");
    assert_eq!(
        b["never_retired_probability"], "1",
        "ningún camino llega al objetivo: {b}"
    );
    assert_eq!(
        b["success_probability"], "0",
        "un plan que no ocurre no es un éxito — con la definición vieja esto valía 1: {b}"
    );
    assert_eq!(
        b["success_given_retired"], Value::Null,
        "sin ningún camino jubilado, «¿aguanta?» no tiene sobre qué formularse: {b}"
    );
    assert_eq!(b["success_verdict"], "red", "{b}");
    assert_success_identity(&b);

    // Y la razón por la que la definición vieja lo llamaba éxito sigue siendo verdad: la cartera
    // no se agota. Es exactamente eso lo que dejó de bastar.
    let s = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();
    assert_eq!(
        s["jubilacion_month_index"],
        Value::Null,
        "el camino determinista tampoco se jubila: {s}"
    );
    assert_eq!(
        s["assets_depleted_month_index"],
        Value::Null,
        "y nunca se agota — el 1 de antes salía justo de aquí: {s}"
    );
    // La tabla de agotamiento va VACÍA: sin jubilación no existe «la probabilidad de agotar a
    // los 75». Un array vacío y un cero son cosas distintas.
    assert_eq!(
        b["depletion_probability_by_age"].as_array().map(Vec::len),
        Some(0),
        "{b}"
    );
}

/// El espejo con DISPERSIÓN: con volatilidad alta unos caminos se jubilan y otros no, así que las
/// tres cifras son estrictamente intermedias y la identidad las ata.
#[tokio::test]
async fn with_dispersion_the_three_success_readings_stay_consistent() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Cerca del objetivo y muy volátil: el cruce depende del mercado que toque.
    seed(&app, &owner, "3000", "2000", &[("Indexado", "300000", Some("30"))]).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS_SPREAD}&seed=7")).await;
    assert_eq!(b["retirement_trigger"], "liquid_crossing", "{b}");
    assert_success_identity(&b);

    let never = prob(&b["never_retired_probability"]);
    assert!((0.0..=1.0).contains(&never), "{b}");
    // El condicional existe ⟺ algún camino se jubila, y nunca es menor que el éxito absoluto:
    // el mismo numerador sobre un denominador más pequeño.
    match b["success_given_retired"].as_str() {
        Some(g) => {
            let given: f64 = g.parse().expect("probabilidad");
            assert!(never < 1.0, "hay caminos jubilados: {b}");
            assert!(
                given + 1e-9 >= prob(&b["success_probability"]),
                "el condicional no puede ser menor que el absoluto: {b}"
            );
            assert!((0.0..=1.0).contains(&given), "{b}");
        }
        None => assert_eq!(never, 1.0, "solo es null si NADIE se jubila: {b}"),
    }
}

// =================================================================================================
// §9 · Colchón derivado del tope de la regla de ahorro (5.0.0, decisión V6 del owner)
// =================================================================================================

/// La cartera de la pauta «cuenta hasta X, resto al fondo»: una cuenta corriente **sin
/// volatilidad declarada** (σ = 0 ⇒ puede hacer de colchón) y un fondo volátil (σ ⇒ hay riesgo de
/// secuencia del que protegerse). Devuelve `(cuenta_id, fondo_id)` y deja el sumidero apuntando
/// al fondo — el `create_asset` del primer activo lo sembró sobre la cuenta (#150), y con el
/// sumidero SIN tope sobre la cuenta el colchón no se derivaría nunca (invariante I1).
async fn seed_buffer_portfolio(app: &TestApp, u: &LoggedInOwner) -> (String, String) {
    let ids = seed(
        app,
        u,
        "3000",
        "2000",
        &[
            ("Cuenta corriente", "1000", None),
            ("Fondo indexado global", "200000", Some("20")),
        ],
    )
    .await;
    let sink = app.sink_rule_id(&u.cookie).await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{sink}"),
            json!({ "target_asset_id": ids[1] }),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "retarget del sumidero: {r:?}");
    (ids[0].clone(), ids[1].clone())
}

async fn capped_rule(app: &TestApp, u: &LoggedInOwner, asset: &str, kind: &str, value: &str) -> String {
    let r = app
        .post_json_with_cookie(
            "/v1/allocation-rules",
            json!({ "target_asset_id": asset, "kind": "fixed", "amount": "200",
                    "cap_kind": kind, "cap_value": value }),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "regla con tope: {r:?}");
    r.json()["id"].as_str().expect("rule id").to_string()
}

/// **El colchón sale del tope de la regla, en euros y sin indexar** (V6/P2).
///
/// PREDICCIÓN antes de correr: perfil vacío (sin `cash_buffer_months`), tope `amount = 6000` sobre
/// la cuenta corriente, gasto de jubilación 2.000 €/mes ⇒ `buffer_source: allocation_cap`,
/// `buffer_target_amount: "6000"`, `buffer_months_effective: 3` (= floor(6000/2000), informativo)
/// y el colchón ACTIVO, porque hay volatilidad declarada en el fondo y la cuenta es un líquido
/// σ = 0 donde alojarlo.
#[tokio::test]
async fn the_buffer_is_derived_from_the_rule_cap() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cuenta, _fondo) = seed_buffer_portfolio(&app, &owner).await;
    let rule_id = capped_rule(&app, &owner, &cuenta, "amount", "6000").await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_source"], "allocation_cap", "{b}");
    assert_eq!(b["buffer_target_amount"], "6000.0000", "{b}");
    assert_eq!(b["buffer_months_effective"], 3, "6000/2000 = 3: {b}");
    assert_eq!(b["buffer_source_rule_id"], rule_id, "{b}");
    assert_eq!(b["buffer_source_asset_name"], "Cuenta corriente", "{b}");
    assert_eq!(b["buffer_active"], true, "{b}");
    assert_eq!(b["buffer_inactive_reason"], Value::Null, "{b}");
    // Y el motor lo ejerció: sin actividad, «derivado» sería una etiqueta sin consecuencia.
    assert!(!b["buffer_refills_p50"].is_null(), "{b}");
}

/// **Explícito gana, y `PATCH null` vuelve a derivado.** El tri-estado del PATCH es el camino de
/// vuelta que la SPA deja de escribir pero el API y el MCP conservan.
#[tokio::test]
async fn an_explicit_buffer_wins_and_patch_null_returns_to_the_derived_one() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cuenta, _fondo) = seed_buffer_portfolio(&app, &owner).await;
    let rule_id = capped_rule(&app, &owner, &cuenta, "amount", "6000").await;

    patch_profile(&app, &owner, json!({"cash_buffer_months": 9})).await;
    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_source"], "explicit", "{b}");
    assert_eq!(b["buffer_months_effective"], 9, "{b}");
    assert_eq!(
        b["buffer_target_amount"],
        Value::Null,
        "un colchón en meses se re-dimensiona cada mes: no hay escalar honesto que publicar: {b}"
    );
    assert_eq!(b["buffer_source_rule_id"], Value::Null, "{b}");
    // El activo SÍ se dice: dónde se aloja el colchón es la mitad de entenderlo.
    assert_eq!(b["buffer_source_asset_name"], "Cuenta corriente", "{b}");
    assert_eq!(b["buffer_active"], true, "{b}");

    patch_profile(&app, &owner, json!({"cash_buffer_months": null})).await;
    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_source"], "allocation_cap", "{b}");
    assert_eq!(b["buffer_target_amount"], "6000.0000", "{b}");
    assert_eq!(b["buffer_source_rule_id"], rule_id, "{b}");
}

/// **El caso común de hoy**: el líquido σ = 0 es el sumidero SIN tope (invariante I1) y no hay
/// importe que perseguir. `no_capped_rule` no es un error — es «pon un tope a tu cuenta», y el
/// copy de la SPA lo dice así.
#[tokio::test]
async fn without_a_capped_rule_the_buffer_says_so() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Sin retargetear el sumidero: la cuenta corriente ES el sumidero, y no tiene tope.
    seed(
        &app,
        &owner,
        "3000",
        "2000",
        &[
            ("Cuenta corriente", "1000", None),
            ("Fondo indexado global", "200000", Some("20")),
        ],
    )
    .await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_source"], "none", "{b}");
    assert_eq!(b["buffer_active"], false, "{b}");
    assert_eq!(b["buffer_inactive_reason"], "no_capped_rule", "{b}");
    assert_eq!(b["buffer_source_asset_name"], "Cuenta corriente", "{b}");
    assert_eq!(b["buffer_target_amount"], Value::Null, "{b}");
    assert_eq!(b["buffer_months_effective"], Value::Null, "{b}");
}

/// Un techo de 0 € no es un colchón, y ese motivo es distinto de «no hay regla»: uno se arregla
/// poniendo un tope, el otro subiéndolo.
#[tokio::test]
async fn a_zero_cap_is_not_a_buffer() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cuenta, _fondo) = seed_buffer_portfolio(&app, &owner).await;
    capped_rule(&app, &owner, &cuenta, "amount", "0").await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_source"], "none", "{b}");
    assert_eq!(b["buffer_inactive_reason"], "cap_is_zero", "{b}");
    assert_eq!(b["buffer_source_asset_name"], "Cuenta corriente", "{b}");
}

/// **Sin un líquido σ = 0 no hay dónde alojarlo** — el mismo literal que el motor emite, porque
/// es el mismo hecho: el handler solo llega antes. Aquí los dos activos declaran volatilidad.
#[tokio::test]
async fn without_a_risk_free_liquid_asset_the_buffer_says_so() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ids = seed(
        &app,
        &owner,
        "3000",
        "2000",
        &[
            ("Monetario", "1000", Some("2")),
            ("Fondo indexado global", "200000", Some("20")),
        ],
    )
    .await;
    let sink = app.sink_rule_id(&owner.cookie).await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{sink}"),
            json!({ "target_asset_id": ids[1] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    capped_rule(&app, &owner, &ids[0], "amount", "6000").await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_source"], "none", "{b}");
    assert_eq!(b["buffer_inactive_reason"], "no_safe_liquid_asset", "{b}");
    assert_eq!(
        b["buffer_source_asset_name"],
        Value::Null,
        "no hay activo que nombrar: ninguno es líquido y sin riesgo a la vez: {b}"
    );
}

/// `no_volatility` es del MOTOR y pasa tal cual: hay colchón derivado (tope y activo), pero sin
/// riesgo de secuencia del que protegerse el resultado es bit a bit el de no pedirlo. El campo es
/// UNO solo, y aquí gana la capa que de verdad impidió la simulación.
#[tokio::test]
async fn the_engine_reason_passes_through_when_nothing_is_volatile() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let ids = seed(
        &app,
        &owner,
        "3000",
        "2000",
        &[("Cuenta corriente", "1000", None), ("Fondo indexado global", "200000", None)],
    )
    .await;
    let sink = app.sink_rule_id(&owner.cookie).await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{sink}"),
            json!({ "target_asset_id": ids[1] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let rule_id = capped_rule(&app, &owner, &ids[0], "amount", "6000").await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=3")).await;
    assert_eq!(b["buffer_active"], false, "{b}");
    assert_eq!(b["buffer_inactive_reason"], "no_volatility", "{b}");
    // La derivación SÍ ocurrió: el colchón existe, lo que falta es la volatilidad.
    assert_eq!(b["buffer_source"], "allocation_cap", "{b}");
    assert_eq!(b["buffer_target_amount"], "6000.0000", "{b}");
    assert_eq!(b["buffer_source_rule_id"], rule_id, "{b}");
    assert_eq!(b["any_volatility_declared"], false, "{b}");
}
