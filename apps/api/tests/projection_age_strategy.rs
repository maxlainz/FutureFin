//! **Estrategias por EDAD** (`retire_at_age` / `coast`): la edad manda, el cruce pasa a lectura
//! (5.0.0, D17 / §C del plan de #207).
//!
//! Hasta 4.15.x la jubilación era UN evento: el mes en que el líquido alcanzaba el objetivo. Con
//! `retire_at_age` el trigger es la edad y el capital deja de decidir — «me jubilo a los 55 pase
//! lo que pase». Eso obliga a separar dos cifras que hasta ahora eran la misma:
//!
//! * `jubilacion_month_index` / `retirement_month_index` — cuándo te jubilas DE VERDAD.
//! * `liquid_crossing_month_index` — cuándo el capital habría bastado. Una LECTURA: puede caer
//!   después (te vas sin llegar) o no caer nunca dentro del horizonte.
//!
//! **5.0.0 llevó esa separación hasta el final**: el cruce contra un objetivo determinista dejó de
//! existir como salida (`liquid_crossing_month_index` y `fire_target_series` se retiraron) porque
//! ya no decide nada — el motor recibe `fire_target: None`. Lo que queda es una sola pregunta con
//! dos respuestas posibles, y `retirement_date_basis` dice cuál:
//!
//! * `target_age` — la fecha es un DATO del usuario, y lo que el sorteo mide es **si se llega**
//!   (`success_of_plan`) y **cuánto falta aportar** (`contribution_required_monthly`).
//! * `success_threshold` — la fecha la decide el umbral.
//!
//! Y de ahí el pin central de este fichero: **una estrategia por edad NO bisecciona la fecha, la
//! CONFIRMA**.

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

/// Hogar mínimo con objetivo FIRE alcanzable pero LEJOS: ingreso 2.400, gasto 1.800 y un activo
/// pequeño. Así el cruce cae tarde y se distingue con claridad de la edad objetivo.
async fn seed(app: &TestApp, u: &LoggedInOwner) {
    let inc = app.create_category(u, "income", "Nómina").await;
    let exp = app.create_category(u, "expense", "Vida").await;
    let ast = app.create_category(u, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "2400"), (&exp, "1800")] {
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

/// Meses de la rejilla hasta el mes en que el usuario cumple `age`, con la MISMA aritmética civil
/// que publica la respuesta (`anchor + m meses`, conservando el día con recorte a fin de mes).
///
/// Se calcula aquí, en el test, a partir del ancla que la propia respuesta declara: derivarlo del
/// código del handler haría que el test confirmara la implementación en vez de la definición.
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

/// `retire_at_age`: la jubilación cae EXACTAMENTE en el mes en que se cumple la edad, el cruce
/// sigue publicándose como lectura, y el objetivo sigue dibujándose.
#[tokio::test]
async fn retire_at_age_puts_the_retirement_on_the_birthday_month_and_keeps_the_crossing_as_a_reading()
{
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await; // nace 1990-01-01
    seed(&app, &owner).await;

    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "retire_at_age", "target_retirement_age": 55, "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil: {r:?}");

    let s = series(&app, &owner.cookie, "?months=600").await;
    let anchor = NaiveDate::parse_from_str(s["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d")
        .expect("ancla");
    let birth = NaiveDate::from_ymd_opt(1990, 1, 1).unwrap();
    let r_grid = months_until_age(anchor, birth, 55);

    assert_eq!(s["strategy"], "retire_at_age", "{s}");
    assert_eq!(
        s["jubilacion_month_index"], r_grid,
        "la edad manda: se jubila en el mes {r_grid}, no cuando llega el capital ({s})"
    );
    assert_eq!(
        s["retirement_month_index"], s["jubilacion_month_index"],
        "R8: los dos nombres son el mismo mes: {s}"
    );
    // El invariante que hace comprobable la aritmética de edades: la fecha publicada es el mes en
    // que cumple 55, así que la edad publicada ES la pedida.
    assert_eq!(s["jubilacion_age"], 55, "{s}");
    // Con fecha de nacimiento y edad objetivo no falta NINGÚN dato del ensamblado. Lo que sí
    // puede viajar es el rojo de D17 (`retire_at_age_underfunded`): este hogar ahorra 600 €/mes y
    // no llega a su objetivo a los 55, que es un resultado del modelo y no un dato ausente. Ese
    // camino tiene test propio más abajo.
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(|w| w.as_str())
        .collect();
    for ausente in ["birth_date_missing", "target_retirement_age_missing"] {
        assert!(
            !warnings.contains(&ausente),
            "no falta ningún dato: {warnings:?} en {s}"
        );
    }

    // **La EDAD manda aunque no haya plan**: `R` es un dato del perfil, no una búsqueda, así que
    // el ensamblado lo coloca en el `PhasePlan` sin gastar un solo sorteo. Es lo que hace que un
    // `?months=` —y un miembro del hogar— publiquen su fecha por edad igual.
    assert_eq!(
        s["jubilacion_absent_reason"], "months_override",
        "un `?months=` no resuelve plan, y la ausencia se nombra: {s}"
    );
    // **El objetivo determinista se retiró en 5.0.0**: el motor recibe `fire_target: None` y el
    // cruce ya no se publica, porque no decide nada. Lo que sobrevive es el escalar informativo.
    // `.get(...).is_none()` y no `is_null()` (WP A12): sobre un objeto JSON, indexar una clave
    // que NO EXISTE devuelve `Value::Null`, así que un `is_null()` sobre un campo retirado pasa
    // haga lo que haga el servidor — incluido volver a publicarlo. Lo que hay que comprobar es que
    // la CLAVE no está.
    assert!(s.get("fire_target_series").is_none(), "{s}");
    assert!(s.get("liquid_crossing_month_index").is_none(), "{s}");
    assert!(!s["fire_number_classic_today"].is_null(), "el escalar sigue: {s}");

    // La fase «jubilado» empieza en el mismo mes que el marcador — invariante de comportamiento
    // (§C): se comprueba sobre la serie, no sobre el enum de la estrategia.
    let fases = s["phase_transitions"].as_array().expect("phase_transitions");
    let retired = fases.iter().find(|f| f["phase"] == "retired").expect("fase jubilado");
    assert_eq!(retired["month_index"], r_grid, "{fases:?}");
}

/// `coast` comparte trigger con `retire_at_age` (la edad), y por tanto el mismo mes efectivo.
/// Lo que las distingue (la serie «si dejas de aportar») llega con `solve.rs`; el trigger no.
#[tokio::test]
async fn coast_uses_the_same_age_trigger() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "coast", "target_retirement_age": 60}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = series(&app, &owner.cookie, "?months=600").await;
    let anchor = NaiveDate::parse_from_str(s["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    let r_grid = months_until_age(anchor, NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), 60);
    assert_eq!(s["strategy"], "coast", "{s}");
    assert_eq!(s["jubilacion_month_index"], r_grid, "{s}");
    assert_eq!(s["jubilacion_age"], 60, "{s}");
}

/// **Sin fecha de nacimiento NO HAY PLAN** (5.0.0, C5) — nunca un 500, y nunca una jubilación
/// inventada.
///
/// Es el estado real de cualquier usuario que elija la estrategia antes de rellenar su perfil, y
/// la respuesta tiene que poder decirlo: `plan_absent_reason: "birth_date_missing"` más el aviso
/// es lo que permite a la SPA enseñar «añade tu fecha de nacimiento» en vez de una fecha que no
/// significa nada.
///
/// **Hasta 4.15.x se degradaba al CRUCE** y la respuesta traía un `jubilacion_month_index` que
/// parecía una fecha de jubilación. En v2 no hay cruce al que degradar, y la única salida honesta
/// es la ausencia con su razón: la proyección de patrimonio sigue publicándose entera.
#[tokio::test]
async fn an_age_strategy_without_a_birth_date_has_no_plan_and_says_why() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;

    // Se quita la fecha de nacimiento por la misma ruta que la escribe.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "retire_at_age", "target_retirement_age": 55, "birth_date": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(r.json()["birth_date"].is_null(), "la DOB debía borrarse: {}", r.json());

    // Sin `?months=`: así la única razón de ausencia posible es la fecha de nacimiento.
    let s = series(&app, &owner.cookie, "").await;
    assert_eq!(s["strategy"], "retire_at_age", "la estrategia guardada no se toca: {s}");
    assert_eq!(s["plan_absent_reason"], "birth_date_missing", "{s}");
    assert!(
        s["jubilacion_month_index"].is_null(),
        "sin DOB no hay edad que convertir en mes, y no se inventa ninguna: {s}"
    );
    assert!(s["retirement_date_basis"].is_null(), "sin plan no hay base: {s}");
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert!(warnings.contains(&"birth_date_missing"), "{warnings:?} en {s}");
    // Y la serie determinista se publica igual: un campo opcional del perfil no tumba una
    // lectura.
    assert!(!s["points"].as_array().expect("points").is_empty(), "{s}");
}

/// Una **edad ya cumplida** jubila desde el primer mes de la simulación (mes 0 de la rejilla), no
/// «nunca» ni «en el mes 1 de dentro de un año».
#[tokio::test]
async fn an_already_reached_target_age_retires_immediately() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await; // 1990 → ya pasó de 30
    seed(&app, &owner).await;
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "retire_at_age", "target_retirement_age": 30}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = series(&app, &owner.cookie, "?months=120").await;
    assert_eq!(s["jubilacion_month_index"], 0, "ya está en edad: se jubila hoy ({s})");
    assert_eq!(
        s["jubilacion_date_ymd"], s["anchor_date_ymd"],
        "la fecha del mes 0 es el ancla: {s}"
    );
}

// ---------------------------------------------------------------------------------------------
// Los solves de §B.7: lo que CUESTA jubilarse a esa edad, y lo que sobra
// ---------------------------------------------------------------------------------------------

/// Presupuesto + activo con importes a medida (el `seed` de arriba está clavado a 2.400/1.800).
async fn seed_with(app: &TestApp, u: &LoggedInOwner, income: &str, expense: &str) {
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

/// Sin impuestos las cifras son de servilleta: `objetivo = 12·gasto/SWR`.
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

/// **`retire_at_age` NO bisecciona la fecha: la CONFIRMA.**
///
/// Es la decisión de reparto del solver, vista desde fuera. Con la fecha dada no hay nada que
/// buscar —la puso el usuario—, así que lo que el sorteo hace es medir DOS cosas con un
/// presupuesto grande: si se llega (`success_of_plan` contra el umbral) y cuánto falta aportar
/// (`contribution_required_monthly`). Biseccionar ahí gastaría veinte sorteos para devolver el
/// número que ya se tenía.
///
/// Se comprueba por lo que la respuesta AFIRMA, que es lo único que un test de integración puede
/// ver: `retirement_date_basis` vale `target_age` —no `success_threshold`—, la fecha es
/// exactamente el mes de la edad pedida, y las tres cifras de la aportación mínima viajan (con la
/// fecha decidida por el umbral no existirían: no hay «cuánto me falta para esa fecha»).
///
/// **Sin `?months=` a propósito**: un horizonte a medida no resuelve plan (D7), y sin plan no hay
/// solve que mirar.
#[tokio::test]
async fn retire_at_age_does_not_bisect_it_only_confirms() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed_with(&app, &owner, "2400", "1000").await;
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "retire_at_age", "target_retirement_age": 60, "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = series(&app, &owner.cookie, "").await;
    let anchor = NaiveDate::parse_from_str(s["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d")
        .expect("ancla");
    let r_grid = months_until_age(anchor, NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(), 60);

    assert_eq!(
        s["retirement_date_basis"], "target_age",
        "la fecha es un DATO, no algo que el umbral haya buscado: {s}"
    );
    assert_eq!(s["jubilacion_month_index"], r_grid, "{s}");
    assert_eq!(
        s["safe_date_month_index"], s["jubilacion_month_index"],
        "los dos nombres son la misma fecha: {s}"
    );
    assert_eq!(s["jubilacion_age"], 60, "{s}");
    assert!(s["plan_absent_reason"].is_null(), "hay plan: {s}");

    // Lo que el sorteo SÍ hizo: medir. Y decir con qué muestra.
    assert!(!s["success_of_plan"].is_null(), "{s}");
    assert!(!s["success_wilson_low"].is_null(), "{s}");
    assert!(
        !s["success_sampling_error_pp"].is_null() && !s["paths_used"].is_null(),
        "una probabilidad sin su barra y su N no se puede leer: {s}"
    );
    assert!(s["seed"].is_string(), "la semilla viaja como string: {s}");

    // Y las tres cifras de la aportación mínima, que SOLO existen con la fecha dada.
    assert!(
        s["contribution_underfunded"].is_boolean(),
        "con fecha dada se contesta SIEMPRE cuánto falta aportar: {s}"
    );
    let techo: f64 = s["contribution_required_search_ceiling"]
        .as_str()
        .expect("el techo de búsqueda viaja como decimal-string")
        .parse()
        .expect("decimal");
    assert!(techo > 0.0, "el techo da sentido al infra-financiado: {techo} ({s})");
    if let Some(c) = s["contribution_required_monthly"].as_str() {
        let c: f64 = c.parse().expect("decimal");
        assert!(
            (0.0..=techo).contains(&c),
            "la aportación mínima cae dentro del techo que se exploró: c = {c}, techo = {techo}"
        );
        assert_eq!(
            s["contribution_underfunded"], false,
            "con solución no está infra-financiado: {s}"
        );
    } else {
        assert_eq!(
            s["contribution_underfunded"], true,
            "sin importe, la única lectura es «ni el techo llega»: {s}"
        );
    }

    // El capital necesario HOY viaja siempre — es la única cifra del plan que no depende de la
    // estrategia («cuánto necesitarías para jubilarte ya» se contesta igual).
    assert!(
        !s["needed_capital_today"].is_null() || s["needed_capital_absent_reason"].is_string(),
        "o hay cifra o hay razón; nunca un hueco mudo: {s}"
    );
}

/// **El rojo del modelo v2**: un hogar al que **no le queda tiempo** para llegar a su edad
/// objetivo. La respuesta no falla ni esconde nada — se jubila igual, publica la serie entera, y
/// dice las dos cosas que hacen falta para pintar el banner rojo: que no llega
/// (`contribution_underfunded`) y contra qué cota se midió.
///
/// **Por qué una edad YA CUMPLIDA y no un hogar pobre**: en el modelo v2 el techo de la búsqueda
/// **se dobla** hasta que cumple (`minimum_extra_contribution` en `crates/engine-stochastic`), y
/// el crate lo dice sin adornos — «no es *lo que el hogar puede aportar*: es la cota de la
/// bisección». Así que un hogar con poco sobrante casi nunca sale infra-financiado: sale con una
/// aportación mínima que no puede permitirse, que es información distinta y también útil. Lo que
/// **ningún** importe arregla es no tener meses en los que aportarlo: con la edad objetivo ya
/// cumplida, `R = 1` y la fase de acumulación dura cero meses.
///
/// Predicho a mano: el owner del arnés nació en 1990 (36 años), la edad objetivo son 36 ⇒ `R = 1`
/// ⇒ se jubila el mes 0 de la rejilla con los 20.000 € que tiene, contra un gasto de 1.800 €/mes
/// de por vida. Ni el techo doblado cambia un euro de esa cartera.
#[tokio::test]
async fn a_target_age_with_no_time_to_save_is_flagged_underfunded() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed_with(&app, &owner, "2400", "1800").await;
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "retire_at_age", "target_retirement_age": 36, "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = series(&app, &owner.cookie, "").await;
    assert_eq!(s["jubilacion_month_index"], 0, "la edad ya está cumplida: {s}");
    assert_eq!(s["contribution_underfunded"], true, "{s}");
    assert!(
        s["contribution_required_monthly"].is_null(),
        "sin solución no se publica el techo como si fuera la respuesta: {s}"
    );
    assert!(
        !s["contribution_required_search_ceiling"].is_null(),
        "el techo viaja igual: es lo que da sentido al infra-financiado ({s})"
    );
    // Y aun así la simulación existe entera: se jubila, con su serie y su éxito medido.
    assert_eq!(s["retirement_date_basis"], "target_age", "{s}");
    assert!(!s["points"].as_array().expect("points").is_empty(), "{s}");
    assert!(!s["success_of_plan"].is_null(), "{s}");
}
