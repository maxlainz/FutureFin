//! **El perfil de jubilación → el `PhasePlan` que el motor ejecuta**: qué entra, qué NO entra y
//! qué mes se fuerza (5.0.0, modelo v2).
//!
//! Lo que se pinea AQUÍ es el **mapeo y la publicación**, no la aritmética: que el perfil de un
//! usuario se convierta en el plan correcto (edades → meses del bucle, reglas de retirada,
//! bloques que la estrategia no usa) y que lo que sale por el wire sea la cifra que el motor
//! calculó, en su rejilla y con su unidad.
//!
//! **5.0.0 retiró de aquí las tres familias de tests del objetivo determinista** —el hueco de la
//! media jornada (`partial_gap_target`), el objetivo puente y sus tres bases de descuento, y el
//! mes/número/serie del coast contra el objetivo—: las tres capitalizaban una necesidad al SWR
//! para decidir algo, y en el modelo v2 la fecha la decide el umbral de éxito sobre miles de
//! caminos (`crates/engine-stochastic`), no un cruce. Su cobertura vive hoy en
//! `projection_plan_solve.rs` (los dos niveles del plan por HTTP) y en los tests del crate
//! estocástico (la bisección y el criterio).
//!
//! Los impuestos van FUERA en todos estos tests: con el gross-up de tramos españoles encima,
//! ninguna cifra sería comprobable a mano — que es justo lo que estos tests compran.

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

async fn patch_profile(app: &TestApp, u: &LoggedInOwner, body: Value) {
    let r = app
        .patch_json_with_cookie("/v1/auth/me/retirement-profile", body, &u.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil: {r:?}");
}

/// Los impuestos van FUERA en todos los tests de este fichero: el ejemplo del issue («2.000 €/mes,
/// 4 %, sin impuestos ⇒ 600.000 / 270.000») está escrito así, y con el gross-up de tramos
/// españoles encima ninguna de las cifras sería comprobable a mano — que es justo lo que estos
/// tests compran.
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

/// Presupuesto + un activo líquido.
async fn seed(app: &TestApp, u: &LoggedInOwner, income: &str, expense: &str, asset_return: &str) {
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
                   "is_liquid": true, "expected_annual_return_percent": asset_return}),
            &u.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
}

/// Meses de la rejilla hasta el mes en que el usuario cumple `age`, con la MISMA aritmética civil
/// que publica la respuesta. Se recalcula en el test —y no se importa del handler— para que lo
/// que se compruebe sea la DEFINICIÓN y no la implementación.
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

fn anchor_of(s: &Value) -> NaiveDate {
    NaiveDate::parse_from_str(s["anchor_date_ymd"].as_str().expect("ancla"), "%Y-%m-%d")
        .expect("ancla parseable")
}

const OWNER_BIRTH: (i32, u32, u32) = (1990, 1, 1);
fn owner_birth() -> NaiveDate {
    NaiveDate::from_ymd_opt(OWNER_BIRTH.0, OWNER_BIRTH.1, OWNER_BIRTH.2).unwrap()
}

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un string decimal, llegó {v}"))
        .parse()
        .expect("decimal parseable")
}

// ---------------------------------------------------------------------------------------------
// El aterrizaje exacto (pase de correcciones de la revisión adversarial)
// ---------------------------------------------------------------------------------------------

/// **La cartera que se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto
/// NO está agotada.**
///
/// Es el hallazgo #2 de la segunda revisión adversarial. Hasta el pase de correcciones,
/// `assets_depleted_month_index` lo decidía un solo predicado —«venta bruta ≥ drenable»— evaluado
/// ANTES de vender, así que un plan perfecto (24 meses de puente pagados al céntimo con una
/// pensión detrás que cubre el 125 % del gasto) se publicaba como «cartera agotada en el mes
/// 311» con `uncovered_deficit_total = 0`: dos cifras de la misma respuesta contándose la una a
/// la otra que mentían. Hoy hacen falta DOS condiciones —la venta dejó la cartera a cero **Y**
/// alguna venta posterior se quedó sin fundar—, y aquí la segunda no se cumple.
///
/// **El puente se mide, no se supone.** Una primera pasada con una cartera holgada dice cuánto
/// cuesta el puente entero (`Σ points[].withdrawal`: después de la pensión no se retira nada, así
/// que la suma del horizonte ES el coste del puente); la segunda pone en el activo exactamente
/// esa cifra. Sin este rodeo el test dependería de que 24 × 2.000 sea la cuenta correcta, que es
/// justo lo que no puede darse por hecho en un test de aterrizajes exactos.
#[tokio::test]
async fn an_exact_landing_on_a_fully_covering_pension_is_not_a_depleted_portfolio() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    // El ingreso TERMINA al jubilarse y el gasto no: durante la acumulación la caja es 0 exacta
    // (2.000 − 2.000), así que el activo no crece ni mengua y el puente empieza con el saldo que
    // este test le ponga.
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": inc, "amount": "2000", "ends_at_retirement": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            json!({"category_id": exp, "amount": "2000", "ends_at_retirement": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    // 0 % de rentabilidad: el saldo solo cambia por lo que se vende, y la aritmética del
    // aterrizaje es exacta en `Decimal`.
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": ast, "name": "Puente", "current_value": "500000",
                   "is_liquid": true, "expected_annual_return_percent": "0"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let asset_id = r.json()["id"].as_str().expect("asset id").to_string();

    // Jubilación por EDAD a los 60 (mes conocido, sin depender del cruce) y pensión CON FECHA a
    // los 62 que cubre el 125 % del gasto: 24 meses de puente y ni un euro de venta después.
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "target_retirement_age": 60, "swr_pct": "4",
               "pension": {"monthly_amount_today": "2500", "starts_at_age": 62}}),
    )
    .await;

    // --- Pasada 1: cartera holgada. Se MIDE el coste del puente. -------------------------------
    let holgado = series(&app, &owner.cookie, "?months=600&density=monthly").await;
    // Con `?months=` no hay plan (D7), pero la EDAD sí coloca el mes: es un dato del perfil, no
    // una búsqueda. Es lo que hace comprobable el aterrizaje sin depender de ningún sorteo.
    assert_eq!(
        holgado["plan_absent_reason"], "months_override",
        "un horizonte a medida no resuelve plan: {holgado}"
    );
    let r_grid = holgado["jubilacion_month_index"].as_u64().expect("jubilación");
    let p_grid = holgado["pension_start_month_index"]
        .as_u64()
        .expect("pensión con fecha");
    assert_eq!(
        p_grid - r_grid,
        24,
        "60 → 62 son 24 meses de puente: {holgado}"
    );
    let puntos = holgado["points"].as_array().expect("points");
    let coste_puente: f64 = puntos.iter().map(|p| p["withdrawal"].as_f64().unwrap()).sum();
    assert!(
        (coste_puente - 48_000.0).abs() < 1.0,
        "24 meses × 2.000 € = 48.000 €, medidos {coste_puente}: {holgado}"
    );
    // Después de la pensión no se vende nada: la suma de arriba es el puente ENTERO y no una
    // parte de un drenaje que sigue.
    for p in puntos.iter().filter(|p| p["month_index"].as_u64().unwrap() > p_grid) {
        assert_eq!(
            p["withdrawal"].as_f64(),
            Some(0.0),
            "la pensión cubre el gasto: no hay venta después: {p}"
        );
    }

    // --- Pasada 2: el aterrizaje EXACTO. -------------------------------------------------------
    let exacto = format!("{coste_puente:.4}");
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{asset_id}"),
            json!({"current_value": exacto}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = series(&app, &owner.cookie, "?months=600&density=monthly").await;
    assert_eq!(
        s["assets_depleted_month_index"],
        Value::Null,
        "la cartera se vacía al céntimo el último mes del puente, y NADIE se queda sin cobrar: \
         eso no es una ruina, es un plan perfecto ({s})"
    );
    assert_eq!(
        s["uncovered_deficit_total"], "0.0000",
        "y el escalar hermano lo confirma — las dos cifras ya no pueden contradecirse: {s}"
    );
    let pts = s["points"].as_array().expect("points");
    for p in pts {
        assert_eq!(
            p["unmet_need"].as_f64(),
            Some(0.0),
            "ninguna venta se quedó sin fundar: {p}"
        );
    }
    // El aterrizaje es REAL: el líquido del cierre del último mes del puente es cero al céntimo.
    let liquido = |m: u64| -> f64 {
        pts.iter()
            .find(|p| p["month_index"].as_u64() == Some(m))
            .unwrap_or_else(|| panic!("sin punto {m}"))["net_worth_liquid"]
            .as_f64()
            .unwrap()
    };
    assert!(
        liquido(p_grid).abs() < 0.01,
        "cierre del último mes del puente: {} (debería ser 0)",
        liquido(p_grid)
    );
    assert!(
        liquido(p_grid - 1) > 0.0,
        "y el mes anterior todavía tenía saldo: {}",
        liquido(p_grid - 1)
    );

    // --- Control negativo: un mes de puente MENOS y el veredicto cambia. ------------------------
    // Sin esto, el test pasaría igual con un handler que publicara `null` siempre.
    let corto = format!("{:.4}", coste_puente - 2_000.0);
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/assets/{asset_id}"),
            json!({"current_value": corto}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let corta = series(&app, &owner.cookie, "?months=600&density=monthly").await;
    assert_ne!(
        corta["assets_depleted_month_index"],
        Value::Null,
        "faltando 2.000 € el último mes del puente SÍ se queda sin fundar: {corta}"
    );
    assert!(
        dec(&corta["uncovered_deficit_total"]) > 0.0,
        "y el descubierto ya no es cero: {corta}"
    );
}

// ---------------------------------------------------------------------------------------------
// U4 — el porcentaje de retirada es ÚNICO (el `pct` omitido hereda `swr_pct`)
// ---------------------------------------------------------------------------------------------

/// **La herencia llega al MOTOR, no solo al formulario.**
///
/// El riesgo de U4 no es que el perfil publique un `pct` bonito: es que el `PhasePlan` que se
/// arma en `handlers/projection.rs` reciba un `0` (o el número viejo) mientras la pantalla enseña
/// el SWR. Eso sería un plan distinto del que el usuario lee, y no habría ningún campo que lo
/// delatara — el modo de fallo de esta casa: la cifra plausible y equivocada.
///
/// Así que se comprueba sobre la salida real de `/v1/projection/series`, con una jubilación
/// DENTRO del horizonte (estrategia por edad) para que la regla de retirada llegue a actuar:
///
/// 1. `percent_of_balance` sin `pct` y `percent_of_balance` con `pct = swr_pct` producen la
///    **misma respuesta byte a byte**. Es la igualdad que define «heredar».
/// 2. Un `pct` distinto produce otra serie. Sin este tercer caso, el punto 1 se cumpliría igual
///    si el motor estuviera ignorando el `pct` por completo.
#[tokio::test]
async fn an_omitted_withdrawal_pct_reaches_the_engine_as_the_swr() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    // Cartera generosa y jubilación por EDAD: el drenaje empieza dentro del horizonte y la regla
    // de retirada gobierna lo que se vende cada mes.
    seed(&app, &owner, "4000", "2000", "5").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": app.create_category(&owner, "asset", "Cartera").await,
                   "name": "Global", "current_value": "400000",
                   "is_liquid": true, "expected_annual_return_percent": "5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");

    // `swr_pct` 3,5 (el default explícito) + jubilación a los 45: la DOB del arnés es 1990-01-01.
    let rule = |pct: Option<&str>| -> Value {
        let mut w = json!({"kind": "percent_of_balance", "spend_mode": "ceiling"});
        if let Some(v) = pct {
            w["pct"] = json!(v);
        }
        json!({
            "strategy": "retire_at_age",
            "target_retirement_age": 45,
            "swr_pct": "3.5",
            "withdrawal_rule": w
        })
    };

    // (a) Sin `pct`: el perfil publica el SWR heredado.
    patch_profile(&app, &owner, rule(None)).await;
    let p = app
        .get_with_cookie("/v1/auth/me/retirement-profile", &owner.cookie)
        .await
        .json();
    assert_eq!(p["profile"]["withdrawal_rule"]["pct"], "3.5", "{p}");
    assert_eq!(p["profile"]["withdrawal_rule"]["pct_source"], "swr", "{p}");
    let inherited = series(&app, &owner.cookie, "").await;

    // La jubilación ocurre dentro del horizonte: sin eso la comparación sería trivial (dos
    // curvas que nunca retiran nada coinciden con cualquier regla).
    let jubilacion = inherited["jubilacion_month_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("la jubilación debe caer dentro del horizonte: {inherited}"));
    assert!(
        jubilacion > 0 && jubilacion < inherited["months"].as_u64().expect("months"),
        "jubilación fuera del horizonte: {jubilacion} de {inherited}"
    );

    // (b) Con el MISMO número escrito a mano: byte a byte lo mismo.
    patch_profile(&app, &owner, rule(Some("3.5"))).await;
    let p = app
        .get_with_cookie("/v1/auth/me/retirement-profile", &owner.cookie)
        .await
        .json();
    assert_eq!(
        p["profile"]["withdrawal_rule"]["pct_source"], "explicit",
        "{p}"
    );
    let explicit = series(&app, &owner.cookie, "").await;
    assert_eq!(
        serde_json::to_string(&inherited).expect("json"),
        serde_json::to_string(&explicit).expect("json"),
        "heredar el SWR debe dar EXACTAMENTE la misma serie que escribirlo a mano"
    );

    // (c) Y con otro número, otra serie: la prueba de que el `pct` de verdad gobierna el motor.
    patch_profile(&app, &owner, rule(Some("1"))).await;
    let other = series(&app, &owner.cookie, "").await;
    assert_ne!(
        serde_json::to_string(&inherited).expect("json"),
        serde_json::to_string(&other).expect("json"),
        "un pct distinto debe mover la serie; si no, el motor está ignorando la regla"
    );
}

// ---------------------------------------------------------------------------------------------
// Los bloques que la estrategia NO usa se conservan en el perfil, pero no entran en la simulación
// ---------------------------------------------------------------------------------------------
//
// El perfil es acumulativo a propósito: cambiar de estrategia y volver no pierde nada, y el GET
// resuelto sigue devolviendo cada bloque que el usuario llegó a rellenar. Lo que estos tests
// pinean es la otra mitad del contrato — que un bloque GUARDADO no es una declaración de que esa
// fase se viva—, porque el ensamblado del plan la incumplía en silencio.

/// Camino del PRIMER punto en que dos respuestas difieren, descendiendo por objetos y arrays
/// hasta el escalar. Baja hasta el fondo a propósito: la divergencia de este bug aparece en el
/// mes ~40 de una serie de 600, así que quedarse en el campo de primer nivel imprimiría dos
/// prefijos idénticos y diría «difieren» sin enseñar dónde.
fn first_difference(a: &Value, b: &Value) -> Option<String> {
    fn walk(a: &Value, b: &Value, path: &str) -> Option<String> {
        if a == b {
            return None;
        }
        match (a, b) {
            (Value::Object(oa), Value::Object(ob)) => {
                for (k, va) in oa {
                    match ob.get(k) {
                        Some(vb) => {
                            if let Some(d) = walk(va, vb, &format!("{path}.{k}")) {
                                return Some(d);
                            }
                        }
                        None => return Some(format!("{path}.{k} solo existe en una de las dos")),
                    }
                }
                ob.keys()
                    .find(|k| !oa.contains_key(*k))
                    .map(|k| format!("{path}.{k} solo existe en una de las dos"))
            }
            (Value::Array(aa), Value::Array(ab)) => {
                if aa.len() != ab.len() {
                    return Some(format!(
                        "{path}: {} elementos con el bloque guardado, {} sin él",
                        aa.len(),
                        ab.len()
                    ));
                }
                aa.iter()
                    .zip(ab.iter())
                    .enumerate()
                    .find_map(|(i, (va, vb))| walk(va, vb, &format!("{path}[{i}]")))
            }
            _ => Some(format!("{path}: {a} con el bloque guardado, {b} sin él")),
        }
    }
    walk(a, b, "")
}

/// Las dos respuestas tienen que ser la MISMA, byte a byte. La igualdad de `Value` da el mensaje
/// legible; la de los dos strings serializados es la comprobación de verdad (el orden de los
/// campos y el formato de cada cifra también son parte de lo que recibe el cliente).
fn assert_same_series(with_block: &Value, without_block: &Value, what: &str) {
    assert!(
        with_block == without_block,
        "{what}: la simulación cambió por un bloque que la estrategia NO usa. Primera \
         diferencia → {}",
        first_difference(with_block, without_block)
            .unwrap_or_else(|| "(difieren en algo que no es un campo de primer nivel)".into())
    );
    assert_eq!(
        serde_json::to_string(with_block).expect("json"),
        serde_json::to_string(without_block).expect("json"),
        "{what}: idénticas byte a byte, no solo equivalentes"
    );
}

fn warnings_of(s: &Value) -> Vec<String> {
    s["warnings"]
        .as_array()
        .expect("`warnings` es siempre un array, vacío si no hay nada que advertir")
        .iter()
        .map(|w| w.as_str().expect("avisos son literales").to_string())
        .collect()
}

/// **Una media jornada guardada NO se simula con `asap`** — el bug medido en vivo sobre la demo
/// (imagen construida de `debc52d`).
///
/// El perfil de la demo, literal: `strategy: "asap"` con una media jornada de una prueba anterior
/// todavía guardada (empieza a los 40, 1.000 €/mes). El ensamblado del plan mapeaba
/// `partial_retirement` al `PhasePlan` **mirara o no la estrategia**, así que la serie llegaba
/// con `warnings: ["partial_phase_capital_shrinking"]`, con un `partial_retirement_month_index` y
/// sin cruzar nunca el objetivo — de una fase que la jubilación rediseñada ni siquiera enseña
/// para esa estrategia (U2), actuando sola sobre los números (U12).
///
/// La aritmética de por qué el síntoma era tan grande, con inflación 0 y sin impuestos: ingreso
/// 3.000 y gasto 2.000 dan +1.000 €/mes; desde la media jornada el ingreso pasa a 1.000 contra
/// los mismos 2.000 de gasto, o sea **−1.000 €/mes** sobre un capital de ~66.000 € que rinde ~275
/// €/mes. El patrimonio menguaba, y el objetivo (`12·2000/0,04 = 600.000 €`) no se alcanzaba
/// jamás. Sin la fase, el cruce llega sobre el mes ~285.
#[tokio::test]
async fn a_stored_partial_block_does_not_reach_the_engine_under_asap() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "asap", "swr_pct": "4",
               "partial_retirement": {"starts_at_age": 40, "income_monthly_today": "1000"}}),
    )
    .await;

    let with_block = series(&app, &owner.cookie, "?months=600").await;

    // El bloque SIGUE guardado: la puerta está en el ensamblado del plan, no en el guardado, y el
    // perfil resuelto lo sigue publicando para cuando el usuario vuelva a `partial`.
    let r = app
        .get_with_cookie("/v1/auth/me/retirement-profile", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "perfil: {r:?}");
    let profile = r.json();
    assert_eq!(
        profile["profile"]["partial_retirement"]["starts_at_age"], 40,
        "el perfil almacenado NO se toca: {profile}"
    );

    // El mismo perfil sin el bloque. Es la referencia: lo que el usuario ve en la UI rediseñada.
    patch_profile(&app, &owner, json!({"partial_retirement": null})).await;
    let without_block = series(&app, &owner.cookie, "?months=600").await;

    assert_same_series(
        &with_block,
        &without_block,
        "`asap` con una media jornada guardada",
    );

    // Y los síntomas medidos, uno a uno.
    assert!(
        with_block["partial_retirement_month_index"].is_null(),
        "`asap` no atraviesa ninguna media jornada: {with_block}"
    );
    assert!(
        !warnings_of(&with_block).contains(&"partial_phase_capital_shrinking".to_string()),
        "un aviso sobre una fase que esta estrategia no simula: {with_block}"
    );
    assert!(
        with_block["partial_start_month_index"].is_null(),
        "las lecturas de la fase describen la MISMA fase, y aquí no hay ninguna: {with_block}"
    );
    // Con `?months=` no hay plan (D7), así que lo que este test compara es la CURVA: es
    // exactamente donde estaba el bug — con la fase dentro, el capital menguaba desde los 40.
    assert_eq!(
        with_block["plan_absent_reason"], "months_override",
        "un horizonte a medida no resuelve plan: {with_block}"
    );
}

/// **Lo mismo con `retire_at_age`**: la edad manda (D17), y una media jornada guardada de otra
/// estrategia no puede meter una fase entre hoy y esa edad.
///
/// Perfil: jubilación total a los 55 con una media jornada guardada a los 40 (1.000 €/mes). Con
/// el bug, el hogar pasaba quince años a −1.000 €/mes antes de jubilarse; sin él, ahorra +1.000
/// hasta los 55. Dos curvas radicalmente distintas para un plan que la UI presenta igual.
#[tokio::test]
async fn a_stored_partial_block_does_not_reach_the_engine_under_retire_at_age() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "retire_at_age", "swr_pct": "4", "target_retirement_age": 55,
               "partial_retirement": {"starts_at_age": 40, "income_monthly_today": "1000"}}),
    )
    .await;

    let with_block = series(&app, &owner.cookie, "?months=600").await;
    patch_profile(&app, &owner, json!({"partial_retirement": null})).await;
    let without_block = series(&app, &owner.cookie, "?months=600").await;

    assert_same_series(
        &with_block,
        &without_block,
        "`retire_at_age` con una media jornada guardada",
    );

    assert_eq!(with_block["strategy"], "retire_at_age", "{with_block}");
    // **La edad sigue mandando aunque no haya plan**: `R` es un DATO del perfil, no una búsqueda,
    // así que el ensamblado lo coloca en el `PhasePlan` sin gastar un solo sorteo. Es lo que hace
    // que un `?months=` —y un miembro del hogar— publiquen su fecha por edad igual.
    assert_eq!(
        with_block["jubilacion_month_index"],
        months_until_age(anchor_of(&with_block), owner_birth(), 55),
        "se jubila el mes en que cumple 55: {with_block}"
    );
    assert!(
        with_block["partial_retirement_month_index"].is_null(),
        "entre hoy y los 55 no hay ninguna fase que la estrategia haya declarado: {with_block}"
    );
    assert!(
        !warnings_of(&with_block).contains(&"partial_phase_capital_shrinking".to_string()),
        "{with_block}"
    );
}

/// **`target_retirement_age` NUNCA filtró, y esto lo pinea.** Es la hermana simétrica de los dos
/// tests de arriba, escrita al comprobar el alcance del bug: la edad objetivo ya pasaba por su
/// propia puerta (`wants_age_trigger`), que la lee solo en `retire_at_age`/`coast` y, como fin
/// OPCIONAL de la media jornada, en `partial`. Con `asap` guardada se conserva y no dispara nada.
///
/// Sin este pin, la puerta de la edad y la de la fase parcial son dos reglas del mismo párrafo
/// con una sola red debajo.
#[tokio::test]
async fn a_stored_target_retirement_age_does_not_retire_an_asap_plan() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;
    patch_profile(
        &app,
        &owner,
        json!({"strategy": "asap", "swr_pct": "4", "target_retirement_age": 55}),
    )
    .await;

    let with_age = series(&app, &owner.cookie, "?months=600").await;
    patch_profile(&app, &owner, json!({"target_retirement_age": null})).await;
    let without_age = series(&app, &owner.cookie, "?months=600").await;

    assert_same_series(
        &with_age,
        &without_age,
        "`asap` con una edad objetivo guardada",
    );

    assert!(
        with_age["jubilacion_month_index"].is_null(),
        "con `asap` la fecha la decide el umbral, y con `?months=` no hay plan: la edad guardada \
         no puede jubilar a nadie: {with_age}"
    );
    assert!(
        !warnings_of(&with_age).contains(&"retire_at_age_underfunded".to_string()),
        "un aviso de la estrategia por edad en un plan que no la usa: {with_age}"
    );
}

// ---------------------------------------------------------------------------------------------
// El motor SIEMPRE recibe un mes forzado (5.0.0, C5)
// ---------------------------------------------------------------------------------------------

/// **El cruce dejó de jubilar: el `PhasePlan` que sale del ensamblado lleva siempre un
/// `RetirementTrigger::AtMonth`, y nunca `LiquidCrossing`.**
///
/// Es la pieza que sostiene el modelo entero. Hasta 4.15.x había DOS disparadores —el cruce
/// contra el objetivo y la edad— y el ensamblado elegía uno; desde 5.0.0 hay uno solo, y el mes lo
/// pone el plan: el umbral de éxito (`asap`, `coast` modo B, `partial`), la edad
/// (`retire_at_age`, `coast` modo A) o, cuando no hay ninguno, **`horizonte + 1`** — «no se
/// jubila dentro del horizonte», que es la línea honesta y no una fecha inventada.
///
/// Se comprueba por su EFECTO observable, que es lo único que un test de integración puede ver:
/// sin plan (`?months=`, sin fecha de nacimiento) la serie sale entera y `jubilacion_month_index`
/// es `null` **con su razón al lado**, nunca un 0 ni un mes que nadie calculó. Si el cruce
/// siguiera vivo, este hogar —que acumula 1.000 €/mes— se jubilaría al cruzar el objetivo y el
/// campo traería un número.
#[tokio::test]
async fn the_engine_always_receives_a_forced_month() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    taxes_off(&app, &owner).await;
    seed(&app, &owner, "3000", "2000", "5").await;
    patch_profile(&app, &owner, json!({"strategy": "asap", "swr_pct": "4"})).await;

    // (a) Horizonte a medida: sin plan, y por tanto sin fecha. La curva se publica igual.
    let sin_plan = series(&app, &owner.cookie, "?months=600").await;
    assert_eq!(sin_plan["plan_absent_reason"], "months_override", "{sin_plan}");
    assert!(
        sin_plan["jubilacion_month_index"].is_null(),
        "sin plan no hay fecha, y el cruce ya no puede poner una: {sin_plan}"
    );
    assert_eq!(
        sin_plan["jubilacion_absent_reason"], "months_override",
        "y la ausencia se nombra, en vez de dejar un hueco: {sin_plan}"
    );
    assert!(
        sin_plan["retirement_date_basis"].is_null(),
        "sin plan no hay base de fecha que declarar: {sin_plan}"
    );
    assert!(
        !sin_plan["points"].as_array().expect("points").is_empty(),
        "la serie determinista se publica igual: {sin_plan}"
    );
    assert!(
        sin_plan["needed_capital_today"].is_null()
            && sin_plan["success_of_plan"].is_null(),
        "ninguna cifra del sorteo viaja sin plan: {sin_plan}"
    );

    // (b) Sin fecha de nacimiento tampoco hay plan (C5), y la razón es OTRA.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"birth_date": Value::Null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "borrar la DOB: {r:?}");
    let sin_dob = series(&app, &owner.cookie, "").await;
    assert_eq!(sin_dob["plan_absent_reason"], "birth_date_missing", "{sin_dob}");
    assert!(sin_dob["jubilacion_month_index"].is_null(), "{sin_dob}");
    assert!(
        warnings_of(&sin_dob).contains(&"birth_date_missing".to_string()),
        "y además se avisa: {}",
        sin_dob["warnings"]
    );
}
