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
    // La clave del plan se conserva: envenenar el cuerpo no puede desconectar la entrada de su
    // nivel 2, o el HIT publicaría `unavailable` y el centinela se leería como un fallo de cache.
    let plan_key = app.state.projection_cache_plan_key(&key).await;
    // La generación se lee JUSTO antes de insertar (5.0.0, WP A12): aquí no hay cómputo lento en
    // medio, así que es la vigente y la entrada entra. Un test que la inventara estaría probando
    // la guardia en vez del cache.
    let generation = app.state.projection_generation(iid).await;
    app.state
        .projection_cache_insert_if_current(
            key.clone(),
            generation,
            std::sync::Arc::new(poisoned),
            plan_key,
        )
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

// ---------------------------------------------------------------------------------------------
// 5.0.0 WP5-2b — los SOLVES viajan dentro de la entrada cacheada (M4)
// ---------------------------------------------------------------------------------------------

/// Hogar con estrategia por edad: lo que hace falta para que la respuesta lleve solves.
async fn seed_retire_at_age(app: &TestApp, owner: &common::LoggedInOwner) {
    let inc = app.create_category(owner, "income", "Nómina").await;
    let exp = app.create_category(owner, "expense", "Vida").await;
    let ast = app.create_category(owner, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "2400"), (&exp, "1000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                serde_json::json!({"category_id": cat, "amount": amount,
                                   "ends_at_retirement": false}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": ast, "name": "Indexado", "current_value": "20000",
                               "is_liquid": true, "expected_annual_return_percent": "5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            // **La fecha de nacimiento es obligatoria para que haya plan** (C5): sin ella no hay
            // edad que convertir en mes, y la respuesta sale con `plan_absent_reason:
            // "birth_date_missing"` y sin una sola cifra del sorteo.
            serde_json::json!({"strategy": "retire_at_age", "target_retirement_age": 60,
                               "swr_pct": "4", "birth_date": "1986-04-01"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
}

/// **El plan se resuelve UNA vez, con la serie, y se sirve desde la cache** (M4 · 5.0.0).
///
/// El nivel 1 del plan es una bisección estocástica sobre miles de caminos: entre uno y cinco
/// segundos de CPU. Recalcularlo en cada GET convertiría la lectura más cara de la app en la más
/// cara por un orden de magnitud.
///
/// Se prueba con el mismo centinela que el resto del fichero, y no con un cronómetro: se envenena
/// la entrada cacheada con un `success_of_plan` imposible y se comprueba que el siguiente GET lo
/// devuelve. Si el read path volviera a sortear, el centinela desaparecería.
#[tokio::test]
async fn the_plan_is_solved_once_and_served_from_the_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_retire_at_age(&app, &owner).await;

    let iid = installation_id_of(&app, &owner.cookie).await;
    let uid = user_id_of(&app, &owner.cookie).await;
    app.settle_login_warmup(iid).await;

    let warm = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(warm.status, http::StatusCode::OK, "{warm:?}");
    let body = warm.json();
    assert_eq!(
        body["retirement_date_basis"], "target_age",
        "una estrategia por edad publica su base: {body}"
    );
    assert!(
        !body["success_of_plan"].is_null(),
        "con la fecha dada, el sorteo mide si se llega: {body}"
    );
    assert!(
        !body["contribution_underfunded"].is_null(),
        "con fecha dada se contesta SIEMPRE cuánto falta aportar: {body}"
    );

    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Mine,
        owner_user_id: Some(uid),
        density: Density::Monthly,
    };
    {
        let mut cache = app.state.projection_cache.write().await;
        let entry = cache.get_mut(&key).expect("entrada cacheada tras el GET");
        let mut poisoned = (*entry.response).clone();
        poisoned.success_of_plan = Some(rust_decimal::Decimal::new(4242, 4));
        entry.response = std::sync::Arc::new(poisoned);
    }
    let cached = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(
        cached.json()["success_of_plan"], "0.4242",
        "el plan sale de la cache, no de un sorteo nuevo: {}",
        cached.json()
    );
}

/// **Un PATCH del perfil mueve la clave del PLAN**, además de invalidar la serie.
///
/// Son dos mecanismos distintos y los dos hacen falta. La cache de proyección se invalida por
/// instalación (el perfil es input del motor: estrategia, edad, SWR, umbral, pensión…); la cache
/// de PLAN no se invalida nunca porque está direccionada por CONTENIDO — su clave es una huella
/// de la entrada entera, así que una entrada obsoleta no se borra: se queda **inalcanzable**.
///
/// Lo que este test sujeta es exactamente eso: tras el PATCH, la clave de plan que sirve el GET es
/// OTRA. Si no se moviera, el nivel 2 del plan viejo se serviría junto a la serie nueva.
#[tokio::test]
async fn a_retirement_profile_patch_invalidates_the_series_and_moves_the_plan_key() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_retire_at_age(&app, &owner).await;

    let iid = installation_id_of(&app, &owner.cookie).await;
    let uid = user_id_of(&app, &owner.cookie).await;
    app.settle_login_warmup(iid).await;

    let before = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_eq!(before["strategy"], "retire_at_age", "{before}");
    assert_eq!(before["success_threshold_pct"], 95, "{before}");

    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Mine,
        owner_user_id: Some(uid),
        density: Density::Monthly,
    };
    assert!(app.cache_contains(&key).await, "el GET dejó entrada");
    let plan_key_before = app.state.projection_cache_plan_key(&key).await;
    assert!(
        plan_key_before.is_some(),
        "una entrada con plan lleva su clave: sin ella el nivel 2 no se podría volver a encontrar"
    );

    // Cambiar SOLO el umbral: ninguna fila del ledger se mueve, pero la pregunta sí.
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            serde_json::json!({"success_threshold_pct": 85}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    app.assert_invalidated(&key, "PATCH del perfil de jubilación").await;

    let after = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_eq!(after["success_threshold_pct"], 85, "{after}");
    let plan_key_after = app.state.projection_cache_plan_key(&key).await;
    assert_ne!(
        plan_key_before, plan_key_after,
        "otro umbral es otra pregunta: la huella del plan tiene que moverse o la cache serviría \
         los extras del umbral anterior"
    );
}

/// **Una mutación de una REGLA DE AHORRO tira las DOS caches.**
///
/// Hasta 5.0.0 este fichero no tenía ni un test de reglas de asignación: la invalidación existía
/// (`allocation_rules.rs` llama a `invalidate_projection_by_installation` en create/patch/delete/
/// reorder) pero nadie la sujetaba. Cambiar el tope de una regla mueve la CASCADA que ve el motor
/// —y por tanto el reparto del ahorro y el patrimonio simulado—, así que una banda cacheada con
/// la regla vieja describiría un plan que ya no es el guardado.
#[tokio::test]
async fn an_allocation_rule_mutation_drops_the_projection_and_the_bands() {
    use futurefin_api::state::BandsCacheKey;

    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let inc = app.create_category(&owner, "income", "Nómina").await;
    let exp = app.create_category(&owner, "expense", "Vida").await;
    let ast = app.create_category(&owner, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "3000"), (&exp, "2000")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                serde_json::json!({"category_id": cat, "amount": amount, "ends_at_retirement": false}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    // Cuenta corriente sin volatilidad (el colchón) + fondo volátil (el riesgo del que protege).
    let mut assets = Vec::new();
    for (name, value, vol) in [
        ("Cuenta corriente", "1000", None),
        ("Fondo indexado global", "200000", Some("20")),
    ] {
        let mut body = serde_json::json!({
            "category_id": ast, "name": name, "current_value": value,
            "is_liquid": true, "expected_annual_return_percent": "5",
        });
        if let Some(v) = vol {
            body["annual_volatility_percent"] = serde_json::json!(v);
        }
        let r = app.post_json_with_cookie("/v1/assets", body, &owner.cookie).await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
        assets.push(r.json()["id"].as_str().expect("asset id").to_string());
    }
    // El sumidero sembrado apunta a la cuenta (#150): se retargetea al fondo para que la cuenta
    // pueda llevar un tope.
    let sink = app.sink_rule_id(&owner.cookie).await;
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{sink}"),
            serde_json::json!({ "target_asset_id": assets[1] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let rule_id = {
        let r = app
            .post_json_with_cookie(
                "/v1/allocation-rules",
                serde_json::json!({ "target_asset_id": assets[0], "kind": "fixed", "amount": "200",
                                    "cap_kind": "amount", "cap_value": "6000" }),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
        r.json()["id"].as_str().expect("rule id").to_string()
    };

    let iid = installation_id_of(&app, &owner.cookie).await;
    let uid = user_id_of(&app, &owner.cookie).await;
    app.settle_login_warmup(iid).await;

    // Poblar las dos caches.
    let series_key = app.default_view_key(iid, uid);
    let r = app.get_with_cookie("/v1/projection/series", &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(app.cache_contains(&series_key).await, "el GET de la serie dejó entrada");

    let bands_key = BandsCacheKey {
        installation_id: iid,
        user_id: uid,
        paths: 24,
        seed: 11,
        // 5.0.0: el umbral entra en la clave porque entra en la RESPUESTA (el veredicto se mide
        // contra él). Aquí va el default del perfil, que es el que la petición usó.
        threshold_pct: 95,
    };
    let b = app
        .get_with_cookie("/v1/projection/bands?paths=24&seed=11", &owner.cookie)
        .await;
    assert_eq!(b.status, http::StatusCode::OK, "{b:?}");
    let _b = b.json();
    assert!(
        app.state.bands_cache.read().await.contains_key(&bands_key),
        "el GET de bandas dejó entrada"
    );

    // Subir el tope: la cascada cambia.
    let r = app
        .patch_json_with_cookie(
            &format!("/v1/allocation-rules/{rule_id}"),
            serde_json::json!({ "cap": {"kind": "amount", "value": "9000"} }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "PATCH de la regla: {r:?}");

    app.assert_invalidated(&series_key, "PATCH de una regla de ahorro").await;
    assert!(
        !app.state.bands_cache.read().await.contains_key(&bands_key),
        "la mutación de una regla tiene que tirar TAMBIÉN las bandas"
    );

    // Y el recálculo repuebla la cache de bandas.
    let r = app
        .get_with_cookie("/v1/projection/bands?paths=24&seed=11", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(
        app.state.bands_cache.read().await.contains_key(&bands_key),
        "el recálculo debe dejar una entrada nueva"
    );
}

// ---------------------------------------------------------------------------------------------
// 5.0.0 WP A12 — la mutación gana la carrera contra un cómputo lento
// ---------------------------------------------------------------------------------------------

/// **La carrera, forzada exactamente, sin hook y sin reloj.**
///
/// El fallo: un miss (o un warm-up) captura los datos, computa durante segundos y DESPUÉS inserta.
/// Si una mutación invalida en medio, esa inserción llega tarde y **repuebla la cache con la foto
/// de antes** — el usuario edita un activo y la cifra no cambia hasta que expira el TTL. En 4.x la
/// ventana era de ~500 ms; desde 5.0.0 el nivel 1 del plan vive DENTRO del mismo permiso y son
/// segundos.
///
/// No hace falta un hook para reproducirlo: `AppState` expone las dos mitades del cómputo —capturar
/// la generación e insertar—, así que el test **es** el interleaving que se quiere probar, escrito
/// en el orden exacto en que ocurre y sin depender de que dos tareas se crucen. Lo que ejecuta la
/// prueba de verdad es el par de aserciones: con la generación vieja se descarta, con la de ahora
/// se guarda. El segundo caso es el control: sin él, un método que devolviera `false` siempre
/// pasaría el test y habría roto la cache entera.
#[tokio::test]
async fn a_mutation_during_a_compute_wins_over_the_stale_insert() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_retire_at_age(&app, &owner).await;
    let iid = installation_id_of(&app, &owner.cookie).await;
    let user_id = user_id_of(&app, &owner.cookie).await;

    // Un GET normal para tener una respuesta REAL que insertar (nada de un objeto inventado: lo
    // que se prueba es el camino de escritura, no la serialización).
    let r = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Mine,
        owner_user_id: Some(user_id),
        density: Density::Hybrid,
    };
    let (response, plan_key) = {
        let cache = app.state.projection_cache.read().await;
        let e = cache.get(&key).expect("el GET debe dejar la entrada");
        (e.response.clone(), e.plan_key)
    };

    // 1. El cómputo captura la generación de ANTES…
    let captured = app.state.projection_generation(iid).await;
    // 2. …la mutación llega mientras computa (una invalidación es lo que toda mutación dispara)…
    app.state.invalidate_projection_by_installation(iid).await;
    assert!(
        !app.cache_contains(&key).await,
        "la invalidación debe haber dejado la cache sin la entrada"
    );
    // 3. …y el cómputo termina e intenta insertar su foto vieja.
    let stored = app
        .state
        .projection_cache_insert_if_current(key.clone(), captured, response.clone(), plan_key)
        .await;
    assert!(
        !stored,
        "la inserción con una generación obsoleta tiene que descartarse"
    );
    assert!(
        !app.cache_contains(&key).await,
        "y la cache tiene que seguir vacía: si se repuebla, el usuario ve la cifra de antes de su \
         edición hasta que expire el TTL"
    );

    // Control: con la generación de AHORA la entrada entra. Sin esta mitad, un método que
    // descartara siempre pasaría el test de arriba y habría roto la cache entera.
    let current = app.state.projection_generation(iid).await;
    assert_ne!(
        current, captured,
        "la invalidación tiene que haber movido la generación"
    );
    let stored = app
        .state
        .projection_cache_insert_if_current(key.clone(), current, response, plan_key)
        .await;
    assert!(stored, "con la generación vigente la entrada se guarda");
    assert!(app.cache_contains(&key).await, "y queda en la cache");
}

/// **La misma carrera, pero de verdad**: un GET real computando mientras entra una mutación.
///
/// El de arriba fuerza el interleaving escribiéndolo; este lo provoca. La ventana es enorme y por
/// eso no es flaky: el cómputo dura **segundos** (bisección estocástica del nivel 1 dentro del
/// mismo permiso) y la generación se captura en el primer milisegundo, justo después del fallo de
/// cache. La espera de 250 ms está para caer con holgura DENTRO de esa ventana — antes de ella la
/// invalidación llegaría demasiado pronto (el GET capturaría ya la generación nueva y su inserción
/// sería legítima), y ese caso no probaría nada.
///
/// Se comprueba además que el GET responde 200: la guardia descarta la ENTRADA de cache, nunca la
/// respuesta — era correcta cuando se calculó y el llamante está esperándola.
#[tokio::test]
async fn a_get_racing_a_real_mutation_does_not_repopulate_the_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_retire_at_age(&app, &owner).await;
    let iid = installation_id_of(&app, &owner.cookie).await;
    let user_id = user_id_of(&app, &owner.cookie).await;
    let key = ProjectionCacheKey {
        installation_id: iid,
        view: LedgerView::Mine,
        owner_user_id: Some(user_id),
        density: Density::Hybrid,
    };
    app.state.invalidate_projection_by_installation(iid).await;
    assert!(!app.cache_contains(&key).await, "se parte de cache vacía");

    // El router de Axum es `Clone` y se despacha con `oneshot`, igual que `TestApp::request`: es
    // la forma de tener un GET REAL corriendo en otra tarea mientras esta invalida.
    let router = app.router.clone();
    let cookie = owner.cookie.clone();
    let getter = tokio::spawn(async move {
        use tower::ServiceExt;
        let req = http::Request::builder()
            .uri("/v1/projection/series?density=hybrid")
            .header(http::header::COOKIE, cookie)
            .header(http::header::HOST, "futurefin.test")
            .body(axum::body::Body::empty())
            .expect("build GET request");
        router.oneshot(req).await.expect("router oneshot").status()
    });

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    app.state.invalidate_projection_by_installation(iid).await;

    let status = getter.await.expect("la tarea del GET no debe panicar");
    assert_eq!(status, http::StatusCode::OK, "el GET responde igual");
    assert!(
        !app.cache_contains(&key).await,
        "el cómputo que empezó antes de la mutación no puede dejar su foto en la cache"
    );
}

// ---------------------------------------------------------------------------------------------
// 5.0.0 WP A12 — MEDICIONES (`#[ignore]`: cuestan segundos y no son puertas de CI)
// ---------------------------------------------------------------------------------------------

/// Hogar con estrategia `asap`: la que de verdad BISECCIONA la fecha, y por tanto la que mide el
/// coste real del nivel 1. Con `retire_at_age` la fecha es un dato y el solve solo confirma.
async fn seed_asap(app: &TestApp, owner: &common::LoggedInOwner) {
    let inc = app.create_category(owner, "income", "Nómina").await;
    let exp = app.create_category(owner, "expense", "Vida").await;
    let ast = app.create_category(owner, "asset", "Fondos").await;
    for (cat, amount) in [(&inc, "3200"), (&exp, "1400")] {
        let r = app
            .post_json_with_cookie(
                "/v1/budget/entries",
                serde_json::json!({"category_id": cat, "amount": amount,
                                   "ends_at_retirement": false}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    }
    let r = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": ast, "name": "Indexado", "current_value": "90000",
                               "is_liquid": true, "expected_annual_return_percent": "5",
                               "annual_volatility_percent": "14"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            serde_json::json!({"strategy": "asap", "swr_pct": "4", "birth_date": "1986-04-01"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
}

/// **Cuánto se ahorra reutilizando el nivel 1 en las bandas** (WP A12, punto 2).
///
/// `build_installation_projection_input` no resuelve plan, así que `/v1/projection/bands` llegaba
/// SIEMPRE con `plan_level1` vacío y corría `solve_plan_level1` entero para obtener el mismo
/// resultado que la serie acababa de calcular. Se mide con `computed_in_ms`, que es el reloj de
/// dentro del permiso de CPU (solve + sorteo) y no el del request entero:
///
/// - **frío**: una instalación donde NADIE ha pedido la serie ⇒ el nivel 1 se resuelve aquí;
/// - **caliente**: la misma instalación después de un GET de la serie ⇒ `level1_cache` HIT, y lo
///   único que queda es el sorteo.
///
/// Son dos `TestApp` porque la cache del nivel 1 es `pub(crate)` y no se puede vaciar desde un test
/// de integración — y no debería poder: lo que se mide es el camino real, no un mapa manipulado.
/// El `?paths=` distinto en el lado caliente fuerza un MISS del cache de BANDAS sin tocar el plan
/// (el solve usa siempre 500/2.500 y la semilla estable, pase lo que pase con ese parámetro).
#[tokio::test]
#[ignore = "medición: cuesta varios segundos"]
async fn measure_bands_reusing_the_series_solve() {
    const PATHS: u32 = 120;

    let cold_app = TestApp::spawn().await;
    let cold_owner = cold_app.register_and_login_owner("alice").await;
    seed_asap(&cold_app, &cold_owner).await;
    let cold = cold_app
        .get_with_cookie(
            &format!("/v1/projection/bands?paths={PATHS}"),
            &cold_owner.cookie,
        )
        .await;
    assert_eq!(cold.status, http::StatusCode::OK, "{cold:?}");
    let cold_ms = cold.json()["computed_in_ms"].as_u64().expect("computed_in_ms");

    let warm_app = TestApp::spawn().await;
    let warm_owner = warm_app.register_and_login_owner("alice").await;
    seed_asap(&warm_app, &warm_owner).await;
    let series = warm_app
        .get_with_cookie("/v1/projection/series?density=hybrid", &warm_owner.cookie)
        .await;
    assert_eq!(series.status, http::StatusCode::OK, "{series:?}");
    let warm = warm_app
        .get_with_cookie(
            &format!("/v1/projection/bands?paths={}", PATHS + 1),
            &warm_owner.cookie,
        )
        .await;
    assert_eq!(warm.status, http::StatusCode::OK, "{warm:?}");
    let warm_ms = warm.json()["computed_in_ms"].as_u64().expect("computed_in_ms");

    println!("[A12·2] bandas con el nivel 1 FRÍO: {cold_ms} ms");
    println!("[A12·2] bandas con el nivel 1 CALIENTE (HIT de level1_cache): {warm_ms} ms");
    println!("[A12·2] ahorro: {} ms", cold_ms.saturating_sub(warm_ms));

    // Las dos respuestas describen el MISMO plan: si el hit sirviera otro nivel 1, la fecha se
    // movería. Esta es la mitad que hace que la medición signifique algo.
    assert_eq!(
        cold.json()["strategy"],
        warm.json()["strategy"],
        "los dos lados tienen que describir el mismo plan"
    );
}

/// **El presupuesto de tiempo del plan, medido** (WP A12, punto 6).
///
/// Dos cifras y las dos importan por separado:
///
/// 1. el primer `GET /v1/projection/series?view=mine` **después de un PATCH del perfil** —o sea, un
///    miss con el nivel 1 dentro del permiso—, que es lo que el usuario espera mirando la pantalla;
/// 2. cuánto tarda `needed_capital_curve_state` en llegar a `ready`, que es el NIVEL 2 en segundo
///    plano: la curva de capital por edad, unas ocho biseccciones por nodo de la rejilla.
///
/// Se mide en `debug`, que es como corre el arnés; en release el motor va varias veces más rápido.
/// Si alguna se sale de su presupuesto, lo que hay que hacer es **decirlo con el número**, no
/// ajustar la constante hasta que pase.
#[tokio::test]
#[ignore = "medición: cuesta decenas de segundos"]
async fn measure_the_plan_time_budget() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    seed_asap(&app, &owner).await;

    // Un GET previo para que lo que se mida sea el miss de DESPUÉS del PATCH y no el de estrenar
    // la instalación (que además arrastra el warm-up del login).
    let _ = app
        .get_with_cookie("/v1/projection/series?view=mine", &owner.cookie)
        .await;
    let r = app
        .patch_json_with_cookie(
            "/v1/auth/me/retirement-profile",
            serde_json::json!({"swr_pct": "3.5"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let t0 = std::time::Instant::now();
    let first = app
        .get_with_cookie("/v1/projection/series?view=mine", &owner.cookie)
        .await;
    let first_ms = t0.elapsed().as_millis();
    assert_eq!(first.status, http::StatusCode::OK, "{first:?}");
    println!("[A12·6] primer GET series tras el PATCH: {first_ms} ms");
    println!(
        "[A12·6]   estado inicial de la curva: {}",
        first.json()["needed_capital_curve_state"]
    );

    // El nivel 2 llega en segundo plano; se sondea el MISMO endpoint, que es lo que hace la SPA.
    let t1 = std::time::Instant::now();
    let mut state = String::new();
    while t1.elapsed() < std::time::Duration::from_secs(120) {
        let body = app
            .get_with_cookie("/v1/projection/series?view=mine", &owner.cookie)
            .await
            .json();
        state = body["needed_capital_curve_state"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if state != "computing" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!(
        "[A12·6] `needed_capital_curve_state: {state}` a los {} ms del primer GET",
        t1.elapsed().as_millis()
    );
    assert_eq!(
        state, "ready",
        "el nivel 2 tiene que aterrizar; `unavailable` aquí sería un fallo del sorteo"
    );
}
