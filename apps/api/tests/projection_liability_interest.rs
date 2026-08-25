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
    let fixed = net_worth_series("fixed_payments", Some("3")).await;
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

/// Compatibilidad: en `fixed_payments` el TIN es informativo y el engine lo ignora. La serie de
/// un pasivo con TIN configurado es la misma que la de uno sin él — que es exactamente lo que se
/// prometió al migrar (`DEFAULT 'fixed_payments'`: nadie ve moverse un número al actualizar).
#[tokio::test]
async fn fixed_payments_ignores_the_apr_so_existing_liabilities_do_not_move() {
    let without_apr = net_worth_series("fixed_payments", None).await;
    let with_apr = net_worth_series("fixed_payments", Some("3")).await;

    assert_eq!(without_apr.len(), with_apr.len());
    for (k, (a, b)) in without_apr.iter().zip(with_apr.iter()).enumerate() {
        assert!(
            (a - b).abs() < 0.01,
            "mes {k}: el TIN no puede mover la serie en fixed_payments ({a} vs {b})"
        );
    }
}
