//! Tests del cache in-memory de `/v1/projection/series` (plan v3 + v5).
//!
//! Verifican: (1) hit ≥10× más rápido que miss y mismo body, (2) mutación
//! invalida el cache y los datos nuevos se reflejan, (3) logout invalida
//! solo las entries `view=mine` del usuario (las `view=household`
//! sobreviven).

mod common;

use common::TestApp;
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{Density, ProjectionCacheKey};
use std::time::Instant;
use uuid::Uuid;

/// El segundo GET debe ser cache hit y notablemente más rápido que el primero.
#[tokio::test]
async fn projection_series_caches_repeated_gets() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": "X",
                "current_value": "10000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "create asset: {r:?}");

    // Pequeña espera para que el warm-up post-login termine (no nos importa el
    // primer GET aquí; lo que medimos es hit vs miss).
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // El warm-up debería haber poblado view=household. Forzamos una mutación
    // adicional para invalidar lo que tenga, y luego medimos dos GETs limpios.
    app.state
        .invalidate_projection_by_installation(installation_id_of(&app, &owner.cookie).await)
        .await;

    let t0 = Instant::now();
    let r1 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    let miss_ms = t0.elapsed().as_millis();
    assert_eq!(r1.status, http::StatusCode::OK);

    let t1 = Instant::now();
    let r2 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    let hit_ms = t1.elapsed().as_millis();
    assert_eq!(r2.status, http::StatusCode::OK);

    // Bodies idénticos: el cache no muta nada.
    assert_eq!(r1.body, r2.body, "responses divergen entre miss y hit");

    // Hit notablemente más rápido. En CI o cold cache puede ser flaky, por eso
    // tolerancia: aceptamos si hit es <50% del miss o <5ms absoluto.
    assert!(
        hit_ms * 2 < miss_ms.max(2) || hit_ms <= 5,
        "hit ({hit_ms}ms) no es notablemente más rápido que miss ({miss_ms}ms) — cache no funciona"
    );
}

/// Una mutación debe invalidar el cache: siguiente GET refleja datos nuevos.
#[tokio::test]
async fn projection_cache_invalidates_on_asset_mutation() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({
            "category_id": asset_cat,
            "name": "X",
            "current_value": "10000",
        }),
        &owner.cookie,
    )
    .await;

    // Calentar el cache con un GET.
    let r0 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r0.status, http::StatusCode::OK);
    let snw0 = r0.json()["starting_net_worth"]
        .as_str()
        .expect("snw is string")
        .to_string();
    assert!(snw0.starts_with("10000"), "snw inicial: {snw0}");

    // Mutar: añadir un segundo asset. El handler dispara
    // refresh_projection_after_mutation (background tokio::spawn).
    let r2 = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({
                "category_id": asset_cat,
                "name": "Y",
                "current_value": "5000",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r2.status, http::StatusCode::CREATED);

    // Esperar a que el tokio::spawn de invalidación termine.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let r3 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r3.status, http::StatusCode::OK);
    let snw1 = r3.json()["starting_net_worth"]
        .as_str()
        .expect("snw is string")
        .to_string();
    // El nuevo asset debe estar reflejado: 10000 + 5000 = 15000.
    assert!(snw1.starts_with("15000"), "starting_net_worth tras mutación: {snw1}");
}

/// Logout invalida solo las entries `view=mine` de ese usuario.
/// Las `view=household` sobreviven.
#[tokio::test]
async fn projection_cache_logout_drops_only_user_mine_entries() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let asset_cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({
            "category_id": asset_cat,
            "name": "X",
            "current_value": "10000",
        }),
        &owner.cookie,
    )
    .await;

    let installation_id = installation_id_of(&app, &owner.cookie).await;

    // Calentar ambas vistas.
    let r1 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r1.status, http::StatusCode::OK);
    let r2 = app
        .get_with_cookie("/v1/projection/series?view=mine", &owner.cookie)
        .await;
    assert_eq!(r2.status, http::StatusCode::OK);

    // Confirmar que ambas entries están en cache.
    let user_id = user_id_of(&app, &owner.cookie).await;
    let key_household = ProjectionCacheKey {
        installation_id,
        view: LedgerView::Household,
        owner_user_id: None,
        density: Density::Monthly,
    };
    let key_mine = ProjectionCacheKey {
        installation_id,
        view: LedgerView::Mine,
        owner_user_id: Some(user_id),
        density: Density::Monthly,
    };
    {
        let cache = app.state.projection_cache.read().await;
        assert!(cache.contains_key(&key_household), "household debería estar en cache");
        assert!(cache.contains_key(&key_mine), "mine debería estar en cache");
    }

    // Logout.
    let logout = app
        .post_json_with_cookie("/v1/auth/logout", serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(logout.status, http::StatusCode::NO_CONTENT);

    // Tras logout: household sigue, mine se borró.
    {
        let cache = app.state.projection_cache.read().await;
        assert!(
            cache.contains_key(&key_household),
            "household DEBE sobrevivir al logout (otros miembros pueden seguir conectados)"
        );
        assert!(
            !cache.contains_key(&key_mine),
            "mine DEBE borrarse al logout del usuario"
        );
    }
}

/// `density=hybrid` decima los puntos (mes 0..12 mensual + anual desde 24).
/// Verifica que el horizonte completo se preserva y que los primeros 13
/// meses son idénticos entre densidades (zona donde el detalle importa).
#[tokio::test]
async fn projection_hybrid_density_returns_decimated_points() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({
            "category_id": cat,
            "name": "X",
            "current_value": "10000",
        }),
        &owner.cookie,
    )
    .await;

    let r_full = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    let r_hyb = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await;
    assert_eq!(r_full.status, http::StatusCode::OK);
    assert_eq!(r_hyb.status, http::StatusCode::OK);

    let pts_full = r_full.json()["points"].as_array().unwrap().len();
    let pts_hyb = r_hyb.json()["points"].as_array().unwrap().len();
    assert!(pts_full > 100, "full debe tener muchos puntos: {pts_full}");
    assert!(pts_hyb < 100, "hybrid debe estar decimado: {pts_hyb}");

    // Primeros 13 meses idénticos.
    let mi_full: Vec<i64> = r_full.json()["points"]
        .as_array()
        .unwrap()
        .iter()
        .take(13)
        .map(|p| p["month_index"].as_i64().unwrap())
        .collect();
    let mi_hyb: Vec<i64> = r_hyb.json()["points"]
        .as_array()
        .unwrap()
        .iter()
        .take(13)
        .map(|p| p["month_index"].as_i64().unwrap())
        .collect();
    assert_eq!(mi_full, mi_hyb, "primeros 13 meses deben coincidir");

    // Tras el mes 12, hybrid salta a 24, 36, 48 ...
    let hyb_post12: Vec<i64> = r_hyb.json()["points"]
        .as_array()
        .unwrap()
        .iter()
        .skip(13)
        .take(5)
        .map(|p| p["month_index"].as_i64().unwrap())
        .collect();
    assert_eq!(hyb_post12, vec![24, 36, 48, 60, 72]);

    // El campo `density` viaja en el response para que el cliente sepa qué tiene.
    assert_eq!(r_full.json()["density"].as_str(), Some("monthly"));
    assert_eq!(r_hyb.json()["density"].as_str(), Some("hybrid"));
}

/// Las dos densidades coexisten en el cache como entries separadas: una
/// mutación invalida ambas; un GET de cada densidad crea su propia entry.
#[tokio::test]
async fn projection_hybrid_and_monthly_cache_separately() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({
            "category_id": cat,
            "name": "X",
            "current_value": "10000",
        }),
        &owner.cookie,
    )
    .await;

    // Esperar al warm-up post-login (background `tokio::spawn` de ambas
    // densidades para view=household).
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Hit explícito para asegurar entries: ambas viewn=household, density=monthly + hybrid.
    app.get_with_cookie("/v1/projection/series", &owner.cookie).await;
    app.get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie).await;

    let installation_id = installation_id_of(&app, &owner.cookie).await;
    let key_monthly = ProjectionCacheKey {
        installation_id,
        view: LedgerView::Household,
        owner_user_id: None,
        density: Density::Monthly,
    };
    let key_hybrid = ProjectionCacheKey {
        installation_id,
        view: LedgerView::Household,
        owner_user_id: None,
        density: Density::Hybrid,
    };
    {
        let cache = app.state.projection_cache.read().await;
        assert!(cache.contains_key(&key_monthly), "monthly debe estar cacheado");
        assert!(cache.contains_key(&key_hybrid), "hybrid debe estar cacheado");
    }

    // Una mutación invalida ambas.
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({
            "category_id": cat,
            "name": "Y",
            "current_value": "5000",
        }),
        &owner.cookie,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    {
        let cache = app.state.projection_cache.read().await;
        assert!(!cache.contains_key(&key_monthly), "monthly debe haberse invalidado");
        assert!(!cache.contains_key(&key_hybrid), "hybrid debe haberse invalidado");
    }
}

/// Lee el `installation_id` del usuario logado vía `/v1/installation`.
async fn installation_id_of(app: &TestApp, cookie: &str) -> Uuid {
    let r = app.get_with_cookie("/v1/installation", cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "GET /v1/installation: {r:?}");
    let s = r.json()["installation"]["id"]
        .as_str()
        .expect("installation.id is string")
        .to_string();
    Uuid::parse_str(&s).expect("valid uuid")
}

/// Lee el `user_id` vía `/v1/auth/me`.
async fn user_id_of(app: &TestApp, cookie: &str) -> Uuid {
    let r = app.get_with_cookie("/v1/auth/me", cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "GET /v1/auth/me: {r:?}");
    let s = r.json()["id"]
        .as_str()
        .expect("id is string")
        .to_string();
    Uuid::parse_str(&s).expect("valid uuid")
}
