//! **Rendimiento neto** de `GET /v1/summary` (`financial_health.net_return_*_annual_pct`).
//!
//! La métrica es el rendimiento anual ESPERADO del patrimonio neto: lo que rinden los activos
//! según la rentabilidad configurada en cada uno, menos el interés de los pasivos que DEVENGAN
//! (#121: modelo con intereses + TIN > 0 + plan vivo — el predicado del engine), sobre
//! `net_worth`. Los tres tests fijan lo que puede romperse en silencio:
//!
//! - los pesos y el lastre de la deuda, con los dos números **calculados a mano** antes de correr;
//! - patrimonio neto ≤ 0 ⇒ los dos campos **no viajan** (un cociente con NW negativo se leería
//!   con el signo cambiado);
//! - `?view=mine` pondera solo lo del usuario — el activo de otro miembro no entra ni en el
//!   numerador ni en el denominador.

mod common;

use chrono::{Datelike, NaiveDate};
use common::TestApp;
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::str::FromStr;

fn dec(v: &Value) -> Decimal {
    Decimal::from_str(
        v.as_str()
            .unwrap_or_else(|| panic!("expected decimal string, got {v:?}")),
    )
    .expect("parse decimal string")
}

async fn server_today(app: &TestApp, cookie: &str) -> NaiveDate {
    let resp = app.get_with_cookie("/v1/history/series", cookie).await;
    NaiveDate::parse_from_str(resp.json()["anchor_date_ymd"].as_str().unwrap(), "%Y-%m-%d").unwrap()
}

/// Activo con rentabilidad esperada opcional (`None` = sin configurar, cuenta como 0 %).
async fn asset(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    name: &str,
    value: &str,
    annual_return_pct: Option<&str>,
) {
    let mut body = json!({
        "category_id": cat, "name": name, "current_value": value, "is_liquid": true
    });
    if let Some(r) = annual_return_pct {
        body["expected_annual_return_percent"] = json!(r);
    }
    let r = app.post_json_with_cookie("/v1/assets", body, cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset {name}: {r:?}");
}

/// Pasivo con plan de pago vigente (fin dentro de 5 años) y TAE.
async fn liability(
    app: &TestApp,
    cookie: &str,
    cat: &str,
    exp_cat: &str,
    label: &str,
    principal: &str,
    apr: &str,
    today: NaiveDate,
) {
    let future = NaiveDate::from_ymd_opt(today.year() + 5, 1, 15)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let l = app
        .post_json_with_cookie(
            "/v1/liabilities",
            // Francés explícito desde #144: el default histórico (`fixed_payments`) rechaza el
            // TIN que este helper necesita declarar. Y desde #121 el numerador solo resta el
            // TIN de lo que DEVENGA — este helper crea planes vivos, así que sigue restando y
            // los esperados históricos (3,5556/1,5251…) no se mueven.
            json!({ "category_id": cat, "expense_category_id": exp_cat, "label": label,
                    "principal": principal, "apr_percent": apr,
                    "repayment_model": "french",
                    "payment_amount": "300", "payment_frequency": "monthly",
                    "payment_end_date": future }),
            cookie,
        )
        .await;
    assert_eq!(l.status, http::StatusCode::CREATED, "liability {label}: {l:?}");
}

async fn set_inflation(app: &TestApp, cookie: &str, pct: &str) {
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            json!({ "annual_inflation_assumption_percent": pct }),
            cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "set inflation: {r:?}");
}

async fn health(app: &TestApp, cookie: &str, uri: &str) -> Value {
    let resp = app.get_with_cookie(uri, cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK, "summary: {resp:?}");
    resp.json()["financial_health"].clone()
}

/// Escenario trabajado a mano:
/// - activos: 100.000 al 5 % (= 5.000 €/año) + 50.000 **sin rentabilidad configurada** (= 0)
/// - pasivo vivo: 60.000 al 3 % de TAE (= 1.800 €/año de interés)
/// - numerador = 5.000 − 1.800 = **3.200 €/año**; patrimonio neto = 150.000 − 60.000 = **90.000 €**
/// - nominal = 100 × 3.200 / 90.000 = 3,5555…% → publicado a 4 decimales: **3,5556 %**
/// - real con inflación 2 % = 100 × (1,0355555…/1,02 − 1) = 1,52505…% → **1,5251 %**
///
/// El activo sin rentabilidad NO se excluye: pesa en el denominador y por eso diluye (con solo
/// los 100.000 al 5 % la cifra sería 100 × 3.200/40.000 = 8 %).
#[tokio::test]
async fn net_return_weights_by_value_and_subtracts_liability_interest() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cartera").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Hipoteca").await;

    let today = server_today(&app, &owner.cookie).await;
    set_inflation(&app, &owner.cookie, "2").await;

    asset(&app, &owner.cookie, &asset_cat, "Fondo", "100000", Some("5")).await;
    asset(&app, &owner.cookie, &asset_cat, "Cuenta", "50000", None).await;
    liability(
        &app,
        &owner.cookie,
        &liab_cat,
        &expense_cat,
        "Hipoteca",
        "60000",
        "3",
        today,
    )
    .await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    assert_eq!(
        dec(&h["net_return_nominal_annual_pct"]),
        Decimal::new(35556, 4),
        "nominal esperado 3,5556 %"
    );
    assert_eq!(
        dec(&h["net_return_real_annual_pct"]),
        Decimal::new(15251, 4),
        "real esperado 1,5251 % (división de factores, no 3,5556 − 2)"
    );
}

/// Con más deuda que activos el cociente no significa nada: los dos campos se **omiten**.
/// 50.000 de activos − 80.000 de pasivo ⇒ patrimonio neto = −30.000.
#[tokio::test]
async fn non_positive_net_worth_omits_both_fields() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let asset_cat = app.create_category(&owner, "asset", "Cartera").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    let today = server_today(&app, &owner.cookie).await;
    asset(&app, &owner.cookie, &asset_cat, "Fondo", "50000", Some("5")).await;
    liability(
        &app,
        &owner.cookie,
        &liab_cat,
        &expense_cat,
        "Préstamo",
        "80000",
        "3",
        today,
    )
    .await;

    let h = health(&app, &owner.cookie, "/v1/summary").await;
    assert!(
        h.get("net_return_nominal_annual_pct").is_none(),
        "con NW ≤ 0 el nominal no debe viajar: {h:?}"
    );
    assert!(
        h.get("net_return_real_annual_pct").is_none(),
        "con NW ≤ 0 el real no debe viajar: {h:?}"
    );
}

/// `?view=mine` pondera **solo** lo del usuario. Alice tiene 100.000 al 6 %; Bob, 100.000 al 0 %.
/// - hogar: 6.000 / 200.000 = **3 %**
/// - alice: 6.000 / 100.000 = **6 %** (el activo de Bob no entra ni arriba ni abajo)
///
/// Inflación 0 para que el real coincida con el nominal y el test mida una sola cosa.
#[tokio::test]
async fn view_mine_ignores_another_members_asset_in_both_terms() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;

    let asset_cat = app.create_category(&owner, "asset", "Cartera").await;
    set_inflation(&app, &owner.cookie, "0").await;

    asset(&app, &owner.cookie, &asset_cat, "Fondo Alice", "100000", Some("6")).await;
    asset(&app, &bob.cookie, &asset_cat, "Cuenta Bob", "100000", Some("0")).await;

    let hogar = health(&app, &owner.cookie, "/v1/summary").await;
    assert_eq!(dec(&hogar["net_return_nominal_annual_pct"]), Decimal::from(3));

    let mine = health(&app, &owner.cookie, "/v1/summary?view=mine").await;
    assert_eq!(dec(&mine["net_return_nominal_annual_pct"]), Decimal::from(6));
    assert_eq!(
        dec(&mine["net_return_real_annual_pct"]),
        dec(&mine["net_return_nominal_annual_pct"]),
        "con inflación 0 el real es exactamente el nominal"
    );
}

/// #121 + #145, a mano. Activo 100.000 € al 5 % (numerador 5.000) + pasivo `french` de 50.000 €
/// al TIN 5 % cuyo plan venció AYER. NW = 50.000 en ambos casos (el saldo vencido sigue siendo
/// deuda, #145) — lo único que cambia es el numerador:
/// - plan vencido ⇒ no devenga ⇒ 5.000 − 0 = 5.000 ⇒ **10,0000 %**;
/// - el MISMO pasivo con el plan vivo ⇒ 5.000 − 2.500 = 2.500 ⇒ **5,0000 %**.
/// Hasta 4.6.0 el KPI restaba el TIN sin mirar el plan (y #145 ni siquiera dejaba ver la fila):
/// las dos mitades del assert son la evidencia de que el predicado distingue los dos estados.
#[tokio::test]
async fn a_liability_whose_plan_expired_no_longer_drags_the_net_return() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cartera").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Hipoteca").await;

    set_inflation(&app, &owner.cookie, "0").await;
    let today = server_today(&app, &owner.cookie).await;
    let yesterday = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
    let future = (today + chrono::Duration::days(365)).format("%Y-%m-%d").to_string();

    asset(&app, &owner.cookie, &asset_cat, "Fondo", "100000", Some("5")).await;
    let created = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": expense_cat,
                    "label": "Vencida", "principal": "50000",
                    "repayment_model": "french", "apr_percent": "5",
                    "payment_amount": "300", "payment_frequency": "monthly",
                    "payment_end_date": yesterday }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");
    let id = created.json()["id"].as_str().unwrap().to_string();

    let root = app.get_with_cookie("/v1/summary", &owner.cookie).await.json();
    assert_eq!(
        dec(&root["net_worth"]),
        Decimal::from(50_000),
        "el saldo vencido sigue restando (#145)"
    );
    assert_eq!(
        dec(&root["financial_health"]["net_return_nominal_annual_pct"]),
        Decimal::new(100_000, 4),
        "sin devengo: 5.000/50.000 = 10,0000 %"
    );

    // La otra mitad de la evidencia: el MISMO pasivo, con el plan vivo, vuelve a costar.
    let patched = app
        .patch_json_with_cookie(
            &format!("/v1/liabilities/{id}"),
            json!({ "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(patched.status, http::StatusCode::OK, "{patched:?}");
    let h = health(&app, &owner.cookie, "/v1/summary").await;
    assert_eq!(
        dec(&h["net_return_nominal_annual_pct"]),
        Decimal::new(50_000, 4),
        "con plan vivo: (5.000 − 2.500)/50.000 = 5,0000 %"
    );
}

/// #121 + #144, a mano: el préstamo sin intereses nunca arrastra el rendimiento. Activo
/// 300.000 € al 4 % (12.000) + pasivo sin intereses de 100.000 € (sin TIN — post-#144 no puede
/// llevarlo). NW = 200.000 ⇒ **6,0000 %**. Es el escenario del issue (que publicaba 3,50 % con
/// un TIN 5 % «informativo») reexpresado en el catálogo nuevo: la brecha de los pasivos
/// fixed+TIN la cerró la migración de #144 desde el otro lado (convirtiéndolos a `french`, la
/// proyección empieza a cobrar); este test pinea el residuo.
#[tokio::test]
async fn the_zero_interest_loan_never_drags_the_net_return() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cartera").await;
    let expense_cat = app.create_category(&owner, "expense", "Gastos").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamo").await;

    set_inflation(&app, &owner.cookie, "0").await;
    let today = server_today(&app, &owner.cookie).await;
    let future = (today + chrono::Duration::days(365 * 5)).format("%Y-%m-%d").to_string();

    asset(&app, &owner.cookie, &asset_cat, "Fondo", "300000", Some("4")).await;
    let created = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": liab_cat, "expense_category_id": expense_cat,
                    "label": "Sin intereses", "principal": "100000",
                    "repayment_model": "fixed_payments",
                    "payment_amount": "500", "payment_frequency": "monthly",
                    "payment_end_date": future }),
            &owner.cookie,
        )
        .await;
    assert_eq!(created.status, http::StatusCode::CREATED, "{created:?}");

    let root = app.get_with_cookie("/v1/summary", &owner.cookie).await.json();
    assert_eq!(dec(&root["net_worth"]), Decimal::from(200_000));
    assert_eq!(
        dec(&root["financial_health"]["net_return_nominal_annual_pct"]),
        Decimal::new(60_000, 4),
        "12.000/200.000 = 6,0000 % — la deuda sin intereses diluye, no resta"
    );
}
