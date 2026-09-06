//! **`GET /v1/projection/bands`** — la superficie HTTP de Monte Carlo (5.0.0, modelo v2 de
//! jubilación, WP A6).
//!
//! Lo que estos tests compran, en orden de importancia:
//!
//! 1. **σ = 0 ⇒ la banda ES la línea determinista.** Es el único gate que ata el camino `f64` al
//!    camino `Decimal` que la app publica como dinero: si el ensamblado del endpoint tomara otro
//!    input —otro perfil, otro horizonte, otro scope, **otro mes de jubilación**—, la banda
//!    seguiría saliendo bonita y el error sería invisible. Aquí se compara punto a punto contra
//!    `/v1/projection/series`.
//! 2. **El sorteo describe EL PLAN, no otro.** Desde la v2 el escenario lleva el mes de
//!    jubilación que el solver fijó, así que el éxito de estas bandas y el `success_of_plan` de
//!    la serie tienen que ser la MISMA cifra con el sorteo por defecto
//!    (`the_bands_success_equals_the_plan_success_for_the_default_draw`).
//! 3. **El veredicto se mide contra el umbral DEL PERFIL y contra su INTERVALO**, no contra un
//!    corte fijo: el mismo sorteo cambia de color con el umbral, y con pocos caminos el intervalo
//!    no llega aunque el estimador puntual sí.
//! 4. **El vector de volatilidades sigue el orden de los activos.** Un vector descolocado produce
//!    bandas ESTRECHAS Y CREÍBLES, que es el peor fallo posible en esta superficie.
//! 5. **Reproducibilidad**: misma semilla ⇒ mismo cuerpo byte a byte; otra semilla ⇒ otro mercado.
//! 6. **El hogar no tiene bandas** (400 declarado) y el cache se invalida con las mismas
//!    mutaciones que la serie — con el **umbral** dentro de la clave.

mod common;

use common::{LoggedInOwner, TestApp};
use futurefin_api::state::BandsCacheKey;
use serde_json::{json, Value};
use uuid::Uuid;

/// Caminos de los tests. **Deliberadamente pocos**: lo que se comprueba aquí es el ensamblado, la
/// rejilla y el contrato, no la convergencia estadística — y en `debug` cada camino cuesta un
/// orden de magnitud más que en release (0,2 ms/camino medidos en release, §doc del módulo).
const PATHS: u32 = 24;

/// Los tests que miran la DISPERSIÓN o que esperan un VERDE suben a este. No es cosmético:
/// **con el umbral por defecto (95) el verde es inalcanzable por debajo de 73 caminos**, porque
/// con cero fallos la cota de Wilson topa en `n/(n + 1,96²)` y con 24 caminos eso es 0,862. 120
/// da 0,969 y sigue siendo barato.
const PATHS_SPREAD: u32 = 120;

/// El umbral por defecto del perfil (`DEFAULT_SUCCESS_THRESHOLD_PCT`). Se escribe aquí porque
/// entra en la CLAVE del cache y en el eco de la respuesta, y un test que lo dé por supuesto sin
/// nombrarlo se rompe de forma incomprensible el día que el default cambie.
const DEFAULT_THRESHOLD_PCT: u32 = 95;

async fn bands(app: &TestApp, cookie: &str, q: &str) -> Value {
    let r = app
        .get_with_cookie(&format!("/v1/projection/bands{q}"), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET bands{q}: {r:?}");
    r.json()
}

/// La serie, **recomputada a propósito**.
///
/// La invalidación previa no es higiene decorativa: desde el modelo v2 un MISS de proyección
/// resuelve el plan (decenas de segundos en `debug`), y el warm-up que el login lanza en
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

/// La clave del cache de bandas. **Lleva el umbral desde 5.0.0**: el veredicto se mide contra él,
/// así que dos umbrales describen dos respuestas distintas del mismo sorteo.
fn key(iid: Uuid, user_id: Uuid, paths: u32, seed: &str, threshold_pct: u32) -> BandsCacheKey {
    BandsCacheKey {
        installation_id: iid,
        user_id,
        paths,
        seed: seed.parse().expect("semilla decimal"),
        threshold_pct,
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

/// Los tres contadores de `failures_by_kind`, en su orden fijo `[F1, F2, F3]`.
fn failures_by_kind(b: &Value) -> [u64; 3] {
    let a = b["failures_by_kind"].as_array().expect("failures_by_kind");
    assert_eq!(a.len(), 3, "el reparto tiene TRES motivos y solo tres: {b}");
    [
        a[0].as_u64().expect("F1"),
        a[1].as_u64().expect("F2"),
        a[2].as_u64().expect("F3"),
    ]
}

/// **Las tres lecturas del éxito tienen que ser coherentes entre sí**, siempre y en todos los
/// tests: el intervalo por debajo del punto, la barra igual a su distancia (con el redondeo de
/// publicación) y el reparto por motivo sumando los fallos que la probabilidad implica.
fn assert_success_block_is_consistent(b: &Value) {
    let success = prob(&b["success_of_plan"]);
    let low = prob(&b["success_wilson_low"]);
    let paths = b["paths"].as_u64().expect("paths") as f64;
    assert!(
        (0.0..=1.0).contains(&success) && (0.0..=1.0).contains(&low),
        "las dos cifras son fracciones: {b}"
    );
    assert!(
        low <= success + 1e-9,
        "la cota inferior de Wilson no puede estar por encima del estimador puntual: {b}"
    );
    // La barra es la distancia entre las dos, en puntos porcentuales y con UN decimal.
    let bar: f64 = b["success_sampling_error_pp"]
        .as_str()
        .expect("la barra viaja como string decimal")
        .parse()
        .expect("barra parseable");
    // **La barra es cero en un solo caso, y no es «medición sin error»**: con TODOS los caminos
    // fallidos (`p̂ = 0`) la cota inferior de Wilson vale 0 exacto —una probabilidad no baja de
    // cero—, así que la barra HACIA ABAJO no tiene dónde ir. Con cualquier éxito por encima de 0,
    // incluido el 1 perfecto, es estrictamente positiva: ese es el sentido entero de Wilson
    // frente a la aproximación normal.
    if success > 0.0 {
        assert!(
            bar > 0.0,
            "con éxito > 0 la barra de Wilson NUNCA es cero, tampoco con CERO fallos: {b}"
        );
    } else {
        assert_eq!(
            bar, 0.0,
            "con p̂ = 0 la cota inferior es 0 exacto y la barra hacia abajo también: {b}"
        );
    }
    assert!(
        (bar - (success - low) * 100.0).abs() <= 0.06,
        "la barra ({bar} pp) debe ser la distancia punto→cota ({} pp) salvo el redondeo a un \
         decimal: {b}",
        (success - low) * 100.0
    );
    // El reparto por motivo suma exactamente los fallos que implica la probabilidad.
    let k = failures_by_kind(b);
    let total = (k[0] + k[1] + k[2]) as f64;
    assert!(
        (total - (1.0 - success) * paths).abs() <= 0.5,
        "failures_by_kind suma {total} y la probabilidad implica {}: {b}",
        (1.0 - success) * paths
    );
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
    ] {
        assert!(
            b.get(dead).is_none(),
            "`{dead}` se retiró en el modelo v2 y no puede volver: {b}"
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
    assert_eq!(b["strategy"], "asap", "{b}");
    // El umbral del perfil VUELVE a la respuesta en v2: es la restricción que decidió la fecha y
    // el listón del veredicto, así que sin él el color no se puede auditar.
    assert_eq!(b["success_threshold_pct"], DEFAULT_THRESHOLD_PCT, "{b}");
    assert_eq!(b["any_volatility_declared"], true, "{b}");
    // Las cuatro cifras del éxito viajan JUNTAS: el punto, su intervalo, su barra y su N.
    for k in [
        "success_of_plan",
        "success_wilson_low",
        "success_sampling_error_pp",
    ] {
        assert!(b[k].is_string(), "`{k}` viaja como string decimal: {b}");
    }
    assert!(
        ["green", "amber", "red"].contains(&b["success_verdict"].as_str().expect("veredicto")),
        "{b}"
    );
    assert_success_block_is_consistent(&b);
    assert_the_v1_fields_are_gone(&b);
    assert!(
        b["model_note"]
            .as_str()
            .expect("nota")
            .contains("SOLO PUEDE FALLAR ESTANDO JUBILADO"),
        "la nota debe declarar el límite de la definición de éxito: {b}"
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
// 3. El gate: σ = 0 ⇒ la banda ES la línea, y es la línea DEL PLAN
// ---------------------------------------------------------------------------------------------

/// **Sin volatilidad declarada, los tres percentiles coinciden con la serie determinista.**
///
/// Es el único punto donde el camino `f64` se mide contra el `Decimal` que la app publica como
/// dinero, y desde la v2 comprueba una cosa más: que las dos superficies simulan **el mismo
/// plan**. Si las bandas sortearan la entrada en crudo (jubilación por cruce) y la serie el
/// escenario con el mes forzado, las curvas divergirían justo a partir de la jubilación — que es
/// donde nadie mira, porque el principio coincide.
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

    // Con σ = 0 todos los caminos son EL camino: el éxito solo puede ser 1 o 0, sin fracciones.
    let success = prob(&b["success_of_plan"]);
    assert!(
        success == 1.0 || success == 0.0,
        "sin dispersión el éxito es binario: {b}"
    );
    assert_success_block_is_consistent(&b);
}

/// El espejo del anterior: un plan que **sí** se rompe da éxito `0` exacto, veredicto rojo, y
/// **dice por qué motivo se rompió**.
///
/// **Se fuerza la fecha con `retire_at_age`** y no se deja a `asap`, y la razón es el hallazgo que
/// documenta el módulo: el motor solo clasifica fallos estando JUBILADO, así que un plan que no
/// llega a jubilarse nunca falla. Con `asap` este hogar no alcanzaría ninguna fecha válida y el
/// sorteo mediría «un plan sin jubilación», que es otra pregunta.
///
/// **Y el motivo es F2, no F1**, que es justo lo que el reparto por motivo existe para poder
/// decir. Jubilarse a los 45 con una hucha de 3.000 € y un gasto de 2.500 €/mes exige vender el
/// **1.000 % anual** de la cartera; la puerta de tasa inicial lo tumba en el PRIMER mes jubilado,
/// antes de que a la cartera le dé tiempo a agotarse. El orden de prioridad del motor es
/// `F1 > F2 > F3` y F1 exige una venta que se quedó corta — en el primer mes la hucha todavía
/// paga. Con un solo «probabilidad de ruina» los dos casos se veían iguales; con `failures_by_kind`
/// tienen arreglos distintos (F1 pide más capital, F2 pide retrasar la fecha).
#[tokio::test]
async fn a_plan_that_breaks_deterministically_scores_zero_and_says_which_gate_broke() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "2600"), (&exp, "2500")] {
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
            json!({"category_id": ast, "name": "Hucha", "current_value": "3000",
                   "is_liquid": true, "expected_annual_return_percent": "0"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 45}),
    )
    .await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(b["strategy"], "retire_at_age", "{b}");
    assert_eq!(b["success_of_plan"], "0", "{b}");
    assert_eq!(b["success_verdict"], "red", "{b}");
    // El motivo es la PUERTA DE TASA INICIAL (F2), y el reparto lo dice sin ambigüedad.
    let k = failures_by_kind(&b);
    assert_eq!(
        k[1], u64::from(PATHS),
        "los {PATHS} caminos fallan por tasa inicial excedida: {b}"
    );
    assert_eq!(
        [k[0], k[2]], [0, 0],
        "y por nada más: F1 exige una venta que se quede corta y F3 una regla con techo: {b}"
    );
    assert_success_block_is_consistent(&b);
    // La curva de fallo lo fecha: el 100 % ya en la primera fila, la de la jubilación.
    let curve = b["failure_probability_by_age"].as_array().expect("curva");
    assert_eq!(
        curve.first().map(|r| &r["probability"]),
        Some(&json!("1")),
        "el plan se rompe EN la jubilación, no más tarde: {b}"
    );
}

/// **El plan sin fecha alcanzable, pinchado a propósito.**
///
/// Un hogar que ahorra 50 €/mes no alcanza ninguna fecha que cumpla el umbral, así que el
/// escenario que se sortea es el que la serie publica: **no jubilarse dentro del horizonte**. Y
/// como el motor solo clasifica fallos estando jubilado, ese plan **no puede fallar**: el éxito
/// sale 1 y el veredicto verde.
///
/// **Ese verde no dice «llegas», dice «este plan sin jubilación no se rompe».** El test existe
/// para que la trampa esté escrita y medida en vez de descubrirse en producción: quien pinte el
/// semáforo mira antes `retirement_date_basis` en la serie, y la `model_note` lo dice para quien
/// solo ve el JSON.
#[tokio::test]
async fn a_plan_with_no_reachable_date_draws_the_line_that_never_retires() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "2000", "1950", &[("Hucha", "1000", Some("0"))]).await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS_SPREAD}")).await;
    assert_eq!(
        b["success_of_plan"], "1",
        "sin jubilación no hay nada que pueda fallar: {b}"
    );
    assert_eq!(failures_by_kind(&b), [0, 0, 0], "{b}");
    // La tabla trae UNA fila —la del horizonte— y vale 0. No está vacía y no dice «seguro».
    let curve = b["failure_probability_by_age"].as_array().expect("curva");
    assert_eq!(
        curve.len(),
        1,
        "la rejilla arranca en el mes forzado (horizonte + 1): solo cabe el cierre: {b}"
    );
    assert_eq!(curve[0]["probability"], "0", "{curve:?}");
    assert_eq!(
        curve[0]["month_index"].as_u64(),
        b["months"].as_u64().map(|m| m - 1),
        "esa única fila es la del horizonte, y el horizonte del BUCLE es `months − 1` en la \
         rejilla publicada (`engine_month_to_grid`): {b}"
    );
    // Y la nota lo declara, que es lo único que ve un consumidor conversacional.
    assert!(
        b["model_note"]
            .as_str()
            .expect("nota")
            .contains("retirement_date_basis"),
        "la nota tiene que mandar al lector a la serie: {b}"
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
    let k = key(iid, owner.user_id, PATHS, "11", DEFAULT_THRESHOLD_PCT);
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

/// **El umbral está en la CLAVE, no solo en la respuesta.**
///
/// El fallo que esto cierra es el que describe `state.rs`: cambiar el umbral en Ajustes y recibir
/// el veredicto del umbral anterior —verde donde tocaba ámbar— sin que ningún campo lo dijera.
///
/// La prueba fuerte es la última: después de cambiar el umbral se **reinyecta a mano** la entrada
/// vieja (la del umbral anterior) y el siguiente GET **no** puede servirla. Si el umbral no
/// estuviera en la clave, ese centinela saldría por la respuesta.
#[tokio::test]
async fn the_bands_cache_key_carries_the_threshold() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("12"))]).await;
    let iid = app.installation_id().await;

    let q = format!("?paths={PATHS}&seed=11");
    let b = bands(&app, &owner.cookie, &q).await;
    assert_eq!(b["success_threshold_pct"], DEFAULT_THRESHOLD_PCT, "{b}");

    let k95 = key(iid, owner.user_id, PATHS, "11", DEFAULT_THRESHOLD_PCT);
    let k80 = key(iid, owner.user_id, PATHS, "11", 80);
    assert_ne!(k95, k80, "dos umbrales son dos claves");
    assert!(app.state.bands_cache.read().await.contains_key(&k95));

    const SENTINEL: &str = "SENTINEL-umbral-viejo";
    let poisoned = {
        let cache = app.state.bands_cache.read().await;
        let mut resp = (*cache.get(&k95).expect("entrada del 95").response).clone();
        resp.model_note = SENTINEL.to_string();
        resp
    };

    patch_profile(&app, &owner, json!({"success_threshold_pct": 80})).await;
    // El PATCH invalida las bandas enteras (sale del mismo `ProjectionInput`).
    assert!(
        !app.state.bands_cache.read().await.contains_key(&k95),
        "un PATCH del perfil invalida las bandas"
    );
    // Se REINYECTA la entrada del umbral viejo: si la clave no llevara el umbral, el GET de
    // abajo la serviría.
    app.state
        .bands_cache_insert(k95.clone(), std::sync::Arc::new(poisoned))
        .await;

    let after = bands(&app, &owner.cookie, &q).await;
    assert_ne!(
        after["model_note"], SENTINEL,
        "la respuesta del umbral 95 no puede servirse a quien ahora tiene 80: {after}"
    );
    assert_eq!(after["success_threshold_pct"], 80, "{after}");
    assert!(
        app.state.bands_cache.read().await.contains_key(&k80),
        "el GET con el umbral nuevo deja su PROPIA entrada"
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
            .contains_key(&key(iid, owner.user_id, PATHS, "5", DEFAULT_THRESHOLD_PCT)),
        "la entrada debe existir antes del logout"
    );
    app.state.invalidate_projection_by_user(owner.user_id).await;
    assert!(
        app.state.bands_cache.read().await.is_empty(),
        "el logout debe llevarse las bandas del usuario"
    );
}

// ---------------------------------------------------------------------------------------------
// 7. Veredicto: el umbral del perfil Y su intervalo
// ---------------------------------------------------------------------------------------------

/// **El veredicto se mide contra el umbral DEL PERFIL y contra el INTERVALO, no contra un corte
/// fijo** (5.0.0, modelo v2, C3). Sustituye al corte fijo al 100 % de la primera vuelta.
///
/// Las tres regiones sobre **la misma muestra**, que es lo que las hace comparables:
///
/// | caminos | umbral | cota de Wilson (0 fallos) | color | por qué |
/// |---|---|---|---|---|
/// | 120 | 95 | 0,9690 | verde | el intervalo llega |
/// | 24 | 95 | 0,8621 | **ámbar** | el puntual (1) llega, el intervalo no |
/// | 24 | 80 | 0,8621 | verde | con menos exigencia, el intervalo sí llega |
/// | 120 | 100 | — | verde | cero fallos, que es la regla del 100 % |
///
/// El ámbar es la región que un corte fijo no sabía nombrar: «puede que sí, pero esta muestra no
/// lo demuestra». Y que 24 caminos no puedan ser verdes al 95 % **no es un bug**: con cero fallos
/// la cota topa en `n/(n + 1,96²)`, así que el 95 % exige al menos 73 caminos. El plan no se
/// resiente —se resuelve siempre con 500/2.500—, solo el veredicto de ESTE sorteo.
#[tokio::test]
async fn the_verdict_follows_the_profile_threshold_and_its_interval() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // σ = 0: el éxito es exactamente 1, así que lo único que mueve el color es el umbral y el N.
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("0"))]).await;

    let wide = bands(&app, &owner.cookie, &format!("?paths={PATHS_SPREAD}")).await;
    assert_eq!(wide["success_of_plan"], "1", "sin dispersión, 1 exacto: {wide}");
    assert_eq!(
        wide["success_verdict"], "green",
        "{PATHS_SPREAD} caminos limpios superan el 95 %: {wide}"
    );

    let narrow = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(narrow["success_of_plan"], "1", "{narrow}");
    assert_eq!(
        narrow["success_verdict"], "amber",
        "con {PATHS} caminos el puntual llega y el INTERVALO no — esa franja es el ámbar: {narrow}"
    );

    // Mismo sorteo, otro umbral: el color cambia sin que cambie ni un camino.
    patch_profile(&app, &owner, json!({"success_threshold_pct": 80})).await;
    let relaxed = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    assert_eq!(relaxed["success_threshold_pct"], 80, "{relaxed}");
    assert_eq!(relaxed["success_of_plan"], "1", "{relaxed}");
    assert_eq!(
        relaxed["success_verdict"], "green",
        "la misma muestra, con menos exigencia, sí cumple: {relaxed}"
    );

    // Y el 100 %: la regla deja de mirar el intervalo y cuenta FALLOS. Cero fallos ⇒ verde.
    patch_profile(&app, &owner, json!({"success_threshold_pct": 100})).await;
    let strict = bands(&app, &owner.cookie, &format!("?paths={PATHS_SPREAD}")).await;
    assert_eq!(strict["success_threshold_pct"], 100, "{strict}");
    assert_eq!(
        strict["success_verdict"], "green",
        "el 100 % es «cero fallos de N», y no hay ninguno: {strict}"
    );
}

/// **El umbral, los caminos y la barra de error se ECOAN**, y la barra encoge con la muestra.
///
/// Es la trinidad que hace auditable una probabilidad: sin el umbral el color no se puede
/// comprobar, sin `paths` la cifra no se puede comparar con otra, y sin la barra un 94 % de 24
/// caminos parece lo mismo que un 94 % de 2.500.
#[tokio::test]
async fn the_bands_echo_the_threshold_the_paths_and_the_sampling_error() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner, "3000", "2000", &[("Indexado", "20000", Some("12"))]).await;
    patch_profile(&app, &owner, json!({"success_threshold_pct": 90})).await;

    let few = bands(&app, &owner.cookie, &format!("?paths={PATHS}&seed=7")).await;
    let many = bands(&app, &owner.cookie, &format!("?paths={PATHS_SPREAD}&seed=7")).await;

    for (b, paths) in [(&few, PATHS), (&many, PATHS_SPREAD)] {
        assert_eq!(b["success_threshold_pct"], 90, "el umbral del perfil: {b}");
        assert_eq!(b["paths"], paths, "los caminos efectivos: {b}");
        assert_success_block_is_consistent(b);
    }

    let bar = |b: &Value| -> f64 {
        b["success_sampling_error_pp"]
            .as_str()
            .expect("string")
            .parse()
            .expect("barra")
    };
    assert!(
        bar(&few) > bar(&many),
        "más caminos, menos incertidumbre: {} pp con {PATHS} frente a {} pp con {PATHS_SPREAD}",
        bar(&few),
        bar(&many)
    );
}

// ---------------------------------------------------------------------------------------------
// 8. La curva de fallo cuenta MÁS que el agotamiento
// ---------------------------------------------------------------------------------------------

/// **`failure_probability_by_age` sustituye a `depletion_probability_by_age` porque el
/// agotamiento ya no es el único motivo de fallo.**
///
/// El hogar: se jubila a los 45 por EDAD (así hay fecha pase lo que pase), con **800.000 €**
/// líquidos y una regla `percent_of_balance` al **1 % anual** contra un gasto de 2.000 €/mes.
///
/// Las tres cifras están elegidas para aislar F3, y conviene decir cómo, porque el motor prioriza
/// `F1 > F2 > F3`:
///
/// - **F2 no puede saltar**: la puerta de tasa inicial compara el SWR del perfil (3,5 %) contra
///   la necesidad anual — 3,5 % de 800.000 € son 28.000 €/año y hacen falta 24.000. Con una
///   cartera pequeña saltaría F2 y F3 no llegaría a evaluarse nunca (y eso es exactamente lo que
///   hacía fallar a la primera versión de este test, con 300.000 €).
/// - **F1 tampoco**: retirar el 1 % de un capital que crece al 5 % no vacía nada.
/// - **F3 sí**: la regla permite ~667 €/mes donde hacen falta 2.000, en el primer mes jubilado.
///
/// Con la tabla vieja este plan publicaba «0 % de probabilidad de agotar» en todas las filas —
/// porque, literalmente, no se agota. Con la nueva publica el 100 % de fallo, y
/// `failures_by_kind` dice por qué.
#[tokio::test]
async fn the_failure_curve_counts_more_than_depletion() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // σ = 0: el resultado es binario y exacto, sin un sorteo que interpretar.
    seed(&app, &owner, "3000", "2000", &[("Indexado", "800000", Some("0"))]).await;
    patch_profile(
        &app,
        &owner,
        json!({
            "strategy": "retire_at_age",
            "target_retirement_age": 45,
            "withdrawal_rule": {"kind": "percent_of_balance", "pct": "1", "spend_mode": "ceiling"},
        }),
    )
    .await;

    let b = bands(&app, &owner.cookie, &format!("?paths={PATHS}")).await;
    let k = failures_by_kind(&b);
    assert_eq!(
        k[0], 0,
        "la cartera NO se agota: retirar el 1 % de un capital que crece al 5 % no la vacía: {b}"
    );
    assert_eq!(
        k[1], 0,
        "y la tasa inicial NO se excede: el SWR del perfil cubre la necesidad de sobra — si esto \
         saltara, F3 no llegaría a evaluarse y el test mediría otra cosa: {b}"
    );
    assert_eq!(
        k[2], u64::from(PATHS),
        "y sin embargo TODOS los caminos fallan, por F3 — la regla no llega a la necesidad: {b}"
    );
    assert_eq!(b["success_of_plan"], "0", "{b}");
    assert_success_block_is_consistent(&b);

    // La curva lo dice a lo largo del tiempo, con el reparto por motivo al lado de cada fila.
    let curve = b["failure_probability_by_age"].as_array().expect("curva");
    assert!(!curve.is_empty(), "con fecha de jubilación hay tabla: {b}");
    let mut prev = -1.0f64;
    for row in curve {
        assert!(row["month_index"].is_u64(), "{row}");
        assert!(
            row["age"].is_u64(),
            "el arnés registra con fecha de nacimiento, así que la edad existe: {row}"
        );
        let p = prob(&row["probability"]);
        assert!(p >= prev - 1e-9, "la acumulada no puede bajar: {curve:?}");
        prev = p;
        assert_eq!(
            row["by_kind"],
            b["failures_by_kind"],
            "el reparto por fila es el de la EJECUCIÓN entera, declarado como tal: {row}"
        );
    }
    // La última fila es el horizonte y cierra en 1 − éxito.
    let last = curve.last().expect("última fila");
    assert_eq!(
        last["month_index"].as_u64(),
        b["months"].as_u64().map(|m| m - 1),
        "la tabla cierra SIEMPRE en el horizonte — que en la rejilla publicada es `months − 1`, \
         un punto ANTES del último de `points[]`: {b}"
    );
    assert!(
        (prob(&last["probability"]) - (1.0 - prob(&b["success_of_plan"]))).abs() <= 1e-6,
        "la última fila y el éxito cuentan el mismo conjunto de caminos: {b}"
    );

    // Y el contraste que da nombre al test: la serie NO marca agotamiento.
    let s = series(&app, &owner.cookie).await;
    assert_eq!(
        s["assets_depleted_month_index"],
        Value::Null,
        "la tabla vieja habría publicado 0 % en todas las filas de este plan: {s}"
    );
}

// ---------------------------------------------------------------------------------------------
// 9. La identidad con el plan de la serie
// ---------------------------------------------------------------------------------------------

/// **El éxito que dibuja el fan chart y el que decide la fecha son la MISMA cifra.**
///
/// Es la propiedad que justifica que `DEFAULT_BANDS_PATHS` sea, literalmente,
/// `SOLVE_CONFIRM_PATHS`: con el sorteo por defecto —2.500 caminos y la semilla estable del
/// usuario— las bandas re-ejecutan la MISMA muestra que confirmó la fecha sobre el MISMO
/// escenario, así que las dos pantallas no pueden publicar dos probabilidades del mismo plan.
///
/// Se compara a 6 decimales porque las dos salen del mismo redondeo de publicación. Y se exige
/// primero que el plan TENGA fecha: sin ella la serie publica la mejor observación de la búsqueda
/// (500 caminos) y las bandas sortean el escenario sin jubilación — dos preguntas distintas, y la
/// identidad no aplica (ver el doc del módulo del handler).
///
/// **Es el test más caro de la suite**: usa los caminos por defecto a propósito, porque la
/// identidad solo se promete ahí.
#[tokio::test]
async fn the_bands_success_equals_the_plan_success_for_the_default_draw() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // Un hogar con margen: hace falta que el plan alcance una fecha válida.
    seed(&app, &owner, "4000", "1500", &[("Indexado", "400000", Some("12"))]).await;

    let s = series(&app, &owner.cookie).await;
    assert_ne!(
        s["retirement_date_basis"], "not_reachable",
        "sin fecha la identidad no aplica; este hogar tiene que alcanzarla: {s}"
    );
    let plan_success = s["success_of_plan"]
        .as_str()
        .unwrap_or_else(|| panic!("la serie debe publicar `success_of_plan`: {s}"));

    // Sin `?paths=` ni `?seed=`: el sorteo por defecto, que es el único con identidad prometida.
    let b = bands(&app, &owner.cookie, "").await;
    assert_eq!(
        b["success_of_plan"].as_str(),
        Some(plan_success),
        "el éxito del fan chart y el del plan salen de la MISMA muestra: bandas {b}, serie {s}"
    );
    assert_eq!(
        b["success_threshold_pct"], s["success_threshold_pct"],
        "y contra el mismo umbral"
    );
    assert_eq!(
        b["paths"], 2500,
        "el default de la superficie es el presupuesto de confirmación del plan: {b}"
    );
}

// ---------------------------------------------------------------------------------------------
// 10. Tamaño del payload
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
/// cambiaría ni un byte de `points[]` y cuesta cien veces más. Lo que sí creció en 5.0.0 es la
/// `model_note` y el `by_kind` de cada fila de la curva de fallo; los dos se imprimen aparte para
/// que el margen esté medido y no supuesto.
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

