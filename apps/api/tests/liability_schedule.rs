//! `GET /v1/liabilities/{id}/schedule` (4.5.0): calendario de amortización, interés devengado y
//! mes de extinción.
//!
//! Lo que estos tests protegen es un número que hasta ahora **no existía en ninguna superficie**:
//! el motor calculaba el principal de cierre de cada pasivo hasta 840 veces por request y
//! `ProjectionOutput` no lo publicaba, así que «¿cuánto pago de intereses?» y «¿cuándo termino la
//! hipoteca?» eran incontestables. Cada aserción numérica de aquí se **predijo antes de correr el
//! test** con aritmética decimal a 50 dígitos y se contrastó contra la fórmula cerrada de
//! amortización francesa; están en los doc-comments de cada caso.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::{LoggedInOwner, ResponseParts, TestApp};
use serde_json::json;

fn dec(v: &serde_json::Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("esperaba un string decimal, llegó {v}"))
        .parse()
        .expect("decimal")
}

async fn categories(app: &TestApp, owner: &LoggedInOwner) -> (String, String) {
    (
        app.create_category(owner, "liability", "Préstamos").await,
        app.create_category(owner, "expense", "Cuotas").await,
    )
}

async fn post_liability(
    app: &TestApp,
    owner: &LoggedInOwner,
    cat: &str,
    exp_cat: &str,
    extra: serde_json::Value,
) -> ResponseParts {
    let mut body = json!({
        "category_id": cat,
        "expense_category_id": exp_cat,
        "label": "Hipoteca",
    });
    for (k, v) in extra.as_object().expect("extra must be an object") {
        body[k] = v.clone();
    }
    app.post_json_with_cookie("/v1/liabilities", body, &owner.cookie)
        .await
}

/// Crea un pasivo y devuelve su id.
async fn mk_liability(app: &TestApp, owner: &LoggedInOwner, extra: serde_json::Value) -> String {
    let (cat, exp_cat) = categories(app, owner).await;
    let r = post_liability(app, owner, &cat, &exp_cat, extra).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

fn assert_bad_request_code(r: &ResponseParts, code: &str) {
    assert_eq!(
        r.status,
        http::StatusCode::BAD_REQUEST,
        "esperaba 400 {code}: {r:?}"
    );
    let body = r.json();
    assert_eq!(body["code"], code, "código inesperado: {body}");
}

// ---------------------------------------------------------------------------
// 1. Amortización francesa contra la fórmula cerrada
// ---------------------------------------------------------------------------

/// **Hipoteca de 100.000 € al 3 % con 500 €/mes.** i = 3/1200 = 0,0025 exacto; la recurrencia es
/// `P' = P·1,0025 − 500`.
///
/// PREDICCIONES (verificadas a 50 dígitos con aritmética decimal exacta ANTES de correr el test,
/// y coincidentes con el test del engine `french_extinction_at_month_278`, que llega al mismo 278
/// por la vía completamente distinta de simular el patrimonio entero):
/// - `payoff_month_index` = **278**. Sin intereses serían 200 meses: los 78 de diferencia son
///   exactamente lo que el modelo pre-4.2.0 no sabía contar.
/// - `total_interest_remaining` = **38.802,7999 €**
/// - `total_to_pay` = **138.802,7999 €** (= 100.000 + el interés)
/// - mes 1: interés **250,0000 €** (100.000 × 0,0025), principal **250,0000 €**, cierre
///   **99.750,0000 €** — exacto, sin tolerancia.
/// - `months_total` = 278 y la ventana por defecto publica 12.
#[tokio::test]
async fn french_schedule_matches_the_closed_form_annuity() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = mk_liability(
        &app,
        &owner,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "apr_percent": "3",
            "payment_amount": "500",
            "payment_frequency": "monthly",
        }),
    )
    .await;

    let r = app
        .get_with_cookie(&format!("/v1/liabilities/{id}/schedule"), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();

    assert_eq!(b["payoff_month_index"], 278);
    assert!(b["payoff_absent_reason"].is_null(), "{b}");
    assert_eq!(b["months_total"], 278);
    assert_eq!(b["repayment_model"], "french");
    assert_eq!(dec(&b["opening_principal"]), 100_000.0);
    assert_eq!(dec(&b["final_principal"]), 0.0);
    assert_eq!(dec(&b["monthly_payment"]), 500.0);

    let interes = dec(&b["total_interest_remaining"]);
    assert!(
        (interes - 38_802.7999).abs() < 0.05,
        "interés total esperado ≈ 38.802,80 €, obtenido {interes}"
    );
    let total = dec(&b["total_to_pay"]);
    assert!(
        (total - 138_802.7999).abs() < 0.05,
        "total a pagar esperado ≈ 138.802,80 €, obtenido {total}"
    );
    // La identidad que hace comprobable la respuesta con una resta.
    assert!(
        (total - (100_000.0 + interes)).abs() < 0.01,
        "total a pagar = principal + interés"
    );

    // Mes 1, exacto.
    let m1 = &b["months"][0];
    assert_eq!(m1["month_index"], 1);
    assert_eq!(dec(&m1["opening_principal"]), 100_000.0);
    assert_eq!(dec(&m1["interest_accrued"]), 250.0);
    assert_eq!(dec(&m1["principal_repaid"]), 250.0);
    assert_eq!(dec(&m1["payment"]), 500.0);
    assert_eq!(dec(&m1["closing_principal"]), 99_750.0);

    // Ventana por defecto: 12 meses, y el aviso de que hay más.
    assert_eq!(b["months"].as_array().unwrap().len(), 12);
    assert_eq!(b["window_from_month_index"], 1);
    assert_eq!(b["window_months"], 12);
    assert_eq!(b["window_truncated"], true);

    // El resumen anual cubre el préstamo ENTERO aunque la ventana no: es lo que hace legible una
    // hipoteca de 23 años sin servir 278 filas.
    let years = b["years"].as_array().unwrap();
    let meses_en_years: u64 = years
        .iter()
        .map(|y| y["months_count"].as_u64().unwrap())
        .sum();
    assert_eq!(meses_en_years, 278, "el resumen anual cubre todo: {years:?}");
    let interes_years: f64 = years.iter().map(|y| dec(&y["interest_accrued"])).sum();
    assert!(
        (interes_years - interes).abs() < 0.05,
        "Σ interés por año debe cuadrar con el total: {interes_years} vs {interes}"
    );
    // El último año cierra a cero: es el año de la extinción.
    assert_eq!(dec(&years.last().unwrap()["closing_principal"]), 0.0);

    // La identidad contable, mes a mes, sobre la ventana servida.
    for m in b["months"].as_array().unwrap() {
        assert!(
            (dec(&m["payment"]) - (dec(&m["interest_accrued"]) + dec(&m["principal_repaid"])))
                .abs()
                < 0.0001,
            "cuota = interés + principal, mes {}",
            m["month_index"]
        );
    }
}

/// La ventana solo recorta lo **publicado**; los agregados describen siempre el préstamo entero.
/// Un «interés total» que dependiera de cuántos meses pidió mirar el llamante sería una cifra
/// distinta en cada llamada.
///
/// PREDICCIÓN: pidiendo los meses 271..278 llegan 8 filas (el calendario acaba en el 278), la
/// última con `closing_principal` 0 y cuota **parcial** (302,7999 € < 500), y `total_to_pay` sigue
/// valiendo 138.802,7999 €.
#[tokio::test]
async fn the_window_never_moves_the_aggregates() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = mk_liability(
        &app,
        &owner,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "apr_percent": "3",
            "payment_amount": "500",
            "payment_frequency": "monthly",
        }),
    )
    .await;

    let r = app
        .get_with_cookie(
            &format!("/v1/liabilities/{id}/schedule?from_month_index=271&months=24"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();

    let months = b["months"].as_array().unwrap();
    assert_eq!(months.len(), 8, "solo quedan 8 meses de calendario");
    assert_eq!(months[0]["month_index"], 271);
    let ultimo = months.last().unwrap();
    assert_eq!(ultimo["month_index"], 278);
    assert_eq!(dec(&ultimo["closing_principal"]), 0.0);
    let ultima_cuota = dec(&ultimo["payment"]);
    assert!(
        (ultima_cuota - 302.7999).abs() < 0.05,
        "última cuota parcial esperada ≈ 302,80 €, obtenida {ultima_cuota}"
    );

    // Agregados intactos.
    assert_eq!(b["payoff_month_index"], 278);
    assert_eq!(b["months_total"], 278);
    assert!((dec(&b["total_to_pay"]) - 138_802.7999).abs() < 0.05);
    assert_eq!(
        b["window_truncated"], true,
        "8 de 278 meses siguen siendo un recorte"
    );

    // Cotas de la ventana.
    let malo = app
        .get_with_cookie(
            &format!("/v1/liabilities/{id}/schedule?months=1000"),
            &owner.cookie,
        )
        .await;
    assert_bad_request_code(&malo, "schedule_window_out_of_range");
}

/// `fixed_payments` no devenga: interés 0 en todos los meses y el total a pagar es el principal.
/// Es la garantía de que el calendario de un pasivo histórico sigue contando la historia de
/// siempre. (Ajustado en 4.7.0/#144: ya no se puede declarar un TIN «informativo» sobre este
/// modelo — el alta lo rechaza y la columna dejó de ser el default, ahora es `french` —, así que
/// el caso «con TIN o sin él» quedó irrepresentable y este test lo prueba sin TIN.)
///
/// PREDICCIÓN: 1.200 € a 100 €/mes ⇒ extinción en el mes **12**, interés **0**, total **1.200 €**.
#[tokio::test]
async fn fixed_payments_schedule_charges_no_interest() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = mk_liability(
        &app,
        &owner,
        json!({
            "principal": "1200",
            "payment_amount": "100",
            "payment_frequency": "monthly",
        }),
    )
    .await;

    let r = app
        .get_with_cookie(
            &format!("/v1/liabilities/{id}/schedule?months=480"),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["repayment_model"], "fixed_payments");
    assert_eq!(b["payoff_month_index"], 12);
    assert_eq!(dec(&b["total_interest_remaining"]), 0.0);
    assert_eq!(dec(&b["total_to_pay"]), 1_200.0);
    assert_eq!(b["window_truncated"], false, "12 de 12 meses: no hay recorte");
    for m in b["months"].as_array().unwrap() {
        assert_eq!(dec(&m["interest_accrued"]), 0.0);
        assert_eq!(dec(&m["payment"]), 100.0);
    }
}

// ---------------------------------------------------------------------------
// 2. Las cuatro razones por las que no hay mes de extinción
// ---------------------------------------------------------------------------

/// Cada razón tiene un **remedio distinto**, y por eso no se colapsan en «no se puede calcular».
/// Un `payoff_month_index` a null sin razón invitaría a inventarse la causa.
///
/// PREDICCIONES:
/// - sin cuota ⇒ `no_payment_plan`, calendario **vacío** y el principal intacto.
/// - `interest_only` de 80.000 a 400 €/mes ⇒ `payment_does_not_reduce_principal`, saldo final
///   80.000 y **todo lo pagado es interés** (la cuota YA es el interés: devengarlo otra vez lo
///   cobraría dos veces).
/// - francés de 100.000 al 3 % con cuota **280 €** (por debajo de los 250 € del primer devengo
///   solo por 30 €) ⇒ baja, pero en 840 meses solo llega a 14.262,43 € ⇒ `not_within_horizon`.
#[tokio::test]
async fn payoff_absent_reasons_are_distinguishable() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let (cat, exp_cat) = categories(&app, &owner).await;

    // (1) Sin plan de pago.
    let sin_plan = post_liability(&app, &owner, &cat, &exp_cat, json!({ "principal": "50000" }))
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = app
        .get_with_cookie(
            &format!("/v1/liabilities/{sin_plan}/schedule"),
            &owner.cookie,
        )
        .await
        .json();
    assert!(b["payoff_month_index"].is_null());
    assert_eq!(b["payoff_absent_reason"], "no_payment_plan");
    assert_eq!(b["months"].as_array().unwrap().len(), 0);
    assert_eq!(b["months_total"], 0);
    assert_eq!(dec(&b["final_principal"]), 50_000.0);
    assert_eq!(dec(&b["total_to_pay"]), 0.0);

    // (2) Solo intereses: el principal no se mueve nunca.
    let solo_int = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "80000",
            "repayment_model": "interest_only",
            "apr_percent": "6",
            "payment_amount": "400",
            "payment_frequency": "monthly",
        }),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = app
        .get_with_cookie(
            &format!("/v1/liabilities/{solo_int}/schedule"),
            &owner.cookie,
        )
        .await
        .json();
    assert_eq!(
        b["payoff_absent_reason"], "payment_does_not_reduce_principal",
        "{b}"
    );
    assert_eq!(dec(&b["final_principal"]), 80_000.0);
    assert_eq!(
        dec(&b["total_interest_remaining"]),
        dec(&b["total_to_pay"]),
        "en interest_only todo lo que se paga es interés"
    );

    // (3) Baja, pero no dentro de los 840 meses del calendario.
    let lento = post_liability(
        &app,
        &owner,
        &cat,
        &exp_cat,
        json!({
            "principal": "100000",
            "repayment_model": "french",
            "apr_percent": "3",
            "payment_amount": "280",
            "payment_frequency": "monthly",
        }),
    )
    .await
    .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = app
        .get_with_cookie(&format!("/v1/liabilities/{lento}/schedule"), &owner.cookie)
        .await
        .json();
    assert_eq!(b["payoff_absent_reason"], "not_within_horizon", "{b}");
    assert_eq!(b["months_total"], 840);
    let restante = dec(&b["final_principal"]);
    assert!(
        (restante - 14_262.4313).abs() < 1.0,
        "saldo a los 70 años esperado ≈ 14.262,43 €, obtenido {restante}"
    );
}

/// Un pasivo con el plan **ya vencido** no existe para las lecturas (contrato «reads never
/// mutate»: se filtra en `/v1/liabilities`, `/summary`, `/budget`, `/assets` y `/projection`), así
/// que su calendario es un **404** y no una respuesta vacía que se leería como «no debes nada».
#[tokio::test]
async fn an_expired_liability_with_balance_serves_a_frozen_schedule() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let id = mk_liability(
        &app,
        &owner,
        json!({
            "principal": "1000",
            "payment_amount": "100",
            "payment_frequency": "monthly",
            "payment_end_date": "2030-01-31",
        }),
    )
    .await;

    // Vive mientras el plan está vigente…
    let vivo = app
        .get_with_cookie(&format!("/v1/liabilities/{id}/schedule"), &owner.cookie)
        .await;
    assert_eq!(vivo.status, http::StatusCode::OK, "{vivo:?}");

    // …y al vencer CON SALDO VIVO sigue existiendo (#145, INVERTIDO en 4.7.0: antes era 404).
    // El calendario se sirve CONGELADO: cero meses, principal final = de apertura, interés 0 y
    // `payoff_absent_reason: no_payment_plan` — nada devenga sin plan. Se fuerza en BD porque
    // la API no deja escribir una fecha pasada.
    sqlx::query("UPDATE liabilities SET payment_end_date = DATE '2020-01-31' WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&id).unwrap())
        .execute(&app.pool)
        .await
        .expect("vencer el pasivo");

    let congelado = app
        .get_with_cookie(&format!("/v1/liabilities/{id}/schedule"), &owner.cookie)
        .await;
    assert_eq!(congelado.status, http::StatusCode::OK, "{congelado:?}");
    let b = congelado.json();
    assert_eq!(b["payoff_absent_reason"], "no_payment_plan", "{b}");
    assert_eq!(b["months"].as_array().unwrap().len(), 0);
    assert_eq!(b["final_principal"], "1000.0000", "el saldo queda congelado");
    assert_eq!(b["total_interest_remaining"], "0.0000", "sin plan no hay devengo");

    // El vencido Y SALDADO sí es un 404: esa deuda se extinguió de verdad.
    sqlx::query("UPDATE liabilities SET principal = 0 WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&id).unwrap())
        .execute(&app.pool)
        .await
        .expect("saldar el pasivo");
    let muerto = app
        .get_with_cookie(&format!("/v1/liabilities/{id}/schedule"), &owner.cookie)
        .await;
    assert_eq!(
        muerto.status,
        http::StatusCode::NOT_FOUND,
        "el vencido y saldado no existe para las lecturas: {muerto:?}"
    );

    // Y un id inexistente tampoco filtra su ausencia de otra forma.
    let fantasma = app
        .get_with_cookie(
            "/v1/liabilities/00000000-0000-0000-0000-000000000000/schedule",
            &owner.cookie,
        )
        .await;
    assert_eq!(fantasma.status, http::StatusCode::NOT_FOUND);
}

/// El plan de pago que se acaba antes que la deuda es su propia razón: `payment_end_date` corta el
/// calendario y deja un saldo vivo que ya no se amortiza (ni devenga: sin plan activo el pasivo es
/// una resta constante al patrimonio). Es una respuesta útil —«tu plan acaba con X € pendientes»—
/// que sin código sería un `null` mudo.
#[tokio::test]
async fn a_plan_that_ends_before_the_debt_says_so() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    // `payment_end_date` a ~3 meses vista: la API exige que no esté en el pasado.
    let fin = (chrono::Utc::now().date_naive() + chrono::Duration::days(80))
        .format("%Y-%m-%d")
        .to_string();
    let id = mk_liability(
        &app,
        &owner,
        json!({
            "principal": "50000",
            "repayment_model": "french",
            "apr_percent": "5",
            "payment_amount": "500",
            "payment_frequency": "monthly",
            "payment_end_date": fin,
        }),
    )
    .await;

    let b = app
        .get_with_cookie(&format!("/v1/liabilities/{id}/schedule"), &owner.cookie)
        .await
        .json();
    assert_eq!(
        b["payoff_absent_reason"], "payment_plan_ends_before_payoff",
        "{b}"
    );
    assert!(b["payoff_month_index"].is_null());
    let meses = b["months_total"].as_u64().unwrap();
    assert!(
        (1..=4).contains(&meses),
        "el plan dura tres meses largos, llegaron {meses}"
    );
    assert!(
        dec(&b["final_principal"]) > 48_000.0,
        "queda casi toda la deuda viva: {b}"
    );
}

// ---------------------------------------------------------------------------
// 3. Scope
// ---------------------------------------------------------------------------

/// `?view=mine` es un filtro de scope, no una frontera de autorización — pero el calendario tiene
/// que respetarlo igual que el listado, o `view=mine` devolvería el calendario de la deuda de otro.
#[tokio::test]
async fn the_schedule_respects_the_ledger_scope() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;
    let id = mk_liability(
        &app,
        &owner,
        json!({
            "principal": "1000",
            "payment_amount": "100",
            "payment_frequency": "monthly",
        }),
    )
    .await;

    // El hogar lo ve — pidiéndolo EXPLÍCITAMENTE (5.0.0, R2: el default es `mine`).
    let hogar = app
        .get_with_cookie(
            &format!("/v1/liabilities/{id}/schedule?view=household"),
            &member.cookie,
        )
        .await;
    assert_eq!(hogar.status, http::StatusCode::OK, "{hogar:?}");
    assert_eq!(hogar.json()["view"], "household");

    // `mine` de otro usuario, no: el pasivo es de alice.
    let mio = app
        .get_with_cookie(
            &format!("/v1/liabilities/{id}/schedule?view=mine"),
            &member.cookie,
        )
        .await;
    assert_eq!(mio.status, http::StatusCode::NOT_FOUND, "{mio:?}");

    // Y el de la dueña sí, con el eco de la vista aplicada.
    let suyo = app
        .get_with_cookie(
            &format!("/v1/liabilities/{id}/schedule?view=mine"),
            &owner.cookie,
        )
        .await;
    assert_eq!(suyo.status, http::StatusCode::OK, "{suyo:?}");
    assert_eq!(suyo.json()["view"], "mine");
}
