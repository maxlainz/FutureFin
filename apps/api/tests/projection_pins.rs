//! PINS de regresión del programa de resolución (Olas 3-7): escenarios canónicos cuyos números
//! van a MOVERSE a propósito, una ola cada vez. Disciplina (plan aprobado 2026-08-30): capturar
//! ANTES de tocar el modelo, predecir a mano el nuevo valor en el cuerpo del PR, implementar, y
//! actualizar el pin con el número predicho citándolo en el CHANGELOG. Si el real no coincide
//! con el predicho, el diagnóstico está mal — no el pin.
//!
//! Valores marcados `a mano:` → derivados con la fórmula; `capturado 4.6.0:` → regresión
//! capturada del código actual (patrón projection_marker.rs), con tolerancia estrecha.

mod common;
use chrono::{Datelike, Duration, Months, Utc};
use common::TestApp;
use serde_json::{json, Value};

/// NW(360) del escenario A **sin jubilación** (5.0.0), recapturado tras el cambio de modelo con la
/// predicción escrita ANTES de medir: sube respecto de los 677.335,52 € de 4.15.x porque el hogar
/// deja de drenar desde el mes 235 (con `?months=` no hay plan, y sin plan no hay jubilación —
/// C5). Medido: **1.193.981,1795** (+516.645,66; el orden de magnitud es el esperado: 125 meses de
/// 1.800 €/mes de gasto que ya no se venden, más su composición al 5 %).
/// Se declara aquí, y no incrustado en el `assert`, para que actualizarlo sea un cambio de una
/// línea con su porqué al lado.
const NW360_SIN_JUBILACION: f64 = 1_193_981.18;

fn nw_at(series: &Value, month: u64) -> f64 {
    series["points"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["month_index"] == month)
        .unwrap_or_else(|| panic!("sin punto para el mes {month}"))["net_worth"]
        .as_f64()
        .unwrap()
}

/// Escenario A — «hipoteca viva en modo A» (lo moverán #144/#142/#124 en las Olas 3-4).
/// Activo 50.000 € líquido al 5 %; ingreso 3.000, gasto 1.200 (persiste en jubilación);
/// hipoteca `french` 150.000 € al TIN 3 %, cuota 800 €/mes, plan de 180 meses; SWR 3,5 %,
/// impuestos ON (escala ES por defecto), modo annual_expense, inflación 0.
#[tokio::test]
async fn pin_escenario_a_hipoteca_viva_modo_a() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("pina").await;
    let cat_a = app.create_category(&owner, "asset", "Fondos").await;
    let cat_i = app.create_category(&owner, "income", "Nomina").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let cat_l = app.create_category(&owner, "liability", "Hipoteca").await;
    let cat_le = app.create_category(&owner, "expense", "Cuota").await;

    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Indexado", "current_value": "50000",
                   "is_liquid": true, "expected_annual_return_percent": "5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    // #150: "Indexado" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.
    //
    // La fecha de fin del plan es relativa A PROPÓSITO (#184): el motor ancla en HOY
    // (`installation_naive_today`, tz por defecto UTC — igual que este `Utc::now()`), así que
    // una fecha absoluta encoge el plan un mes cada día 1 y mueve NW(360). Lo que este pin
    // quiere clavar es la LONGITUD del plan: último día del mes (hoy + 179) → exactamente
    // 180 cuotas vivas (meses 0..=179 de la rejilla), sea cual sea el día en que corra el test.
    let m_start = Utc::now().date_naive().with_day(1).expect("día 1 siempre existe");
    let payment_end = (m_start + Months::new(180) - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    for (path, body) in [
        ("/v1/budget/entries", json!({"category_id": cat_i, "amount": "3000"})),
        ("/v1/budget/entries", json!({"category_id": cat_e, "amount": "1200", "ends_at_retirement": false})),
        ("/v1/liabilities", json!({"category_id": cat_l, "expense_category_id": cat_le,
                                   "label": "Casa", "principal": "150000", "apr_percent": "3",
                                   "payment_amount": "800", "payment_frequency": "monthly",
                                   "repayment_model": "french",
                                   "payment_end_date": payment_end})),
    ] {
        let r = app.post_json_with_cookie(path, body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{path}: {r:?}");
    }
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"fire_settings": {"taxes_enabled": true}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    // 6.0.0 (D-7): el perfil de jubilación (modo del objetivo, SWR) se retiró. Estos pins son la
    // trayectoria SIN jubilarse, que nunca dependió de él: sus cifras no se mueven.
    let s = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();

    // **6.0.0 (D-7)**: el número FIRE clásico, la fecha de jubilación, la estrategia, las fases y
    // el bloque «plan» se retiraron con el modelo de jubilación. La serie es la trayectoria SIN
    // jubilarse —la misma que 5.0.x publicaba con `?months=` («sin plan»)—, y los pins de abajo
    // son exactamente los de entonces.
    for dead in [
        "fire_number_classic_today",
        "fire_number_classic_absent_reason",
        "jubilacion_month_index",
        "retirement_month_index",
        "strategy",
        "plan_absent_reason",
        "phase_transitions",
    ] {
        assert!(s.get(dead).is_none(), "`{dead}` se retiró en 6.0.0: {s}");
    }
    let nw12 = nw_at(&s, 12);
    let nw180 = nw_at(&s, 180);
    let nw360 = nw_at(&s, 360);
    let pts = s["points"].as_array().expect("points");
    for p in pts {
        assert!(p["withdrawal"].is_number(), "falta withdrawal: {p}");
        assert!(p.get("withdrawal_shortfall").is_none(), "retirada: {p}");
    }
    // Y la retirada es 0 antes de jubilarse y > 0 después (el déficit de 1.200 €/mes que el
    // patrimonio ya pinea desde la otra cara).
    let w = |m: u64| -> f64 {
        pts.iter()
            .find(|p| p["month_index"] == m)
            .unwrap_or_else(|| panic!("sin punto {m}"))["withdrawal"]
            .as_f64()
            .unwrap()
    };
    assert_eq!(w(180), 0.0, "sin déficit de caja no hay retirada");
    assert_eq!(w(300), 0.0, "sin jubilación no se drena nunca: el mes 300 tampoco retira");
    // El hogar de un solo miembro NO publica `members[]` en `mine`: la respuesta entera es suya.
    assert!(
        s["members"].as_array().is_some_and(|m| m.is_empty()),
        "members[] solo se llena en household: {}",
        s["members"]
    );
    assert!((nw12 - (-80_006.71)).abs() < 0.01, "NW(12) capturado: {nw12}");
    assert!((nw180 - 316_313.32).abs() < 0.01, "NW(180) capturado: {nw180}");
    // Ola 6 (#140 fase 1): el drenaje de jubilación TRIBUTA — con gasto retirado 1.200 €/mes
    // el bruto era gross_up(14.400)/12 = 1.506,33 con g=1, y NW(360) quedó en 653.270,22.
    // 4.12.1 (extensión B de #178): la base que la cascada construyó durante 234 meses es un
    // DATO observado, así que el drenaje deriva su g real (< 1, creciente) en vez del escalar 1
    // — la exención fiscal del difunto «caja primero» heredada de verdad. NW(360) subió a
    // 676.315,04 (+23.044,82 de impuesto que se cobraba sobre euros que eran base).
    // #184: aquel 676.315,04 se capturó con la fecha absoluta 2041-08-31 y ancla de agosto
    // 2026 = 181 cuotas, una más de las 180 que el escenario declara. Con la longitud del plan
    // ya estable en 180, la cuota 181 no se paga: esos 800 € componen al 5 % los ~180 meses
    // restantes y el residual congelado queda más alto → NW(360) = 677.335,52 (Δ +1.020,48,
    // predicho en #184 y confirmado en local antes de actualizar el pin).
    // El mecanismo exacto está pineado a mano en el engine
    // (`derived_g_rises_along_the_trajectory…`, `the_simulated_withdrawal_also_pays_taxes`).
    //
    // **5.0.0 recaptura NW(360)**: sin jubilación no hay drenaje desde el mes 235, así que la
    // cifra SUBE respecto de los 677.335,52 € de 4.15.x. La cota se comprueba antes que el
    // valor: si algún día el número bajara de aquello, sería que un drenaje volvió a colarse.
    assert!(
        nw360 > 677_335.52,
        "sin drenaje desde el mes 235, NW(360) tiene que superar el pin de 4.15.x: {nw360}"
    );
    assert!((nw360 - NW360_SIN_JUBILACION).abs() < 0.01, "NW(360) capturado: {nw360}");
}

/// Escenario B — «inflación 2,5 %» (lo moverán #146/#139/#149 en la Ola 5).
/// Activo 20.000 € al 7 %; ingreso 2.500, gasto 1.500; SWR 4 %, sin impuestos; inflación 2,5 %.
#[tokio::test]
async fn pin_escenario_b_inflacion() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("pinb").await;
    let cat_a = app.create_category(&owner, "asset", "Fondos").await;
    let cat_i = app.create_category(&owner, "income", "Nomina").await;
    let cat_e = app.create_category(&owner, "expense", "Vida").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({"category_id": cat_a, "name": "Indexado", "current_value": "20000",
                   "is_liquid": true, "expected_annual_return_percent": "7"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    // #150: "Indexado" es el primer (y único) activo del owner, así que crearlo ya sembró el
    // sumidero apuntándole — no hace falta crear la regla a mano.
    for (path, body) in [
        ("/v1/budget/entries", json!({"category_id": cat_i, "amount": "2500"})),
        ("/v1/budget/entries", json!({"category_id": cat_e, "amount": "1500", "ends_at_retirement": false})),
    ] {
        let r = app.post_json_with_cookie(path, body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{path}: {r:?}");
    }
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({"annual_inflation_assumption_percent": "2.5",
                   "fire_settings": {"taxes_enabled": false}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let s = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();

    // 6.0.0 (D-7): sin número FIRE ni fecha de jubilación; ninguna de las dos claves viaja.
    for dead in ["fire_number_classic_today", "jubilacion_month_index", "fire_target_series"] {
        assert!(s.get(dead).is_none(), "`{dead}` se retiró: {s}");
    }

    // INVERTIDO en la Ola 5 (#139; capturado en 4.6.0 con el gasto congelado). Con el gasto
    // indexado al 2,5 % e ingresos planos. Números predichos por la réplica a 50 dígitos ANTES
    // de ejecutar (spike §5.2.2).
    let nw120 = nw_at(&s, 120);
    let nw360 = nw_at(&s, 360);
    assert!((nw120 - 181_037.91).abs() < 0.01, "NW(120) predicho: {nw120}");
    assert!((nw360 - 777_970.12).abs() < 0.01, "NW(360) predicho: {nw360}");
}
