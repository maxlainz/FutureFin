//! Deflactado **servido** (4.5.0): `points[].net_worth_real` en `/v1/projection/series` y la
//! conversión suelta de `/v1/projection/deflate`.
//!
//! Qué defienden estos tests, en dos frentes:
//!
//! 1. **Que sigue siendo capa de presentación.** El motor capitaliza en euros NOMINALES y solo el
//!    objetivo FIRE se ajusta por inflación; deflactar el output no es el modelo «real puro» de la
//!    v1.0.12, rechazado en la v1.2.0 porque mezclaba marcos y drenaba los activos ANTES de la
//!    jubilación. La forma testable de esa afirmación es que `net_worth_real` no lleva NINGUNA
//!    información que el motor no haya producido ya: es exactamente
//!    `net_worth / (1 + i)^(month_index/12)`. Si algún día alguien simulara en euros de hoy, esa
//!    identidad se rompería — que es justo el aviso que se quiere.
//! 2. **Que se deflacta por el MES y no por la posición del array.** Con `density=hybrid` los
//!    puntos no son equidistantes (mensuales hasta el 12, anuales después), así que la versión
//!    ingenua deflacta 70 años como si fueran 30. Es literalmente el bug del chart de la v1.4.2.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::json;

fn dec(v: &serde_json::Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba un string decimal, llegó {v}"))
        .parse()
        .expect("decimal")
}

fn num(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("esperaba un número, llegó {v}"))
}

async fn set_inflation(app: &TestApp, owner: &LoggedInOwner, pct: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "annual_inflation_assumption_percent": pct }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "inflación: {r:?}");
}

/// Hogar con proyección no trivial: 3.000 de ingreso, 1.000 de gasto, un activo líquido al 5 % y
/// el sumidero de la cascada.
async fn seed(app: &TestApp, owner: &LoggedInOwner) {
    let cat_inc = app.create_category(owner, "income", "Nómina").await;
    let cat_exp = app.create_category(owner, "expense", "Vida").await;
    let cat_ast = app.create_category(owner, "asset", "Fondos").await;
    for (cat, amount) in [(&cat_inc, "3000"), (&cat_exp, "1000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                json!({"category_id": cat, "amount": amount}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let asset = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({
                "category_id": cat_ast,
                "name": "MSCI World",
                "current_value": "50000",
                "is_liquid": true,
                "expected_annual_return_percent": "5",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(asset.status, http::StatusCode::CREATED, "{asset:?}");
    // #150: "MSCI World" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.
}

// ---------------------------------------------------------------------------
// 1. `net_worth_real` en la serie
// ---------------------------------------------------------------------------

/// La identidad que define el campo, comprobada punto por punto con `density=hybrid` — la densidad
/// donde la posición del array y el número de mes **divergen**, que es donde vivía el bug de la
/// v1.4.2.
///
/// PREDICCIONES con inflación al 2 %:
/// - `deflation_annual_inflation_percent` = "2.0000" (eco de la asunción de la instalación).
/// - para cada punto servido, `net_worth_real == net_worth / 1,02^(month_index/12)`.
/// - el punto del mes 120 deflacta por **0,8203482999** (1/1,02¹⁰), no por 1/1,02^(posición/12),
///   que bajo hybrid es una posición del orden de 20 ⇒ un deflactor de ~0,968: **17 puntos
///   porcentuales de diferencia** sobre la misma cifra.
/// - `net_worth` sigue siendo NOMINAL: estrictamente mayor que `net_worth_real` en todo mes > 0.
#[tokio::test]
async fn net_worth_real_deflates_by_month_index_not_by_array_position() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    set_inflation(&app, &owner, "2").await;

    let r = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["density"], "hybrid");
    assert_eq!(dec(&b["deflation_annual_inflation_percent"]), 2.0);

    let points = b["points"].as_array().unwrap();
    assert!(points.len() > 20, "hybrid debe servir bastantes puntos");

    let mut vio_divergencia = false;
    for (pos, p) in points.iter().enumerate() {
        let m = p["month_index"].as_u64().unwrap() as f64;
        let nominal = num(&p["net_worth"]);
        let real = num(&p["net_worth_real"]);
        let esperado = nominal / 1.02f64.powf(m / 12.0);
        assert!(
            (real - esperado).abs() <= esperado.abs() * 1e-9 + 1e-6,
            "mes {m} (posición {pos}): net_worth_real esperado {esperado}, obtenido {real}"
        );
        if m > 0.0 {
            assert!(
                real < nominal,
                "con inflación positiva el nominal es mayor que el real (mes {m})"
            );
            // La prueba de que NO se está deflactando por la posición: en hybrid, más allá del
            // mes 24, `pos` y `month_index` se separan tanto que los dos deflactores no pueden
            // confundirse.
            if m > 24.0 && (m - pos as f64).abs() > 1.0 {
                vio_divergencia = true;
                let por_posicion = nominal / 1.02f64.powf(pos as f64 / 12.0);
                assert!(
                    (real - por_posicion).abs() > esperado.abs() * 1e-6,
                    "mes {m} / posición {pos}: deflactar por la posición daría {por_posicion} y \
                     coincide con el servido {real} — el bug de la v1.4.2 está de vuelta"
                );
            }
        }
    }
    assert!(
        vio_divergencia,
        "el test no ha llegado a comparar ningún punto donde mes ≠ posición: no prueba nada"
    );
}

/// Con inflación 0 el deflactor es **exactamente** 1, así que `net_worth_real` es el mismo valor
/// que `net_worth` — y el campo **sigue viajando**.
///
/// Omitirlo cuando no hay inflación dejaría a un consumidor sin poder distinguir «no hay
/// inflación» de «esta versión no publica el campo». Es el mismo fallo que ya costó los cuatro
/// campos de jubilación, que se publican con `null` explícito por esta razón.
#[tokio::test]
async fn with_zero_inflation_the_real_series_is_the_nominal_one_and_still_travels() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    // La instalación nace con inflación 0; se fija explícitamente para que el test no dependa del
    // default.
    set_inflation(&app, &owner, "0").await;

    let b = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await
        .json();
    assert_eq!(dec(&b["deflation_annual_inflation_percent"]), 0.0);

    let points = b["points"].as_array().unwrap();
    assert!(!points.is_empty());
    for p in points {
        assert!(
            p.get("net_worth_real").is_some(),
            "el campo debe viajar también sin inflación: {p}"
        );
        assert_eq!(
            num(&p["net_worth_real"]),
            num(&p["net_worth"]),
            "sin inflación el deflactor es exactamente 1, no aproximadamente 1"
        );
    }
    // Y `milestones_real` sí queda vacío con inflación 0 (contrato previo, intacto): la web reusa
    // `milestones`. Los dos campos responden a preguntas distintas y por eso se comportan distinto.
    assert_eq!(b["milestones_real"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// 2. `GET /v1/projection/deflate`
// ---------------------------------------------------------------------------

/// Las dos direcciones a la vez, etiquetadas. `amount` por sí solo es ambiguo —¿está en euros de
/// aquel mes o en los de hoy?— y elegir una por el llamante es cómo se cuela un error de signo en
/// una respuesta que parece razonable.
///
/// PREDICCIONES con inflación al 2 % y mes 120 (diez años):
/// - `deflator` = 1/1,02¹⁰ = **0,8203482999**
/// - `amount_in_today_euros` = 1.000 × 0,82034830 = **820,3483 €**
/// - `amount_in_month_euros` = 1.000 × 1,02¹⁰ = **1.218,9944 €**
/// - las dos son inversas exactas: `today × month == amount²`.
#[tokio::test]
async fn deflate_serves_both_directions_with_the_predicted_factors() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    set_inflation(&app, &owner, "2").await;

    let b = app
        .get_with_cookie(
            "/v1/projection/deflate?amount=1000&month_index=120",
            &owner.cookie,
        )
        .await
        .json();

    assert_eq!(b["month_index"], 120);
    assert_eq!(dec(&b["annual_inflation_percent"]), 2.0);
    assert_eq!(dec(&b["amount"]), 1000.0);

    let deflator = dec(&b["deflator"]);
    assert!(
        (deflator - 0.8203482999).abs() < 1e-9,
        "deflactor esperado ≈ 0,8203482999, obtenido {deflator}"
    );
    let hoy = dec(&b["amount_in_today_euros"]);
    assert!(
        (hoy - 820.3483).abs() < 0.001,
        "1.000 € nominales de dentro de 10 años valen ≈ 820,35 € de hoy, obtenido {hoy}"
    );
    let entonces = dec(&b["amount_in_month_euros"]);
    assert!(
        (entonces - 1218.9944).abs() < 0.001,
        "1.000 € de hoy costarán ≈ 1.218,99 € dentro de 10 años, obtenido {entonces}"
    );
    // Inversas exactas la una de la otra.
    assert!(
        (hoy * entonces - 1_000_000.0).abs() < 1.0,
        "las dos direcciones deben ser inversas: {hoy} × {entonces}"
    );

    // Mes 0 = hoy: deflactor exactamente 1 en las dos direcciones.
    let cero = app
        .get_with_cookie(
            "/v1/projection/deflate?amount=1000&month_index=0",
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(dec(&cero["deflator"]), 1.0);
    assert_eq!(dec(&cero["amount_in_today_euros"]), 1000.0);
    assert_eq!(dec(&cero["amount_in_month_euros"]), 1000.0);
}

/// `date` y `month_index` son el MISMO eje expresado de dos formas: la fecha se mapea al mes con
/// la misma rejilla de meses civiles que `points[].month_index`. Si divergieran, un cliente que
/// pregunta por «2036» y otro que pregunta por «el mes 120» recibirían deflactores distintos para
/// el mismo instante.
#[tokio::test]
async fn deflate_by_date_and_by_month_index_agree() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    set_inflation(&app, &owner, "3").await;

    let por_indice = app
        .get_with_cookie(
            "/v1/projection/deflate?amount=25000&month_index=60",
            &owner.cookie,
        )
        .await
        .json();
    let mes_ymd = por_indice["month_ymd"].as_str().unwrap().to_string();

    let por_fecha = app
        .get_with_cookie(
            &format!("/v1/projection/deflate?amount=25000&date={mes_ymd}"),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(por_fecha["month_index"], 60);
    assert_eq!(
        dec(&por_fecha["amount_in_today_euros"]),
        dec(&por_indice["amount_in_today_euros"])
    );

    // Cualquier día del mismo mes cae en el mismo mes: la rejilla es de meses civiles.
    let a_mitad = format!("{}15", &mes_ymd[..8]);
    let mitad = app
        .get_with_cookie(
            &format!("/v1/projection/deflate?amount=25000&date={a_mitad}"),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(mitad["month_index"], 60, "{mitad}");
}

/// Las cuatro formas de pedirlo mal, cada una con su código estable. Nada de precedencias
/// inventadas: pedir las dos coordenadas a la vez es un error, no una respuesta plausible.
#[tokio::test]
async fn deflate_rejects_ambiguous_and_out_of_range_inputs() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let casos: &[(&str, &str)] = &[
        (
            "/v1/projection/deflate?amount=100",
            "deflate_timing_ambiguous",
        ),
        (
            "/v1/projection/deflate?amount=100&month_index=12&date=2030-01-01",
            "deflate_timing_ambiguous",
        ),
        (
            "/v1/projection/deflate?amount=100&month_index=841",
            "deflate_month_out_of_range",
        ),
        (
            "/v1/projection/deflate?amount=100&date=1999-01-01",
            "deflate_date_in_past",
        ),
        (
            "/v1/projection/deflate?amount=1.234,56&month_index=12",
            "decimal_invalid",
        ),
        (
            "/v1/projection/deflate?amount=100&date=01/03/2030",
            "date_invalid",
        ),
    ];
    for (uri, code) in casos {
        let r = app.get_with_cookie(uri, &owner.cookie).await;
        assert_eq!(
            r.status,
            http::StatusCode::BAD_REQUEST,
            "esperaba 400 {code} en {uri}: {r:?}"
        );
        assert_eq!(r.json()["code"], *code, "en {uri}");
    }

    // Sin sesión no se contesta.
    let anon = app.get("/v1/projection/deflate?amount=100&month_index=12").await;
    assert_eq!(anon.status, http::StatusCode::UNAUTHORIZED);
}

/// El deflactado servido y el que ya producía `milestones_real` son **el mismo cálculo**: un hito
/// real cruzado en el mes M tiene que corresponderse con el patrimonio deflactado de ese mes.
/// Dos deflactores distintos en la misma respuesta serían dos verdades sobre la misma cifra.
#[tokio::test]
async fn the_served_deflator_is_the_one_behind_milestones_real() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    set_inflation(&app, &owner, "2").await;

    let b = app
        .get_with_cookie("/v1/projection/series?density=monthly", &owner.cookie)
        .await
        .json();
    let hitos = b["milestones_real"].as_array().unwrap();
    assert!(!hitos.is_empty(), "el hogar sembrado debe cruzar algún hito");
    let points = b["points"].as_array().unwrap();

    for h in hitos {
        let target = dec(&h["target"]);
        let m = h["reached_month_index"].as_u64().unwrap() as usize;
        // Con `density=monthly` la posición SÍ es el mes, así que se puede indexar directamente.
        assert_eq!(points[m]["month_index"].as_u64().unwrap() as usize, m);
        let real = num(&points[m]["net_worth_real"]);
        assert!(
            real >= target,
            "el hito real {target} se declara alcanzado en el mes {m}, donde el patrimonio \
             deflactado servido es {real}"
        );
        if m > 0 {
            let anterior = num(&points[m - 1]["net_worth_real"]);
            assert!(
                anterior < target,
                "…y el mes anterior aún no llegaba: {anterior} < {target}"
            );
        }
    }

    // Y el deflactado suelto coincide con el de la serie, punto por punto.
    let p = &points[120];
    let suelto = app
        .get_with_cookie(
            &format!(
                "/v1/projection/deflate?amount={}&month_index=120",
                num(&p["net_worth"])
            ),
            &owner.cookie,
        )
        .await
        .json();
    let esperado = num(&p["net_worth_real"]);
    let obtenido = dec(&suelto["amount_in_today_euros"]);
    assert!(
        (esperado - obtenido).abs() < 0.01,
        "el deflactado de la serie ({esperado}) y el de /deflate ({obtenido}) deben coincidir"
    );
}

// ---------------------------------------------------------------------------
// #146 (Ola 5): inflación NEGATIVA — deflactar hacia ARRIBA
// ---------------------------------------------------------------------------

/// Con inflación −2 % el deflactor es > 1 en meses futuros: los euros de un mundo deflacionario
/// valen MÁS en euros de hoy, así que `net_worth_real` queda ESTRICTAMENTE por encima del
/// nominal (hasta 4.8.0 el gate `<= ZERO` devolvía deflactor 1 y las dos series eran idénticas).
/// Verificación puntual: real(m) == nominal(m) / 0,98^(m/12), y en m = 12 el cociente es
/// exactamente 1/0,98. Además `milestones_real` deja de venir vacío (con inflación 0 sigue
/// vacío — ese contrato no cambia) y la serie del objetivo DECRECE.
#[tokio::test]
async fn negative_inflation_deflates_upward_and_shrinks_the_target() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed(&app, &owner).await;
    set_inflation(&app, &owner, "-2").await;

    let s = app
        .get_with_cookie("/v1/projection/series?months=120&density=monthly", &owner.cookie)
        .await
        .json();
    let pts = s["points"].as_array().unwrap();

    for p in pts.iter().filter(|p| p["month_index"].as_i64().unwrap() > 0) {
        let m = p["month_index"].as_i64().unwrap();
        let nominal = num(&p["net_worth"]);
        let real = num(&p["net_worth_real"]);
        assert!(
            real > nominal,
            "mes {m}: con deflación lo real ({real}) debe superar lo nominal ({nominal})"
        );
        let esperado = nominal / 0.98f64.powf(m as f64 / 12.0);
        assert!(
            (real - esperado).abs() <= esperado.abs() * 1e-9 + 1e-6,
            "mes {m}: real esperado {esperado}, obtenido {real}"
        );
    }

    // El objetivo decrece: la serie paralela del target baja entre el primer y el último punto.
    let ft = s["fire_target_series"].as_array().unwrap();
    if ft.len() >= 2 {
        assert!(
            num(&ft[ft.len() - 1]) < num(&ft[0]),
            "con inflación negativa el objetivo debe DECRECER a lo largo de la serie"
        );
    }

    // Y los hitos en euros de hoy viajan (con inflación 0 siguen vacíos — contrato intacto).
    assert!(
        !s["milestones_real"].as_array().unwrap().is_empty()
            || s["milestones"].as_array().unwrap().is_empty(),
        "si hay hitos nominales, con deflación debe haber hitos reales: {s}"
    );
}
