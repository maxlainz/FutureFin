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
    // OLA 4 (#141/#142/#143): este pin NO se movió, y el porqué es la identidad que la ola
    // pinea en el engine (`target_and_crossing_base_agree_on_the_liability_accounting`):
    // `jubilacion_target_net_worth` es la BASE en euros de hoy (el término de deuda viaja
    // aparte en `fire_target_debt_component`), y el cruce (mes 235) cae DESPUÉS del fin del
    // plan (mes ~180), donde término = residual = principal congelado — ahí
    // «líquido ≥ base + término» y el viejo «NW ≥ base» coinciden EXACTAMENTE
    // (liquid − residual = NW). Tampoco hay parpadeo que el latch (#141) congele: tras el
    // cruce el retorno del activo (~5 %/a sobre ~570 k€) supera el déficit de 1.200 €/mes y
    // el patrimonio nunca recae bajo el objetivo. Un cruce DURANTE el plan sí se mueve — eso
    // lo pinean los tests del engine de la Ola 4.
    let target = dec(&s["jubilacion_target_net_worth"]);
    assert!((target - 516_455.6961).abs() < 0.01, "target: {target}");

    // capturado 4.6.0 (#144 default french ya aplicado aquí a mano; verificado inmóvil en la
    // Ola 4 por lo de arriba; #124 no aplica — no hay partidas vencidas):
    let jub = s["jubilacion_month_index"].clone();
    let nw12 = nw_at(&s, 12);
    let nw180 = nw_at(&s, 180);
    let nw360 = nw_at(&s, 360);
    assert_eq!(jub, 235, "jubilacion_month_index capturado: {jub}");
    // R8 (5.0.0) — **el pin de que la mudanza no movió el número.** `jubilacion_month_index`
    // dejó de derivarse del cruce que calcula el handler y pasa a ser el mes EFECTIVO que
    // decide el motor (`ProjectionOutput::retirement_month_index`, traducido a la rejilla
    // publicada). En la estrategia por defecto —`asap`, jubilación por cruce— las tres cifras
    // son la MISMA por construcción: el cruce ES el trigger. Este assert es lo que se rompería
    // si alguien publicara el mes del bucle a pelo (sería 236) o si el cruce-lectura y el
    // trigger dejaran de coincidir en `asap`.
    assert_eq!(
        s["retirement_month_index"], 235,
        "retirement_month_index debe ser el mismo mes efectivo: {}",
        s["retirement_month_index"]
    );
    assert_eq!(
        s["liquid_crossing_month_index"], 235,
        "con `asap` el cruce ES el trigger: {}",
        s["liquid_crossing_month_index"]
    );
    assert_eq!(s["strategy"], "asap", "estrategia por defecto: {}", s["strategy"]);
    assert_eq!(
        s["retirement_trigger"], "liquid_crossing",
        "trigger por defecto: {}",
        s["retirement_trigger"]
    );
    assert!(
        s["jubilacion_absent_reason"].is_null() && s["liquid_crossing_absent_reason"].is_null(),
        "hay objetivo y hay cruce: ninguna razón de ausencia: {s}"
    );
    // Las fases: acumulando desde el mes 0, jubilado desde el mes del cruce. Sobre la serie, no
    // sobre el enum (§C: el invariante es de comportamiento).
    let fases = s["phase_transitions"].as_array().expect("phase_transitions");
    assert_eq!(fases.len(), 2, "acumulación + jubilación: {fases:?}");
    assert_eq!(fases[0]["phase"], "accumulating");
    assert_eq!(fases[0]["month_index"], 0);
    assert_eq!(fases[1]["phase"], "retired");
    assert_eq!(fases[1]["month_index"], 235, "la fase empieza en el mes publicado");
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
    assert!(w(300) > 0.0, "jubilado desde el 235: el mes 300 retira");
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
    // El cruce (235), NW(12) y NW(180) NO se mueven: antes del 235 este hogar no drena, y los
    // meses 1..180 son idénticos con ambas longitudes. El mecanismo exacto está pineado a mano
    // en el engine (`derived_g_rises_along_the_trajectory…`,
    // `the_simulated_withdrawal_also_pays_taxes` — este último sin coste declarado, g=1).
    assert!((nw360 - 677_335.52).abs() < 0.01, "NW(360) capturado: {nw360}");
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

    // a mano: base = 1.500×12/0,04 = 450.000. target(120) = 450.000×1,025^10 = 576.038,04
    // (1,025^10 = 1,280084544196…; la cifra 576.018,10 que llevaba este comentario transponía
    // dígitos del producto — errata detectada en el spike de la Ola 5).
    let base = dec(&s["jubilacion_target_net_worth"]);
    assert!((base - 450_000.0).abs() < 0.01, "base: {base}");
    let ft120 = s["fire_target_series"].as_array().map(|a| a.len());
    assert!(ft120.unwrap_or(0) > 0, "fire_target_series vacío");

    // INVERTIDO en la Ola 5 (#139; capturado en 4.6.0 como 285 / 211.361,91 / 1.094.275,23 con
    // el gasto congelado). Con el gasto indexado al 2,5 % e ingresos planos, este hogar —que
    // ahorra 1.000 €/mes sobre 1.500 de gasto, al 7 % nominal— DEJA DE ALCANZAR el FIRE dentro
    // de 30 años: la señal de producto más dura de la ola, en primera línea del CHANGELOG.
    // Números predichos por la réplica a 50 dígitos ANTES de ejecutar (spike §5.2.2).
    let jub = s["jubilacion_month_index"].clone();
    let nw120 = nw_at(&s, 120);
    let nw360 = nw_at(&s, 360);
    assert!(jub.is_null(), "sin cruce en 360 meses con el gasto indexado: {jub}");
    assert!((nw120 - 181_037.91).abs() < 0.01, "NW(120) predicho: {nw120}");
    assert!((nw360 - 777_970.12).abs() < 0.01, "NW(360) predicho: {nw360}");
}
