//! **`GET /v1/projection/series?view=household` = Σ de una simulación por miembro** (5.0.0, D9 /
//! §D del plan de #207).
//!
//! Hasta 4.15.x «el hogar» era una sola simulación con las filas de todo el mundo bajo el perfil
//! del solicitante. Con la jubilación convertida en estrategia POR PERSONA eso dejó de tener
//! sentido: dos miembros pueden jubilarse con reglas distintas, a edades distintas y con
//! objetivos distintos, y meterlos en un único bucle produce un patrimonio creíble que no
//! describe el plan de ninguno de los dos.
//!
//! Lo que este fichero clava es la definición entera del agregado, campo a campo:
//!
//! 1. **Σ punto a punto**: el hogar es exactamente la suma de las dos respuestas `mine`.
//! 2. **Lo que NO se puede sumar viaja como `null` + `household_aggregate`**, nunca como un 0 o
//!    un hueco: una jubilación del hogar sería una cifra inventada.
//! 3. **`members[]`** lleva el hito de cada persona, con SU estrategia y SU edad.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn asset(app: &TestApp, u: &LoggedInOwner, cat: &str, name: &str, value: &str) {
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat, "name": name, "current_value": value,
                   "is_liquid": true, "expected_annual_return_percent": "4"}),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "alta de activo: {r:?}");
}

async fn budget(app: &TestApp, u: &LoggedInOwner, cat: &str, amount: &str) {
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": cat, "amount": amount, "ends_at_retirement": false}),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "alta de presupuesto: {r:?}");
}

async fn series(app: &TestApp, cookie: &str, q: &str) -> Value {
    let r = app
        .get_with_cookie(&format!("/v1/projection/series{q}"), cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "GET series{q}: {r:?}");
    r.json()
}

fn point_at(s: &Value, month: u64, field: &str) -> f64 {
    s["points"]
        .as_array()
        .expect("points")
        .iter()
        .find(|p| p["month_index"] == month)
        .unwrap_or_else(|| panic!("sin punto para el mes {month}"))[field]
        .as_f64()
        .unwrap_or_else(|| panic!("{field} del mes {month} no es número"))
}

/// Dos miembros con activos, presupuestos y ESTRATEGIAS distintas. El hogar es la suma exacta de
/// sus dos vistas `mine`, punto a punto y en todos los agregados escalares.
///
/// Se comprueba sobre el horizonte COMÚN (`?months=` explícito) para que las tres respuestas
/// compartan rejilla: sin él, el hogar corre a `max(horizontes)` y comparar el mes 240 de una
/// serie de 648 con el de una de 624 seguiría siendo correcto pero dejaría de ser evidente.
#[tokio::test]
async fn household_series_is_the_sum_of_each_members_mine_series() {
    let app = TestApp::spawn().await;
    let alice = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&alice, "bob", "member").await;

    let cat_a = app.create_category(&alice, "asset", "Fondos A").await;
    let inc_a = app.create_category(&alice, "income", "Nómina A").await;
    let exp_a = app.create_category(&alice, "expense", "Vida A").await;
    asset(&app, &alice, &cat_a, "Indexado A", "40000").await;
    budget(&app, &alice, &inc_a, "3000").await;
    budget(&app, &alice, &exp_a, "1500").await;

    let cat_b = app.create_category(&bob, "asset", "Fondos B").await;
    let inc_b = app.create_category(&bob, "income", "Nómina B").await;
    let exp_b = app.create_category(&bob, "expense", "Vida B").await;
    asset(&app, &bob, &cat_b, "Indexado B", "12000").await;
    budget(&app, &bob, &inc_b, "2000").await;
    budget(&app, &bob, &exp_b, "1200").await;

    // Estrategias distintas: alice por cruce (default), bob a los 60.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"strategy": "retire_at_age", "target_retirement_age": 60}),
            &bob.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil de bob: {r:?}");

    let q = "?months=360";
    let mine_a = series(&app, &alice.cookie, q).await;
    let mine_b = series(&app, &bob.cookie, q).await;
    let hh = series(&app, &alice.cookie, "?months=360&view=household").await;

    assert_eq!(mine_a["view"], "mine");
    assert_eq!(hh["view"], "household");

    // 1. Σ punto a punto, en las CINCO series que se suman.
    let n = hh["points"].as_array().unwrap().len();
    assert_eq!(n, mine_a["points"].as_array().unwrap().len(), "misma rejilla");
    for m in [0u64, 1, 12, 120, 240, 359] {
        for campo in [
            "net_worth",
            "net_worth_liquid",
            "contributed_capital",
            "net_worth_real",
            "withdrawal",
        ] {
            let esperado = point_at(&mine_a, m, campo) + point_at(&mine_b, m, campo);
            let real = point_at(&hh, m, campo);
            assert!(
                (real - esperado).abs() < 0.01,
                "mes {m}, {campo}: hogar {real} ≠ {esperado} (alice + bob)"
            );
        }
    }

    // El patrimonio inicial y el neto mensual también suman.
    let dec = |v: &Value| v.as_str().expect("decimal-string").parse::<f64>().unwrap();
    let snw = dec(&mine_a["starting_net_worth"]) + dec(&mine_b["starting_net_worth"]);
    assert!(
        (dec(&hh["starting_net_worth"]) - snw).abs() < 0.01,
        "starting_net_worth: {} vs {snw}",
        hh["starting_net_worth"]
    );
    let delta = dec(&mine_a["monthly_delta_assumption"]) + dec(&mine_b["monthly_delta_assumption"]);
    assert!(
        (dec(&hh["monthly_delta_assumption"]) - delta).abs() < 0.01,
        "monthly_delta_assumption: {} vs {delta}",
        hh["monthly_delta_assumption"]
    );

    // 2. Las series por activo se CONCATENAN (no se suman): siguen identificadas por su id.
    let ids: Vec<&str> = hh["asset_series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["asset_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "un elemento por activo del hogar: {ids:?}");
    let id_a = mine_a["asset_series"][0]["asset_id"].as_str().unwrap();
    let id_b = mine_b["asset_series"][0]["asset_id"].as_str().unwrap();
    assert!(ids.contains(&id_a) && ids.contains(&id_b), "faltan activos: {ids:?}");

    // 3. Lo que no se puede sumar es `null` CON su razón — nunca un 0 ni un campo ausente.
    for campo in [
        "jubilacion_month_index",
        "jubilacion_date_ymd",
        "jubilacion_age",
        "jubilacion_target_net_worth",
        "jubilacion_series_position",
        "jubilacion_target_net_worth_nominal",
        "retirement_month_index",
        "retirement_series_position",
        "liquid_crossing_month_index",
        "compound_outpaces_true_savings_month_index",
        "strategy",
        "retirement_trigger",
        "pension_start_month_index",
        "partial_retirement_month_index",
        "fire_target_debt_component",
    ] {
        assert!(
            hh.get(campo).map(|v| v.is_null()).unwrap_or(true),
            "{campo} debería ser null en el agregado: {}",
            hh[campo]
        );
    }
    assert_eq!(hh["fire_target_absent_reason"], "household_aggregate", "{hh}");
    assert_eq!(hh["jubilacion_absent_reason"], "household_aggregate", "{hh}");
    assert_eq!(hh["liquid_crossing_absent_reason"], "household_aggregate", "{hh}");
    assert_eq!(
        hh["compound_outpaces_true_savings_absent_reason"], "household_aggregate",
        "{hh}"
    );
    assert!(
        hh["fire_target_series"].as_array().unwrap().is_empty(),
        "sin objetivo del hogar no hay línea que dibujar: {hh}"
    );
    assert!(
        hh["phase_transitions"].as_array().unwrap().is_empty(),
        "las fases son de una persona: {hh}"
    );

    // 4. `members[]`: una fila por miembro, con SU estrategia y SU hito.
    let members = hh["members"].as_array().expect("members");
    assert_eq!(members.len(), 2, "dos miembros: {members:?}");
    let by_name = |n: &str| -> Value {
        members
            .iter()
            .find(|m| m["username"] == n)
            .unwrap_or_else(|| panic!("falta {n} en members: {members:?}"))
            .clone()
    };
    let ma = by_name("alice");
    let mb = by_name("bob");
    assert_eq!(ma["strategy"], "asap", "{ma}");
    assert_eq!(mb["strategy"], "retire_at_age", "{mb}");
    assert_eq!(
        ma["jubilacion_month_index"], mine_a["jubilacion_month_index"],
        "el hito de alice en el agregado debe ser el de su propia vista: {ma}"
    );
    assert_eq!(
        mb["jubilacion_month_index"], mine_b["jubilacion_month_index"],
        "ídem para bob: {mb}"
    );
    assert_eq!(mb["jubilacion_age"], 60, "bob se jubila a los 60 por estrategia: {mb}");
    for m in members {
        assert!(
            m["retirement_month_index"] == m["jubilacion_month_index"],
            "R8: los dos nombres son el mismo mes: {m}"
        );
        assert!(m["coast_fire_month_index"].is_null(), "solve.rs no está en esta ola: {m}");
        assert!(m["user_id"].is_string(), "{m}");
        assert!(m["warnings"].as_array().is_some(), "{m}");
    }
}

/// Un miembro **sin ningún dato** no rompe el agregado: aporta una serie plana de ceros y su fila
/// en `members[]`, no un hueco ni un error.
///
/// Es el estado de cualquier hogar el día que se aprueba a alguien nuevo, así que si el agregado
/// fallara aquí fallaría exactamente cuando más se mira.
#[tokio::test]
async fn a_member_without_data_contributes_zeros_and_still_appears() {
    let app = TestApp::spawn().await;
    let alice = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&alice, "asset", "Fondos").await;
    asset(&app, &alice, &cat, "Indexado", "25000").await;
    let _bob = app.register_and_approve_member(&alice, "bob", "member").await;

    let mine = series(&app, &alice.cookie, "?months=120").await;
    let hh = series(&app, &alice.cookie, "?months=120&view=household").await;

    for m in [0u64, 12, 119] {
        let a = point_at(&mine, m, "net_worth");
        let h = point_at(&hh, m, "net_worth");
        assert!(
            (h - a).abs() < 0.01,
            "el miembro sin datos no debe mover el agregado (mes {m}): {h} vs {a}"
        );
    }
    let members = hh["members"].as_array().expect("members");
    assert_eq!(members.len(), 2, "bob existe aunque no tenga nada: {members:?}");
    let bob = members.iter().find(|m| m["username"] == "bob").expect("bob");
    assert!(
        bob["jubilacion_month_index"].is_null(),
        "sin datos no hay jubilación que alcanzar: {bob}"
    );
    assert!(bob["assets_depleted_month_index"].is_null(), "{bob}");
}

/// Un usuario **registrado y pendiente de aprobación NO es del hogar**: su patrimonio no entra en
/// el agregado ni aparece en `members[]`.
///
/// La frontera es `installation_memberships`, la misma que decide el acceso. Sumar a un pendiente
/// enseñaría el dinero de alguien a quien todavía no se ha dado entrada — y al revés, alguien
/// pendiente vería su patrimonio publicado en la pantalla de otro.
#[tokio::test]
async fn pending_users_are_not_part_of_the_household_aggregate() {
    let app = TestApp::spawn().await;
    let alice = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&alice, "asset", "Fondos").await;
    asset(&app, &alice, &cat, "Indexado", "25000").await;

    // Registro SIN aprobación: queda pendiente.
    let reg = app
        .post_json(
            "/v1/auth/register",
            json!({"username": "carol", "password": "correct horse battery staple",
                   "birth_date": "1988-03-03"}),
        )
        .await;
    assert_eq!(reg.status, http::StatusCode::CREATED, "{reg:?}");

    let hh = series(&app, &alice.cookie, "?months=120&view=household").await;
    let members = hh["members"].as_array().expect("members");
    assert_eq!(members.len(), 1, "solo alice es del hogar: {members:?}");
    assert_eq!(members[0]["username"], "alice");
}
