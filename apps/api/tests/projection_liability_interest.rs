//! El `repayment_model` del pasivo llega de verdad al engine desde `GET /v1/projection/series`
//! (4.2.0, parte 2 del cableado).
//!
//! Antes de esta parte el handler mapeaba `FixedPayments`/`apr_percent: None` a pelo con un TODO:
//! el engine sabía devengar intereses pero la API nunca se los pedía. Un test que solo mirase el
//! ledger no habría notado nada — el pasivo se guardaba `french` y la proyección seguía siendo la
//! histórica. Por eso el sujeto de estos tests es la **serie**, no la fila.
//!
//! Cada escenario vive en su propia instalación (`TestApp::spawn()` crea schema + installation
//! frescos), que es como el repo aísla comparaciones entre configuraciones distintas.
//!
//! Requiere un Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::TestApp;
use serde_json::json;

/// Monta una instalación idéntica en todo salvo el modelo/TIN del pasivo y devuelve la serie de
/// patrimonio de los primeros 24 meses (densidad `monthly` por defecto: `points[k]` es el mes k).
async fn net_worth_series(model: &str, apr: Option<&str>) -> Vec<f64> {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cartera").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamos").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({
                "category_id": asset_cat,
                "name": "Cuenta",
                "current_value": "200000",
                "is_liquid": true,
                "expected_annual_return_percent": "0",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "asset: {r:?}");

    let mut body = json!({
        "category_id": liab_cat,
        "expense_category_id": exp_cat,
        "label": "Hipoteca",
        "principal": "100000",
        "repayment_model": model,
        "payment_amount": "500",
        "payment_frequency": "monthly",
        // Lejano: el plan sigue vivo durante todo el tramo que miramos.
        "payment_end_date": "2060-01-01",
    });
    if let Some(a) = apr {
        body["apr_percent"] = json!(a);
    }
    let r = app.post_json_with_cookie("/v1/liabilities", body, &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::CREATED, "liability {model}: {r:?}");

    let r = app
        .get_with_cookie("/v1/projection/series?months=24", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "series {model}: {r:?}");
    let body = r.json();
    let points = body["points"].as_array().expect("points").clone();
    assert_eq!(points[12]["month_index"].as_u64(), Some(12), "densidad monthly esperada");
    points
        .iter()
        .map(|p| p["net_worth"].as_f64().expect("net_worth f64"))
        .collect()
}

/// El discriminador: con el MISMO principal, la MISMA cuota y el MISMO TIN, un pasivo `french`
/// deja menos patrimonio que uno `fixed_payments`, porque parte de cada cuota se va en intereses
/// en vez de amortizar. Si el handler siguiera mandando `FixedPayments` al engine, las dos series
/// serían idénticas y este test fallaría.
#[tokio::test]
async fn a_french_liability_projects_below_an_identical_fixed_payments_one() {
    // El gemelo sin intereses ya no puede declarar TIN (#144, apr_forbidden_for_model) — y no
    // le hace falta: el engine lo ignoraba, así que su serie es la misma que la de siempre.
    let fixed = net_worth_series("fixed_payments", None).await;
    let french = net_worth_series("french", Some("3")).await;

    assert!(
        french[12] < fixed[12],
        "mes 12: french {} debería quedar por debajo de fixed_payments {}",
        french[12],
        fixed[12]
    );
    // Y el mes 0 (foto de hoy) es el mismo: el modelo describe el FUTURO, no el saldo actual.
    assert!(
        (french[0] - fixed[0]).abs() < 0.01,
        "el punto de partida no puede depender del modelo: {} vs {}",
        french[0],
        fixed[0]
    );
}

/// INVERTIDO en 4.7.0 (#144). Hasta 4.6.0 este test pineaba «en `fixed_payments` el TIN es
/// informativo y el engine lo ignora» — el estado que la auditoría llamó préstamo gratis
/// silencioso: un número guardado que no movía nada. Desde la Ola 3 ese estado es
/// **irrepresentable**: el alta lo rechaza (`apr_forbidden_for_model`) y la migración firmada
/// convirtió o anuló las filas existentes. Lo que queda por pinear es la puerta.
#[tokio::test]
async fn fixed_payments_with_an_apr_is_now_unrepresentable() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab_cat = app.create_category(&owner, "liability", "Préstamos").await;
    let exp_cat = app.create_category(&owner, "expense", "Cuotas").await;

    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({
                "category_id": liab_cat,
                "expense_category_id": exp_cat,
                "label": "Hipoteca",
                "principal": "100000",
                "repayment_model": "fixed_payments",
                "apr_percent": "3",
                "payment_amount": "500",
                "payment_frequency": "monthly",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    let msg = r.json()["message"].as_str().unwrap_or_default().to_string();
    assert!(msg.starts_with("apr_forbidden_for_model"), "{msg}");
}
