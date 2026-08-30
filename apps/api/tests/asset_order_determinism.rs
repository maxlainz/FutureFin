//! Regresión (auditoría del modelo financiero, D34): `ORDER BY sort_index, name` no era un
//! orden TOTAL — dos activos empatados en ambos campos podían salir en orden distinto entre
//! peticiones, y ese orden alimenta `per_asset_series` y el desempate del drenaje. Con
//! `, id ASC` el orden queda determinista: empatados en (sort_index, name), gana el id menor.

mod common;
use common::TestApp;

#[tokio::test]
async fn tied_assets_are_served_in_id_order_every_time() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("orden").await;
    let cat = app.create_category(&owner, "asset", "Cuentas").await;

    // Dos activos IDÉNTICOS en nombre y sort_index (nada lo impide): el único desempate
    // posible es el id.
    let mut ids: Vec<String> = Vec::new();
    for _ in 0..2 {
        let resp = app
            .post_json_with_cookie(
                "/v1/assets",
                serde_json::json!({
                    "category_id": cat,
                    "name": "Fondo",
                    "current_value": "1000",
                    "is_liquid": true
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(resp.status, http::StatusCode::CREATED, "{resp:?}");
        ids.push(resp.json()["id"].as_str().unwrap().to_string());
    }
    ids.sort();

    // Dos GET consecutivos: mismo orden entre sí Y exactamente el orden ascendente de id.
    for _ in 0..2 {
        let resp = app.get_with_cookie("/v1/assets", &owner.cookie).await;
        assert_eq!(resp.status, http::StatusCode::OK);
        let body = resp.json();
        let served: Vec<String> = body
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["name"] == "Fondo")
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(served, ids, "los empatados deben servirse por id ascendente");
    }
}
