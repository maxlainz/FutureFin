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
//! Y obliga a una decisión de motor (D17): **un solo trigger por simulación**. El objetivo no
//! entra al bucle como `fire_target`, porque si entrara el cruce podría adelantar la jubilación y
//! la edad dejaría de mandar. Lo que sí se sigue publicando es el objetivo como línea del chart:
//! `fire_target_series` y `jubilacion_target_net_worth` viven, solo que ya no deciden.

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
    assert_eq!(s["retirement_trigger"], "target_age", "{s}");
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
    assert!(
        s["warnings"].as_array().unwrap().is_empty(),
        "con fecha de nacimiento y edad objetivo no hay nada que advertir: {s}"
    );

    // El objetivo sigue vivo como LECTURA: línea del chart, base en euros de hoy y nominal del
    // mes en que se jubila.
    assert!(
        !s["fire_target_series"].as_array().unwrap().is_empty(),
        "la línea discontinua no desaparece porque la edad tome el mando: {s}"
    );
    assert!(s["jubilacion_target_net_worth"].is_string(), "{s}");
    assert!(s["jubilacion_target_net_worth_nominal"].is_string(), "{s}");
    assert!(s["fire_target_absent_reason"].is_null(), "hay objetivo: {s}");
    assert!(s["jubilacion_absent_reason"].is_null(), "hay trigger: {s}");
    assert!(
        s["liquid_crossing_absent_reason"].is_null(),
        "hay objetivo contra el que cruzar: {s}"
    );

    // Y el cruce es OTRA cifra: o cae en un mes distinto, o no cae. Lo que no puede es
    // confundirse con el trigger.
    let cruce = s["liquid_crossing_month_index"].clone();
    if let Some(k) = cruce.as_u64() {
        assert_ne!(
            k as u32, r_grid,
            "este escenario está elegido para que el cruce NO coincida con la edad: {s}"
        );
    }

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
    assert_eq!(s["retirement_trigger"], "target_age", "{s}");
    assert_eq!(s["jubilacion_month_index"], r_grid, "{s}");
    assert_eq!(s["jubilacion_age"], 60, "{s}");
}

/// **Sin fecha de nacimiento, una estrategia por edad DEGRADA a `asap` con aviso** — nunca un 500,
/// y nunca una jubilación inventada.
///
/// Es el estado real de cualquier usuario que elija la estrategia antes de rellenar su perfil, y
/// la respuesta tiene que poder decirlo: `warnings: ["birth_date_missing"]` es lo que permite a la
/// SPA enseñar «añade tu fecha de nacimiento» en vez de una fecha de jubilación que no significa
/// nada.
#[tokio::test]
async fn an_age_strategy_without_a_birth_date_degrades_to_asap_with_a_warning() {
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

    let s = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(s["strategy"], "retire_at_age", "la estrategia guardada no se toca: {s}");
    assert_eq!(
        s["retirement_trigger"], "liquid_crossing",
        "sin DOB no hay edad que convertir en mes: se degrada al cruce ({s})"
    );
    let warnings: Vec<&str> = s["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    assert_eq!(warnings, vec!["birth_date_missing"], "{s}");

    // Degradado a `asap` ⇒ el mes efectivo vuelve a SER el cruce.
    assert_eq!(
        s["jubilacion_month_index"], s["liquid_crossing_month_index"],
        "degradado al cruce, las dos cifras coinciden: {s}"
    );
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
    assert_eq!(s["retirement_trigger"], "target_age", "{s}");
    assert_eq!(
        s["jubilacion_date_ymd"], s["anchor_date_ymd"],
        "la fecha del mes 0 es el ancla: {s}"
    );
}
