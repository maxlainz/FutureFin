//! Tests del cache in-memory de `/v1/projection/series` (plan v3 + v5).
//!
//! Verifican: (1) el segundo GET se sirve DESDE la cache (probado con un centinela, no con un
//! cronómetro), (2) mutación
//! invalida el cache y los datos nuevos se reflejan, (3) logout invalida
//! solo las entries `view=mine` del usuario (las `view=household`
//! sobreviven).

mod common;

use common::TestApp;
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{Density, ProjectionCacheKey};
use uuid::Uuid;

/// El segundo GET se sirve **desde la cache**, y se demuestra sin medir tiempo.
///
/// Antes esto se comprobaba cronometrando (`hit*2 < miss`), y era el test más flaky del repo: con
/// un household de un activo el miss ya baja a ~13 ms, así que bajo carga el margen desaparece y
/// el test fallaba sin que nada estuviera roto. Peor: la aserción tenía una rama de escape
/// (`hit <= 5ms`) por la que pasaba casi siempre, así que ni siquiera medía lo que decía medir.
///
/// La prueba directa es **envenenar la entrada cacheada**: se sustituye por una copia con un
/// centinela y se comprueba que el siguiente GET lo devuelve. Si el read path recomputara en vez
/// de leer la cache, el centinela no aparecería. Binario, determinista y sin reloj.
#[tokio::test]
async fn projection_series_serves_the_second_get_from_the_cache() {
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

    let iid = app.installation_id().await;
    // Espera al warm-up post-login y deja la cache vacía: si aterrizara más tarde, repoblaría la
    // entrada y pisaría el centinela.
    app.settle_login_warmup(iid).await;
    // 5.0.0 (R2): el GET sin parámetros puebla la entrada de `mine`, que es la vista por defecto.
    let key = app.default_view_key(iid, owner.user_id);

    // 1. MISS: puebla la cache.
    let r1 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r1.status, http::StatusCode::OK);
    assert!(app.cache_contains(&key).await, "el primer GET debe dejar la entrada");

    // 2. Envenenar: misma clave, respuesta con un centinela imposible de recomputar.
    const SENTINEL: &str = "SENTINEL-cache-hit";
    let poisoned = {
        let cache = app.state.projection_cache.read().await;
        let mut resp = (*cache.get(&key).expect("entrada recién insertada").response).clone();
        resp.model_note = SENTINEL.to_string();
        resp
    };
    app.state
        .projection_cache_insert(key.clone(), std::sync::Arc::new(poisoned))
        .await;

    // 3. HIT: si el body trae el centinela, salió de la cache y no de un recompute.
    let r2 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r2.status, http::StatusCode::OK);
    assert_eq!(
        r2.json()["model_note"], SENTINEL,
        "el segundo GET debía servirse de la cache; recomputó"
    );

    // 4. Y el cache no muta nada: tras invalidar, el body vuelve a ser el original byte a byte.
    app.state.invalidate_projection_by_installation(iid).await;
    let r3 = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r3.body, r1.body, "miss y re-miss deben coincidir byte a byte");
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

    // Calentar ambas vistas. `household` va EXPLÍCITO desde 5.0.0 (R2): sin parámetro, el GET
    // puebla `mine` y este test estaría comprobando dos veces la misma entrada.
    let r1 = app
        .get_with_cookie("/v1/projection/series?view=household", &owner.cookie)
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
        owner_user_id: Some(user_id),
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

    // Tras logout: las DOS entries del usuario se borran. Desde el arreglo de la clave,
    // `household` también es suya (lleva su fecha de nacimiento, su horizonte y su edad de
    // jubilación), así que dejarla viva serviría su demografía a quien entrara después.
    // Las entries household de OTROS miembros no se tocan: la clave las distingue.
    {
        let cache = app.state.projection_cache.read().await;
        assert!(
            !cache.contains_key(&key_household),
            "household del usuario DEBE borrarse al logout: la entrada lleva SU demografía"
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

    // Espera al warm-up post-login (el `tokio::spawn` de ambas densidades) y limpia, para que
    // los GETs de abajo sean los que pueblan la cache y no una carrera con él.
    app.settle_login_warmup(app.installation_id().await).await;

    // Hit explícito para asegurar entries: ambas en la vista por defecto (`mine` desde 5.0.0),
    // density=monthly + hybrid.
    app.get_with_cookie("/v1/projection/series", &owner.cookie).await;
    app.get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie).await;

    let installation_id = installation_id_of(&app, &owner.cookie).await;
    let user_id = user_id_of(&app, &owner.cookie).await;
    let key_monthly = ProjectionCacheKey {
        installation_id,
        view: LedgerView::Mine,
        owner_user_id: Some(user_id),
        density: Density::Monthly,
    };
    let key_hybrid = ProjectionCacheKey {
        installation_id,
        view: LedgerView::Mine,
        owner_user_id: Some(user_id),
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

/// REGRESIÓN — la entrada de proyección es **por usuario**, no compartida.
///
/// La clave de cache tenía `owner_user_id: None` en household, pero la respuesta lleva
/// demografía del **solicitante**: `viewer_birth_date`, `months`/`horizon_years` (derivados de
/// su edad por la regla `lifespan_age`), `jubilacion_age` y el eje de edades. Con la clave vieja,
/// el primer miembro que pidiera la proyección dejaba SU horizonte cacheado para todo el hogar:
/// el siguiente recibía la fecha de nacimiento ajena y, si su horizonte real era mayor, un
/// «no alcanzas la jubilación» falso. Cifra plausible, silenciosamente incorrecta — el modo de
/// fallo más caro de este repo.
///
/// Owner nace en 1990-01-01 y el miembro en 1992-02-02 (los helpers lo fijan), así que sus
/// horizontes difieren en ~2 años. Si la cache los confundiera, los dos GET darían lo mismo.
///
/// **5.0.0**: se comprueba en la vista POR DEFECTO (`mine`, R2) — donde además cada uno simula
/// con SU perfil— y también en `household`, donde el horizonte pasa a ser común
/// (`household_max_lifespan`) pero la demografía publicada sigue siendo la del solicitante: por
/// eso las dos entradas del hogar tampoco pueden compartirse.
#[tokio::test]
async fn household_cache_is_per_user_and_never_serves_another_members_demographics() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "X", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;
    let member = app
        .register_and_approve_member(&owner, "bob", "member")
        .await;
    // Dos usuarios ⇒ cuatro entradas de warm-up (la vista por defecto × {monthly, hybrid} por
    // cada uno).
    app.settle_login_warmup_for(app.installation_id().await, 2).await;

    let r_owner = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(r_owner.status, http::StatusCode::OK);
    let r_member = app
        .get_with_cookie("/v1/projection/series", &member.cookie)
        .await;
    assert_eq!(r_member.status, http::StatusCode::OK);

    let (jo, jm) = (r_owner.json(), r_member.json());
    assert_eq!(jo["view"], "mine", "5.0.0: sin `?view` la vista es mine: {jo}");
    assert_eq!(
        jo["viewer_birth_date"], "1990-01-01",
        "el owner debe ver SU fecha de nacimiento: {jo}"
    );
    assert_eq!(
        jm["viewer_birth_date"], "1992-02-02",
        "el miembro recibió la demografía de otro usuario desde la cache: {jm}"
    );
    assert_ne!(
        jo["months"], jm["months"],
        "dos miembros de edades distintas deben recibir horizontes distintos: {jo} / {jm}"
    );

    // Y las dos entradas conviven en la cache, cada una bajo su clave.
    let iid = app.installation_id().await;
    assert!(
        app.cache_contains(&app.default_view_key(iid, owner.user_id))
            .await,
        "falta la entrada por defecto del owner"
    );
    assert!(
        app.cache_contains(&app.default_view_key(iid, member.user_id))
            .await,
        "falta la entrada por defecto del miembro"
    );

    // En `household` el horizonte SÍ es común (el mayor del hogar, §D) — pero la demografía
    // publicada sigue siendo la del solicitante, así que la entrada no puede compartirse.
    let ho = app
        .get_with_cookie("/v1/projection/series?view=household", &owner.cookie)
        .await
        .json();
    let hm = app
        .get_with_cookie("/v1/projection/series?view=household", &member.cookie)
        .await
        .json();
    assert_eq!(ho["horizon_basis"], "household_max_lifespan", "{ho}");
    assert_eq!(
        ho["months"], hm["months"],
        "el hogar simula a un horizonte COMÚN: {ho} / {hm}"
    );
    assert_eq!(ho["viewer_birth_date"], "1990-01-01", "{ho}");
    assert_eq!(hm["viewer_birth_date"], "1992-02-02", "{hm}");
    assert!(
        app.cache_contains(&app.household_key(iid, owner.user_id)).await
            && app.cache_contains(&app.household_key(iid, member.user_id)).await,
        "las dos entradas household deben convivir, una por solicitante"
    );
}

/// 5.0.0 (§D) — **el agregado del hogar se cachea como cualquier otra entrada, y la mutación de
/// CUALQUIER miembro lo invalida.**
///
/// Es la pregunta que abre el agregado: la respuesta del owner depende ahora de filas que el
/// owner no puede tocar. Si la invalidación siguiera siendo «por dueño de la fila» en vez de por
/// instalación, el hogar seguiría enseñando el patrimonio de bob de antes de su último cambio —
/// un número plausible, de nadie, y sin nada en la respuesta que lo delatara.
#[tokio::test]
async fn household_aggregate_is_cached_and_any_member_mutation_invalidates_it() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "asset", "Cash").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat, "name": "A", "current_value": "10000"}),
        &owner.cookie,
    )
    .await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let cat_b = app.create_category(&bob, "asset", "Cash B").await;
    app.post_json_with_cookie(
        "/v1/assets",
        serde_json::json!({"category_id": cat_b, "name": "B", "current_value": "5000"}),
        &bob.cookie,
    )
    .await;

    let iid = app.installation_id().await;
    app.settle_login_warmup_for(iid, 2).await;

    let key = app.household_key(iid, owner.user_id);
    let r0 = app
        .get_with_cookie("/v1/projection/series?view=household", &owner.cookie)
        .await;
    assert_eq!(r0.status, http::StatusCode::OK);
    assert!(app.cache_contains(&key).await, "el agregado debe cachearse");
    let snw0 = r0.json()["starting_net_worth"].as_str().unwrap().to_string();
    assert!(snw0.starts_with("15000"), "10.000 + 5.000 = 15.000: {snw0}");

    // Muta BOB — no el solicitante. La entrada del owner tiene que caer igual.
    let m = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat_b, "name": "B2", "current_value": "3000"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(m.status, http::StatusCode::CREATED, "{m:?}");
    app.assert_invalidated(&key, "alta de activo de otro miembro").await;

    let r1 = app
        .get_with_cookie("/v1/projection/series?view=household", &owner.cookie)
        .await;
    let snw1 = r1.json()["starting_net_worth"].as_str().unwrap().to_string();
    assert!(snw1.starts_with("18000"), "la mutación de bob debe verse: {snw1}");
}
