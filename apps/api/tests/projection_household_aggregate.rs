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

/// **`members[].series` es la línea fina por miembro** (5.0.0, D32) — y suma exactamente
/// `points[]`.
///
/// El chart del hogar dibuja la suma en grueso y una línea fina por persona. Esas líneas no se
/// pueden derivar en cliente: el agregado es una suma y una suma no se puede desagregar. Lo que
/// este test clava es lo que las hace fiables:
///
/// 1. **Misma rejilla y misma decimación** que `points[]` — si el servidor decimara cada serie por
///    su cuenta, el chart tendría que reconciliar dos ejes X en el mismo JSON.
/// 2. **Σ miembros == `points[]`**, mes a mes: si no cuadrara, la línea gruesa y las finas
///    contarían historias distintas sobre los mismos datos.
/// 3. **`horizon_months` propio**: en un hogar con edades distintas el agregado corre a
///    `max(horizontes)`, así que hay que poder decir dónde acaba el plan de cada uno.
#[tokio::test]
async fn every_member_publishes_its_own_series_on_the_same_grid() {
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

    // Bob vive hasta los 100: su horizonte PROPIO es mayor que el de alice (90 por defecto), que
    // es justo el caso que `horizon_months` existe para poder contar.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"horizon_lifespan_age": 100}),
            &bob.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil de bob: {r:?}");

    // Sin `?months=`: así el horizonte es el COMÚN derivado (`household_max_lifespan`) y
    // `horizon_months` por miembro tiene algo que decir.
    let hh = series(&app, &alice.cookie, "?view=household&density=hybrid").await;
    assert_eq!(hh["horizon_basis"], "household_max_lifespan", "{hh}");

    let grid: Vec<u64> = hh["points"]
        .as_array()
        .expect("points")
        .iter()
        .map(|p| p["month_index"].as_u64().expect("month_index"))
        .collect();
    let members = hh["members"].as_array().expect("members");
    assert_eq!(members.len(), 2, "{members:?}");

    let common = hh["months"].as_u64().expect("months");
    for m in members {
        let ms = m["series"].as_array().expect("series por miembro");
        let member_grid: Vec<u64> = ms
            .iter()
            .map(|p| p["month_index"].as_u64().expect("month_index"))
            .collect();
        assert_eq!(member_grid, grid, "misma rejilla que points para {}: {m}", m["username"]);
        // `horizon_months` es el PROPIO, y nunca mayor que el común (el común es el máximo).
        let own = m["horizon_months"].as_u64().expect("horizon_months");
        assert!(own > 0 && own <= common, "horizonte propio {own} vs común {common}: {m}");
    }
    // Bob (edad límite 100) es quien fija el horizonte común; alice se queda corta.
    let own_a = members.iter().find(|m| m["username"] == "alice").unwrap()["horizon_months"]
        .as_u64()
        .unwrap();
    let own_b = members.iter().find(|m| m["username"] == "bob").unwrap()["horizon_months"]
        .as_u64()
        .unwrap();
    assert!(own_b > own_a, "bob vive 10 años más: {own_b} vs {own_a}");
    assert_eq!(own_b, common, "el común es el máximo de los propios: {own_b} vs {common}");

    // Σ de las series por miembro == la serie agregada, punto a punto y en los dos campos.
    for (pos, month) in grid.iter().enumerate() {
        for campo in ["net_worth", "net_worth_liquid"] {
            let suma: f64 = members
                .iter()
                .map(|m| m["series"][pos][campo].as_f64().expect("f64"))
                .sum();
            let agregado = point_at(&hh, *month, campo);
            assert!(
                (suma - agregado).abs() < 0.01,
                "Σ miembros != agregado en {campo} del mes {month}: {suma} vs {agregado}"
            );
        }
    }

    // En `view=mine` no hay `members[]` — la respuesta entera ES de una persona.
    let mine = series(&app, &alice.cookie, "?view=mine").await;
    assert!(mine["members"].as_array().expect("members").is_empty(), "{mine}");
}

/// **Presupuesto de payload del agregado del hogar** (D32 + `mcp-catalog.md`).
///
/// `members[].series` multiplica la parte más pesada de la respuesta por el número de miembros, y
/// la tool MCP `get_projection` fuerza `density=hybrid` justo porque el contexto de un modelo es
/// caro. Esto lo MIDE en vez de suponerlo: imprime el tamaño con y sin las series por miembro (la
/// diferencia es el coste real de la decisión) y falla si el agregado de dos miembros se dispara.
///
/// **Medido el 2026-09-03** con este mismo hogar (dos miembros, un activo + nómina + gasto cada
/// uno, horizonte derivado ~780 meses ⇒ 78 puntos hybrid), bytes sin gzip:
/// `mine/hybrid` 21.009 · `household/hybrid` **34.161** (de los cuales `members[].series`
/// **11.748**, ~5,9 KB por miembro, y `points[]` 15.457) · `household/monthly` 300.724.
///
/// La conclusión que esa medida obliga a tomar está en la tool MCP `get_projection`, que fuerza
/// `hybrid` y donde 6 KB por miembro sí compiten con la conversación: allí `members[].series` es
/// **opt-in** (`include_member_series`, default false), igual que `asset_series`. Por HTTP viaja
/// siempre — el chart del hogar no puede dibujar la línea fina sin ella y no hay forma de
/// derivarla en cliente (una suma no se desagrega).
///
/// La cota de aquí es el doble de lo medido: no persigue el byte, caza el crecimiento lineal que
/// no se vio venir (un campo nuevo por punto multiplica por 78 y por el número de miembros). Si
/// se pone roja, la salida honesta es recortar lo que se publica por miembro, no subir el número.
#[tokio::test]
async fn the_household_payload_stays_within_its_budget_at_hybrid_density() {
    const HYBRID_HOUSEHOLD_MAX_BYTES: usize = 68_000;

    let app = TestApp::spawn().await;
    let alice = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&alice, "bob", "member").await;

    for (u, tag, val, inc, exp) in [
        (&alice, "A", "40000", "3000", "1500"),
        (&bob, "B", "12000", "2000", "1200"),
    ] {
        let ca = app.create_category(u, "asset", &format!("Fondos {tag}")).await;
        let ci = app.create_category(u, "income", &format!("Nómina {tag}")).await;
        let ce = app.create_category(u, "expense", &format!("Vida {tag}")).await;
        asset(&app, u, &ca, &format!("Indexado {tag}"), val).await;
        budget(&app, u, &ci, inc).await;
        budget(&app, u, &ce, exp).await;
    }

    async fn raw(app: &TestApp, cookie: &str, q: &str) -> usize {
        let r = app
            .get_with_cookie(&format!("/v1/projection/series{q}"), cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
        r.body.len()
    }

    let hh = raw(&app, &alice.cookie, "?view=household&density=hybrid").await;
    let mine = raw(&app, &alice.cookie, "?view=mine&density=hybrid").await;
    let hh_monthly = raw(&app, &alice.cookie, "?view=household&density=monthly").await;

    let body = series(&app, &alice.cookie, "?view=household&density=hybrid").await;
    let member_series: usize = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| serde_json::to_string(&m["series"]).unwrap().len())
        .sum();
    let points_bytes = serde_json::to_string(&body["points"]).unwrap().len();
    println!(
        "payload (bytes, sin gzip): mine/hybrid={mine} · household/hybrid(2 miembros)={hh} · household/monthly={hh_monthly} · de los cuales members[].series={member_series} y points={points_bytes}"
    );
    assert!(
        hh <= HYBRID_HOUSEHOLD_MAX_BYTES,
        "el agregado del hogar a densidad hybrid con 2 miembros pesa {hh} B y el presupuesto es \
         {HYBRID_HOUSEHOLD_MAX_BYTES} B (medido 34.161 B el 2026-09-03). Recorta lo que se \
         publica por miembro —`members[].series` lleva DOS importes por punto a propósito— en vez \
         de subir la constante."
    );
    // El coste de la decisión, aislado: si esto deja de ser la mitad larga de la diferencia entre
    // `mine` y `household`, alguien ha metido otra cosa por miembro sin medirla.
    assert!(
        member_series < hh,
        "las series por miembro no pueden ser toda la respuesta: {member_series} de {hh}"
    );
}
