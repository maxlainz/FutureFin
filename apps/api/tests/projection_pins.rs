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

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba string decimal, llegó {v:?}"))
        .parse::<f64>()
        .expect("decimal")
}

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
    // 5.0.0 (D13): modo del objetivo y SWR son del perfil del usuario. Se escriben explícitos
    // aunque coincidan con los defaults: un pin no se apoya en un default.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "annual_expense", "swr_pct": "3.5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();

    // a mano: need = 1.200×12 = 14.400 (SIN la cuota). Gross-up escala ES:
    // tramo 19 %: 14.400/0,81 = 17.777,78 > 6.000 → K = 1.140;
    // tramo 21 %: (14.400 + 1.140 − 0,21×6.000)/0,79 = 14.280/0,79 = 18.075,9494 ≤ 50.000 ✓.
    // target = 18.075,9494 / 0,035 = 516.455,6961.
    //
    // **5.0.0** — `jubilacion_target_net_worth` (la BASE del objetivo en euros de hoy) y
    // `fire_target_debt_component` (su término finito de deuda) se retiraron con el objetivo como
    // decisión. Lo que sobrevive es el **número FIRE clásico**, un escalar informativo que vale
    // `base + término de deuda` en el mes 0 — las dos mitades que antes viajaban por separado,
    // ahora sumadas.
    //
    // Aquí NO se pinea su valor exacto a propósito: el término de deuda de este escenario lleva
    // el residual del plan francés, que este fichero no deriva a mano (lo pinean los tests del
    // motor de la Ola 4). Lo que se pinea es la COTA que la descomposición implica: la cifra
    // tiene que superar la base de 516.455,6961 € en al menos las 180 cuotas de 800 € que quedan
    // por pagar. El pin exacto y sin deuda está en el escenario B.
    let clasico = dec(&s["fire_number_classic_today"]);
    assert!(
        clasico >= 516_455.6961 + 144_000.0,
        "número FIRE clásico = base (516.455,6961) + término de deuda (≥ 180×800): {clasico}"
    );
    assert!(s["fire_number_classic_absent_reason"].is_null(), "hay número: {s}");

    // capturado 4.6.0 (#144 default french ya aplicado aquí a mano; verificado inmóvil en la
    // Ola 4 por lo de arriba; #124 no aplica — no hay partidas vencidas):
    let jub = s["jubilacion_month_index"].clone();
    let nw12 = nw_at(&s, 12);
    let nw180 = nw_at(&s, 180);
    let nw360 = nw_at(&s, 360);
    // **5.0.0 — este pin cambia de veredicto, y es la consecuencia entera del modelo v2.**
    //
    // Hasta 4.15.x este hogar se jubilaba en el mes 235 porque el LÍQUIDO cruzaba el objetivo
    // determinista. En v2 el cruce no jubila a nadie: la fecha la decide el umbral de éxito, y
    // un `?months=` **no resuelve plan** (D7 — biseccionar sobre miles de caminos en cada
    // petición con horizonte arbitrario pondría decenas de segundos de CPU detrás de un
    // parámetro de query). Así que esta serie es la trayectoria SIN jubilarse: el motor recibe
    // `AtMonth(horizonte + 1)`.
    //
    // Predicho ANTES de medir: `jubilacion_month_index` pasa de 235 a `null`;
    // `phase_transitions` pierde la fase `retired`; la retirada del mes 300 pasa de > 0 a 0; y
    // NW(360) **sube** —el hogar sigue ingresando 3.000 y gastando 1.200 hasta el final en vez
    // de drenar desde el 235—. NW(12) y NW(180) **no se mueven**: los dos caen antes del mes
    // 235, donde las dos versiones simulan exactamente lo mismo.
    assert!(
        jub.is_null(),
        "un `?months=` no resuelve plan, y sin plan no hay fecha: {jub}"
    );
    assert!(s["retirement_month_index"].is_null(), "{s}");
    assert_eq!(s["strategy"], "asap", "estrategia por defecto: {}", s["strategy"]);
    assert_eq!(
        s["jubilacion_absent_reason"], "months_override",
        "la ausencia se nombra: {s}"
    );
    assert_eq!(s["plan_absent_reason"], "months_override", "{s}");
    // Las fases: solo acumulación. Sobre la serie, no sobre el enum (§C: el invariante es de
    // comportamiento).
    let fases = s["phase_transitions"].as_array().expect("phase_transitions");
    assert_eq!(fases.len(), 1, "sin jubilación solo hay acumulación: {fases:?}");
    assert_eq!(fases[0]["phase"], "accumulating");
    assert_eq!(fases[0]["month_index"], 0);
    // Las tres series de retirada existen en cada punto y, con `fixed_real`, recorte y exceso
    // son cero SIEMPRE (la regla no tiene techo): si alguna vez dejan de serlo sin que cambie
    // la regla, es que el motor está recortando por su cuenta.
    let pts = s["points"].as_array().expect("points");
    for p in pts {
        assert!(p["withdrawal"].is_number(), "falta withdrawal: {p}");
        assert_eq!(p["withdrawal_shortfall"], 0.0, "fixed_real no recorta: {p}");
        assert_eq!(p["withdrawal_excess"], 0.0, "ceiling no gasta de más: {p}");
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
    assert_eq!(w(180), 0.0, "en el mes 180 aún no está jubilado");
    assert_eq!(w(300), 0.0, "sin plan no se jubila nunca: el mes 300 tampoco retira");
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
    // valor: si algún día el número bajara de aquello, sería que la jubilación volvió a colarse
    // en una respuesta que declara `plan_absent_reason`.
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
    // 5.0.0 (D13): el modo del objetivo y el SWR son del PERFIL del usuario, no del hogar. El
    // pin no cambia de números — cambia de dónde se escriben los mismos dos ejes.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            json!({"fire_number_mode": "annual_expense", "swr_pct": "4"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let s = app
        .get_with_cookie("/v1/projection/series?months=360", &owner.cookie)
        .await
        .json();

    // a mano: base = 1.500×12/0,04 = 450.000. **Sin deuda**, el número FIRE clásico de 5.0.0 ES
    // esa base: `PlanFireTarget.at(0)` = base + término de deuda, y aquí el término es 0 exacto.
    // Es el pin limpio de la cifra, sin el residual del plan francés que enturbia el escenario A.
    let clasico = dec(&s["fire_number_classic_today"]);
    assert!((clasico - 450_000.0).abs() < 0.01, "número FIRE clásico: {clasico}");
    // **5.0.0**: la serie del objetivo (`fire_target_series`) se retiró — el motor ya no recibe
    // objetivo y el cruce no decide nada. La línea que la app dibuja contra el patrimonio es hoy
    // `needed_capital_curve`, de NIVEL 2 y por tanto ausente en un `?months=`.
    // `.get(...).is_none()` y no `is_null()` (WP A12): sobre un objeto JSON, indexar una clave
    // que NO EXISTE devuelve `Value::Null`, así que un `is_null()` sobre un campo retirado pasa
    // haga lo que haga el servidor — incluido volver a publicarlo. Lo que hay que comprobar es que
    // la CLAVE no está.
    assert!(s.get("fire_target_series").is_none(), "{s}");

    // INVERTIDO en la Ola 5 (#139; capturado en 4.6.0 como 285 / 211.361,91 / 1.094.275,23 con
    // el gasto congelado). Con el gasto indexado al 2,5 % e ingresos planos, este hogar —que
    // ahorra 1.000 €/mes sobre 1.500 de gasto, al 7 % nominal— DEJA DE ALCANZAR el FIRE dentro
    // de 30 años: la señal de producto más dura de la ola, en primera línea del CHANGELOG.
    // Números predichos por la réplica a 50 dígitos ANTES de ejecutar (spike §5.2.2).
    let jub = s["jubilacion_month_index"].clone();
    let nw120 = nw_at(&s, 120);
    let nw360 = nw_at(&s, 360);
    assert!(
        jub.is_null(),
        "un `?months=` no resuelve plan; y con el gasto indexado este hogar tampoco cruzaría: {jub}"
    );
    assert!((nw120 - 181_037.91).abs() < 0.01, "NW(120) predicho: {nw120}");
    assert!((nw360 - 777_970.12).abs() < 0.01, "NW(360) predicho: {nw360}");
}
