//! Infraestructura compartida por los tests de integración.
//!
//! Cada test obtiene su propio schema Postgres aislado (`ff_test_<uuid>`) con todas las
//! migraciones aplicadas. Los schemas se leakean intencionalmente — bórralos en bloque con:
//! `psql "$TEST_DATABASE_URL" -At -c "SELECT 'DROP SCHEMA '||nspname||' CASCADE;' FROM pg_namespace WHERE nspname LIKE 'ff_test_%'" | psql "$TEST_DATABASE_URL"`
//! (o simplemente recrea el contenedor `ff-test-db`).
//!
//! Requiere un Postgres accesible en `TEST_DATABASE_URL` (por defecto
//! `postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test`).

use axum::body::Body;
use axum::extract::Extension;
use axum::Router;
use futurefin_api::routes;
use futurefin_api::ha_idp::{HaIdentity, HaIdp, HaIdpError, HaTokens};
use futurefin_api::handlers::frame;
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::handlers::spa::{self, SpaIndex};
use futurefin_api::prefix::PeerPolicy;
use futurefin_api::state::{AppState, Density, HaSso, ProjectionCacheKey};
use http::Request;
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test".to_string()
    })
}

/// Crea un schema único en el Postgres de tests, aplica todas las migraciones y devuelve un pool
/// con `search_path` fijado a ese schema. El schema se deja en BD al terminar (intencional).
pub async fn isolated_pool() -> (PgPool, String) {
    let base_url = test_database_url();
    let schema = format!("ff_test_{}", Uuid::new_v4().simple());

    let admin = PgPool::connect(&base_url)
        .await
        .expect("connect to TEST_DATABASE_URL");
    sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
        .execute(&admin)
        .await
        .expect("create test schema");
    admin.close().await;

    let search_path = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |conn, _meta| {
            let sp = search_path.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path TO \"{sp}\", public"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&base_url)
        .await
        .expect("connect with isolated search_path");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("apply migrations in test schema");

    (pool, schema)
}

/// Aplicación bajo test: el router de Axum + el schema asociado (informativo).
#[allow(dead_code)]
pub struct TestApp {
    pub router: Router,
    pub pool: PgPool,
    pub schema: String,
    pub state: Arc<AppState>,
}

/// Owner ya autenticado con cookie de sesión válida. Útil para los tests que necesitan
/// estado más allá de auth.
#[allow(dead_code)]
pub struct LoggedInOwner {
    pub username: String,
    pub cookie: String,
    pub user_id: Uuid,
}

#[allow(dead_code)]
impl TestApp {
    // -----------------------------------------------------------------------
    // Cache de proyección
    //
    // Desde 3.8.0 la invalidación se **espera dentro del handler**
    // (`refresh_projection_after_mutation`), así que tras una mutación el estado de la cache es
    // final y **no hay que dormir para observarlo**. El único `tokio::spawn` que sigue tocando la
    // cache es el warm-up post-login (D7: el login no espera al recompute) — para eso, y solo para
    // eso, está `settle_login_warmup`.
    // -----------------------------------------------------------------------

    /// Id de la instalación singleton.
    pub async fn installation_id(&self) -> Uuid {
        sqlx::query_scalar("SELECT id FROM installation LIMIT 1")
            .fetch_one(&self.pool)
            .await
            .expect("installation id")
    }

    /// Clave de cache de la vista **por defecto** en densidad `monthly` — la que puebla `GET
    /// /v1/projection/series` sin parámetros.
    ///
    /// **Desde 5.0.0 esa vista es `mine`** (R2), no `household`: el helper se renombró en vez de
    /// cambiarle el contenido al viejo `household_key`, porque una clave que dice «household» y
    /// construye otra cosa es exactamente el tipo de mentira que estos tests existen para cazar.
    /// Quien necesite de verdad la entrada del hogar tiene [`TestApp::household_key`] al lado.
    pub fn default_view_key(&self, installation_id: Uuid, user_id: Uuid) -> ProjectionCacheKey {
        ProjectionCacheKey {
            installation_id,
            view: LedgerView::Mine,
            owner_user_id: Some(user_id),
            density: Density::Monthly,
        }
    }

    /// Clave de cache de la vista `household` (densidad `monthly`), que desde 5.0.0 hay que pedir
    /// EXPLÍCITAMENTE con `?view=household`. `user_id` es el SOLICITANTE: la clave lo incluye
    /// también en household porque la respuesta lleva su demografía.
    pub fn household_key(&self, installation_id: Uuid, user_id: Uuid) -> ProjectionCacheKey {
        ProjectionCacheKey {
            installation_id,
            view: LedgerView::Household,
            owner_user_id: Some(user_id),
            density: Density::Monthly,
        }
    }

    /// ¿Está la entrada en la cache?
    pub async fn cache_contains(&self, key: &ProjectionCacheKey) -> bool {
        self.state.projection_cache.read().await.contains_key(key)
    }

    /// Calienta la entrada de la vista POR DEFECTO con un GET sin parámetros y comprueba que
    /// quedó cacheada. Desde 5.0.0 esa vista es `mine` — pásale [`TestApp::default_view_key`].
    ///
    /// **Sin sleep a propósito**: `projection_series_cached` inserta y DESPUÉS responde, y el
    /// cliente de test es in-process, así que al volver el GET la entrada ya está. El sleep que
    /// había aquí no daba margen — se lo daba a una invalidación pendiente para colarse justo
    /// antes del assert (en `current_thread` la tarea spawneada solo corre cuando el test cede, y
    /// el sleep era el único punto donde cedía). Era la causa del flake, no su remedio.
    pub async fn warm_default_view(&self, cookie: &str, key: &ProjectionCacheKey) {
        let r = self.get_with_cookie("/v1/projection/series", cookie).await;
        assert_eq!(r.status, http::StatusCode::OK);
        assert!(
            self.cache_contains(key).await,
            "la cache debería estar caliente tras el GET"
        );
    }

    /// La mutación debía invalidar: se comprueba en el acto, sin poll ni margen.
    pub async fn assert_invalidated(&self, key: &ProjectionCacheKey, what: &str) {
        assert!(
            !self.cache_contains(key).await,
            "la mutación «{what}» debía invalidar la cache de proyección"
        );
    }

    /// Espera a que el warm-up post-login aterrice y **deja la cache vacía**.
    ///
    /// Obligatorio antes de cualquier aserción sobre el CONTENIDO o el TAMAÑO de la cache: el
    /// warm-up de `POST /v1/auth/login` corre en `tokio::spawn` por diseño y puebla household ×
    /// {monthly, hybrid} en cuanto el test cede, así que un `assert!(cache.is_empty())` sin esto es
    /// una carrera — y falla de forma intermitente culpando al código que se estaba probando.
    ///
    /// Es una espera **acotada por un evento**, no un margen a ojo: sale en cuanto las dos
    /// entradas aparecen, y el tope solo se agota si el warm-up realmente no llegó.
    pub async fn settle_login_warmup(&self, installation_id: Uuid) {
        self.settle_login_warmup_for(installation_id, 1).await
    }

    /// Igual, pero esperando el warm-up de `users` usuarios distintos.
    ///
    /// El `len() >= 2` de la versión de un solo usuario dejó de bastar cuando la clave de cache
    /// pasó a incluir al solicitante también en `household` (una entrada household por miembro,
    /// porque la respuesta lleva SU demografía): con dos logins en vuelo, las dos entradas del
    /// usuario A satisfacen la condición mientras las de B siguen en camino, se invalida, y el
    /// warm-up de B aterriza DESPUÉS repoblando la cache que el test creía vacía.
    pub async fn settle_login_warmup_for(&self, installation_id: Uuid, users: usize) {
        // El warm-up inserta {monthly, hybrid} por usuario.
        // La espera es tolerante a propósito: quien llama puede haber mutado antes (crear una
        // categoría o un activo invalida), y entonces las entradas del warm-up ya no existen y
        // no van a aparecer nunca. Lo que NO puede es quedarse corta cuando sí van a llegar —
        // de ahí el conteo por usuario.
        let esperadas = users * 2;
        for _ in 0..200 {
            if self.state.projection_cache.read().await.len() >= esperadas {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        // ...y se limpia: a partir de aquí solo puebla la cache quien el test decida.
        self.state
            .invalidate_projection_by_installation(installation_id)
            .await;
    }

    /// Registra al primer usuario (queda como owner por bootstrap), hace login y devuelve la cookie.
    pub async fn register_and_login_owner(&self, username: &str) -> LoggedInOwner {
        let password = "correct horse battery staple";
        let reg = self
            .post_json(
                "/v1/auth/register",
                serde_json::json!({
                    "username": username,
                    "password": password,
                    "birth_date": "1990-01-01",
                }),
            )
            .await;
        assert_eq!(reg.status, http::StatusCode::CREATED, "register failed: {reg:?}");
        let user_id = Uuid::parse_str(reg.json()["id"].as_str().expect("register id is string"))
            .expect("register id is uuid");

        // Desde 4.9.0 (#146) una instalación NUEVA nace asumiendo 2,5 % de inflación. El arnés
        // la normaliza a 0 para que los cientos de pins escritos bajo el default histórico sigan
        // documentando el comportamiento a inflación 0; un test que quiera inflación la PATCHea
        // explícitamente, y el default real lo pinea
        // `installation_patch.rs::new_installations_are_born_assuming_two_and_a_half_percent`
        // con el flujo crudo (sin este helper). ANTES del login a propósito: el login dispara el
        // warm-up de la cache de proyección y cachearía la serie con 2,5 %.
        sqlx::query("UPDATE installation SET annual_inflation_assumption_percent = 0")
            .execute(&self.pool)
            .await
            .expect("normalize test inflation");

        let login = self
            .post_json(
                "/v1/auth/login",
                serde_json::json!({"username": username, "password": password}),
            )
            .await;
        assert_eq!(login.status, http::StatusCode::OK, "login failed: {login:?}");
        let cookie = login.session_cookie().expect("login sets ff_session");

        // El login dispara el warm-up de la proyección en `tokio::spawn` (D7: no espera al
        // recompute). Se drena AQUÍ, en el helper que lo provoca, para que ningún test tenga que
        // acordarse: si aterriza más tarde repuebla la cache y hace fallar al assert de al lado
        // —culpando a la mutación que se estaba probando—, que es de dónde venían la mitad de los
        // fallos intermitentes de esta suite.
        //
        // `fetch_optional`: el warm-up solo ocurre si el usuario es miembro de la instalación, así
        // que un registro sin bootstrap (o un usuario pending) no tiene nada que drenar y no debe
        // pagar la espera.
        if let Ok(Some(iid)) =
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM installation LIMIT 1")
                .fetch_optional(&self.pool)
                .await
        {
            self.settle_login_warmup(iid).await;
        }

        LoggedInOwner {
            username: username.to_string(),
            cookie,
            user_id,
        }
    }

    /// Registra un segundo usuario (queda pendiente), lo aprueba desde `owner` con el rol
    /// indicado (`"member"` | `"viewer"`), hace login y devuelve su sesión + `user_id`.
    pub async fn register_and_approve_member(
        &self,
        owner: &LoggedInOwner,
        username: &str,
        role: &str,
    ) -> LoggedInOwner {
        let password = "correct horse battery staple";
        let reg = self
            .post_json(
                "/v1/auth/register",
                serde_json::json!({
                    "username": username,
                    "password": password,
                    "birth_date": "1992-02-02",
                }),
            )
            .await;
        assert_eq!(
            reg.status,
            http::StatusCode::CREATED,
            "register member failed: {reg:?}"
        );
        let user_id = Uuid::parse_str(reg.json()["id"].as_str().expect("register id is string"))
            .expect("register id is uuid");

        let approve = self
            .post_json_with_cookie(
                &format!("/v1/installation/pending-users/{user_id}/approve"),
                serde_json::json!({ "role": role }),
                &owner.cookie,
            )
            .await;
        assert_eq!(
            approve.status,
            http::StatusCode::NO_CONTENT,
            "approve member failed: {approve:?}"
        );

        let login = self
            .post_json(
                "/v1/auth/login",
                serde_json::json!({"username": username, "password": password}),
            )
            .await;
        assert_eq!(
            login.status,
            http::StatusCode::OK,
            "member login failed: {login:?}"
        );
        let cookie = login.session_cookie().expect("login sets ff_session");

        LoggedInOwner {
            username: username.to_string(),
            cookie,
            user_id,
        }
    }

    /// Asegura que existe una categoría con ese scope y nombre, y devuelve su id.
    ///
    /// **Tolera el 409 a propósito.** Desde 3.10.0 un hogar nuevo nace con un juego de categorías
    /// por defecto (sin ellas la app no se podía usar recién instalada: el botón de añadir activo
    /// se escondía). Un test que pida «Nómina» o «Supermercado» se choca con una que ya existe, y
    /// eso no es un fallo del test: lo que quiere es *tener* esa categoría, no ser quien la crea.
    /// Reventar aquí obligaría a elegir los nombres de las semillas en función de los tests.
    pub async fn create_category(&self, owner: &LoggedInOwner, scope: &str, name: &str) -> String {
        let resp = self
            .post_json_with_cookie(
                "/v1/categories",
                serde_json::json!({"scope": scope, "name": name}),
                &owner.cookie,
            )
            .await;
        if resp.status == http::StatusCode::CREATED {
            return resp.json()["id"].as_str().expect("category id is string").to_string();
        }
        assert_eq!(
            resp.status,
            http::StatusCode::CONFLICT,
            "create_category failed: {resp:?}"
        );
        let list = self.get_with_cookie("/v1/categories", &owner.cookie).await;
        let found = list
            .json()
            .as_array()
            .expect("categories list")
            .iter()
            .find(|c| c["scope"] == scope && c["name"] == name)
            .unwrap_or_else(|| panic!("409 al crear '{name}' ({scope}) pero no está en la lista"))
            .clone();
        found["id"].as_str().expect("category id is string").to_string()
    }

    /// #150: id de la regla **sembrada** — el sumidero (`kind == "remainder"` sin `cap_kind`) que
    /// `create_asset_core` crea sola al dar de alta el primer activo de un scope virgen. Lee
    /// `GET /v1/allocation-rules` en vez de fiarse de `seeded_allocation_rule_id` (solo lo trae la
    /// respuesta del POST que sembró) para que los tests puedan pedirlo en cualquier punto
    /// posterior. Panic si no hay ninguna: en ese punto del test se asumía un sumidero ya sembrado.
    pub async fn sink_rule_id(&self, cookie: &str) -> String {
        let r = self.get_with_cookie("/v1/allocation-rules", cookie).await;
        assert_eq!(r.status, http::StatusCode::OK, "list allocation-rules: {r:?}");
        r.json()
            .as_array()
            .expect("allocation-rules list is an array")
            .iter()
            .find(|rule| rule["kind"] == "remainder" && rule["cap_kind"].is_null())
            .unwrap_or_else(|| panic!("no uncapped remainder rule (sink) found"))["id"]
            .as_str()
            .expect("rule id is string")
            .to_string()
    }

    /// Cuenta filas en una tabla del schema de tests (sin filtros adicionales).
    pub async fn count_rows(&self, table: &str) -> i64 {
        let q = format!(r#"SELECT COUNT(*)::bigint FROM "{}""#, table.replace('"', ""));
        sqlx::query_scalar(&q)
            .fetch_one(&self.pool)
            .await
            .expect("count rows")
    }
}

/// Ejes de configuración de proxy inverso del `TestApp`. `Default` = todo apagado, que es
/// exactamente el `spawn()` histórico (sin prefijo, sin peers de confianza, sin SSO, sin SPA).
#[allow(dead_code)]
#[derive(Default)]
pub struct TestConfig {
    /// `FUTUREFIN_TRUSTED_PROXY_AUTH`.
    pub trusted_header_auth: bool,
    /// `PeerPolicy::Any` en vez de `Disabled`. En `oneshot` no hay `ConnectInfo`, así que `Any`
    /// es la única política que da un peer «de confianza» en tests.
    pub trusted_peers_any: bool,
    /// `FUTUREFIN_BASE_PATH`.
    pub base_path: String,
    /// HTML del shell SPA: con `Some(_)` se monta el fallback `spa::serve_index` igual que
    /// main.rs (sin `ServeDir`: los tests no sirven assets del disco).
    pub with_spa_index: Option<String>,
    /// `WEB_STATIC_ROOT`: monta el fallback estático **real** del binario publicado
    /// (`spa::mount_static_spa` = `ServeDir` + shell), la misma función que llama `main.rs`.
    ///
    /// No es lo mismo que `with_spa_index`, y la diferencia importa: `ServeDir` **no llama a su
    /// fallback para métodos distintos de GET/HEAD**, así que una ruta ausente devuelve 405 con
    /// cuerpo vacío a un POST y 200 `text/html` a un GET. Los tests que afirman algo sobre lo que
    /// pasa cuando una ruta NO existe (el kill-switch de MCP/OAuth) tienen que usar este eje, o
    /// describen un binario de laboratorio. Gana sobre `with_spa_index` si se dan los dos.
    pub web_static_root: Option<std::path::PathBuf>,
    /// `FUTUREFIN_MCP_ENABLED=0`. Por defecto el MCP va encendido, como en `spawn()`.
    pub mcp_disabled: bool,
    /// `FUTUREFIN_PUBLIC_URL` ya validada (puede llevar subpath: `https://host/futurefin`).
    pub public_url: Option<String>,
    /// Proveedor falso de «Entrar con Home Assistant». Con `Some(_)`, `AppState.ha_sso` queda
    /// puesto y las rutas `/v1/auth/ha/*` funcionan sin un Home Assistant de verdad.
    pub ha_idp: Option<Arc<FakeHaIdp>>,
    /// Origen público de HA que verá el test (default `https://ha.test`). Solo se usa cuando
    /// `ha_idp` es `Some(_)`.
    pub ha_sso_url: Option<String>,
}

/// Llamadas que el doble registra, en orden. Lo que fija el ORDEN (canje → identidad →
/// revocación → base de datos) es un test sobre este vector: la revocación tiene que ocurrir
/// antes de tocar la BD, y sin registro no habría forma de verlo.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeCall {
    Exchange { code: String, client_id: String },
    Identity { access_token: String },
    Revoke { refresh_token: String },
}

/// Doble de `HaIdp` con respuestas guionizadas.
///
/// `revoke` es infalible por firma (el trait lo impone: un fallo revocando no puede tirar un
/// login ya probado), así que «fallo de revocación» se modela como lo que es de verdad — la
/// llamada ocurre, no cambia nada, y el login sigue.
pub struct FakeHaIdp {
    exchange: std::sync::Mutex<Result<HaTokens, HaIdpError>>,
    identity: std::sync::Mutex<Result<HaIdentity, HaIdpError>>,
    calls: std::sync::Mutex<Vec<FakeCall>>,
}

#[allow(dead_code)]
impl FakeHaIdp {
    /// Camino feliz: canje con refresh token e identidad `(id, name)`.
    pub fn happy(external_user_id: Uuid, name: &str) -> Arc<Self> {
        Arc::new(Self {
            exchange: std::sync::Mutex::new(Ok(HaTokens {
                access_token: "ha-access-token".into(),
                refresh_token: Some("ha-refresh-token".into()),
            })),
            identity: std::sync::Mutex::new(Ok(HaIdentity {
                external_user_id,
                name: name.to_string(),
            })),
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Igual, pero HA no devuelve refresh token (⇒ no hay nada que revocar).
    pub fn without_refresh(external_user_id: Uuid, name: &str) -> Arc<Self> {
        let fake = Self::happy(external_user_id, name);
        *fake.exchange.lock().unwrap() = Ok(HaTokens {
            access_token: "ha-access-token".into(),
            refresh_token: None,
        });
        fake
    }

    pub fn exchange_fails() -> Arc<Self> {
        let fake = Self::happy(Uuid::new_v4(), "nadie");
        *fake.exchange.lock().unwrap() = Err(HaIdpError::Exchange);
        fake
    }

    pub fn identity_fails() -> Arc<Self> {
        let fake = Self::happy(Uuid::new_v4(), "nadie");
        *fake.identity.lock().unwrap() = Err(HaIdpError::Identity);
        fake
    }

    /// Cambia la identidad que devolverá la PRÓXIMA llamada. Con esto un mismo `AppState`
    /// puede representar a dos personas distintas de Home Assistant, que es lo que hace falta
    /// para probar la cascada de nombres.
    pub fn set_identity(&self, external_user_id: Uuid, name: &str) {
        *self.identity.lock().unwrap() = Ok(HaIdentity {
            external_user_id,
            name: name.to_string(),
        });
    }

    pub fn calls(&self) -> Vec<FakeCall> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: FakeCall) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait::async_trait]
impl HaIdp for FakeHaIdp {
    async fn exchange_code(&self, code: &str, client_id: &str) -> Result<HaTokens, HaIdpError> {
        self.record(FakeCall::Exchange {
            code: code.to_string(),
            client_id: client_id.to_string(),
        });
        self.exchange.lock().unwrap().clone()
    }

    async fn identity(&self, access_token: &str) -> Result<HaIdentity, HaIdpError> {
        self.record(FakeCall::Identity {
            access_token: access_token.to_string(),
        });
        self.identity.lock().unwrap().clone()
    }

    async fn revoke(&self, refresh_token: &str) {
        self.record(FakeCall::Revoke {
            refresh_token: refresh_token.to_string(),
        });
    }
}

/// Raíz estática temporal con un `index.html` dentro, para los tests que necesitan el
/// `ServeDir` del binario publicado (`TestConfig::web_static_root`). Se borra sola al soltarse.
#[allow(dead_code)]
pub struct TempWebRoot {
    pub path: std::path::PathBuf,
}

#[allow(dead_code)]
impl TempWebRoot {
    pub fn with_index(html: &str) -> Self {
        let path = std::env::temp_dir().join(format!("ff_web_{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).expect("crear la raíz estática temporal");
        std::fs::write(path.join("index.html"), html).expect("escribir index.html");
        Self { path }
    }
}

impl Drop for TempWebRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl TestApp {
    pub async fn spawn() -> Self {
        Self::spawn_with(TestConfig::default()).await
    }

    /// Igual que `spawn()` pero con la configuración de proxy dada. El router se envuelve con la
    /// **misma** capa anti-clickjacking que main.rs (`frame::with_frame_policy`): sin ella, la
    /// política de `X-Frame-Options` / CSP no sería observable desde los tests, porque el router
    /// de `TestApp` no lleva ninguna de las capas exteriores del binario.
    pub async fn spawn_with(cfg: TestConfig) -> Self {
        let (pool, schema) = isolated_pool().await;
        let state = Arc::new(
            AppState::new(
                env!("CARGO_PKG_VERSION"),
                pool.clone(),
                false,
                30,
                !cfg.mcp_disabled,
                cfg.public_url,
            )
            .with_trusted_proxy(
                cfg.base_path,
                if cfg.trusted_peers_any {
                    PeerPolicy::Any
                } else {
                    PeerPolicy::Disabled
                },
                cfg.trusted_header_auth,
            )
            // `Arc<FakeHaIdp>` se convierte solo en `Arc<dyn HaIdp>`.
            .with_ha_idp(cfg.ha_idp.map(|idp| HaSso {
                base_url: cfg
                    .ha_sso_url
                    .unwrap_or_else(|| "https://ha.test".to_string()),
                idp,
            })),
        );
        let mut router = Router::new()
            .merge(routes::app_router(&state))
            .layer(Extension(state.clone()));
        if let Some(root) = cfg.web_static_root {
            // La MISMA función que main.rs: `ServeDir` incluido.
            router = spa::mount_static_spa(router, &root, state.clone());
        } else if let Some(html) = cfg.with_spa_index {
            router = router.fallback_service(
                axum::routing::get(spa::serve_index)
                    .with_state((state.clone(), Arc::new(SpaIndex::from_html(html)))),
            );
        }
        let router = frame::with_frame_policy(router, state.clone());
        Self {
            router,
            pool,
            schema,
            state,
        }
    }

    pub async fn request(&self, mut req: Request<Body>) -> ResponseParts {
        // HTTP/1.1 exige Host y el oneshot no lo pone solo; sin él, los endpoints que
        // derivan la URL pública (OAuth) fallarían con un 400 irreal. Solo si falta.
        if !req.headers().contains_key(http::header::HOST) {
            req.headers_mut().insert(
                http::header::HOST,
                http::HeaderValue::from_static("futurefin.test"),
            );
        }
        let resp = self
            .router
            .clone()
            .oneshot(req)
            .await
            .expect("router oneshot");
        let (parts, body) = resp.into_parts();
        let bytes = body
            .collect()
            .await
            .expect("collect response body")
            .to_bytes();
        ResponseParts {
            status: parts.status,
            headers: parts.headers,
            body: bytes.to_vec(),
        }
    }

    pub async fn get(&self, uri: &str) -> ResponseParts {
        self.request(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("build GET request"),
        )
        .await
    }

    pub async fn get_with_cookie(&self, uri: &str, cookie: &str) -> ResponseParts {
        self.request(
            Request::builder()
                .uri(uri)
                .header(http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("build GET request"),
        )
        .await
    }

    pub async fn post_json(&self, uri: &str, body: serde_json::Value) -> ResponseParts {
        self.request(
            Request::builder()
                .method(http::Method::POST)
                .uri(uri)
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("json")))
                .expect("build POST request"),
        )
        .await
    }

    pub async fn post_json_with_cookie(
        &self,
        uri: &str,
        body: serde_json::Value,
        cookie: &str,
    ) -> ResponseParts {
        self.request(
            Request::builder()
                .method(http::Method::POST)
                .uri(uri)
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, cookie)
                .body(Body::from(serde_json::to_vec(&body).expect("json")))
                .expect("build POST request"),
        )
        .await
    }

    pub async fn patch_json_with_cookie(
        &self,
        uri: &str,
        body: serde_json::Value,
        cookie: &str,
    ) -> ResponseParts {
        self.request(
            Request::builder()
                .method(http::Method::PATCH)
                .uri(uri)
                .header(http::header::CONTENT_TYPE, "application/json")
                .header(http::header::COOKIE, cookie)
                .body(Body::from(serde_json::to_vec(&body).expect("json")))
                .expect("build PATCH request"),
        )
        .await
    }

    pub async fn delete_with_cookie(&self, uri: &str, cookie: &str) -> ResponseParts {
        self.request(
            Request::builder()
                .method(http::Method::DELETE)
                .uri(uri)
                .header(http::header::COOKIE, cookie)
                .body(Body::empty())
                .expect("build DELETE request"),
        )
        .await
    }

    /// POST `application/x-www-form-urlencoded` (token endpoint OAuth).
    pub async fn post_form(&self, uri: &str, form: &[(&str, &str)]) -> ResponseParts {
        let body: String = form
            .iter()
            .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        self.request(
            Request::builder()
                .method(http::Method::POST)
                .uri(uri)
                .header(http::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .expect("build form POST request"),
        )
        .await
    }

    /// Como `post_form` pero con `Authorization: Basic base64(client_id:secret)`.
    pub async fn post_form_with_basic_auth(
        &self,
        uri: &str,
        form: &[(&str, &str)],
        client_id: &str,
        secret: &str,
    ) -> ResponseParts {
        use base64::Engine;
        let creds = base64::engine::general_purpose::STANDARD
            .encode(format!("{client_id}:{secret}"));
        let body: String = form
            .iter()
            .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        self.request(
            Request::builder()
                .method(http::Method::POST)
                .uri(uri)
                .header(http::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(http::header::AUTHORIZATION, format!("Basic {creds}"))
                .body(Body::from(body))
                .expect("build form POST request"),
        )
        .await
    }

    /// GET con `Host` (y opcionalmente otros headers) — para los endpoints de metadata
    /// OAuth, cuyo issuer se deriva del request.
    pub async fn get_with_headers(&self, uri: &str, headers: &[(&str, &str)]) -> ResponseParts {
        let mut builder = Request::builder().uri(uri);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        self.request(builder.body(Body::empty()).expect("build GET request"))
            .await
    }

    /// POST con headers verbatim (y opcionalmente cookie y cuerpo JSON) — la vía para inyectar
    /// `X-Ingress-Path` / `X-Forwarded-Prefix` en login y logout.
    pub async fn post_with_headers(
        &self,
        uri: &str,
        headers: &[(&str, &str)],
        body: Option<serde_json::Value>,
        cookie: Option<&str>,
    ) -> ResponseParts {
        let mut builder = Request::builder().method(http::Method::POST).uri(uri);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        if let Some(c) = cookie {
            builder = builder.header(http::header::COOKIE, c);
        }
        let body = match body {
            Some(json) => {
                builder = builder.header(http::header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&json).expect("json"))
            }
            None => Body::empty(),
        };
        self.request(builder.body(body).expect("build POST request"))
            .await
    }

    /// POST `initialize` mínimo a `/mcp` con el Bearer dado. Devuelve la respuesta cruda
    /// (200 = token válido; 401/403 = rechazado).
    pub async fn mcp_initialize(&self, bearer: Option<&str>) -> ResponseParts {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2026-07-28",
                "capabilities": {},
                "clientInfo": {"name": "oauth-test", "version": "0.0.0"}
            }
        });
        let mut builder = Request::builder()
            .method(http::Method::POST)
            .uri("/mcp")
            .header(http::header::HOST, "futurefin.test")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "initialize");
        if let Some(b) = bearer {
            builder = builder.header(http::header::AUTHORIZATION, format!("Bearer {b}"));
        }
        self.request(
            builder
                .body(Body::from(serde_json::to_vec(&body).expect("json")))
                .expect("build MCP request"),
        )
        .await
    }
}

/// Percent-encoding mínimo suficiente para los tests (valores base64url + URLs).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug)]
pub struct ResponseParts {
    pub status: http::StatusCode,
    pub headers: http::HeaderMap,
    pub body: Vec<u8>,
}

impl ResponseParts {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "response body is not JSON: {e}\nbody:\n{}",
                String::from_utf8_lossy(&self.body)
            )
        })
    }

    /// Cabecera `Location` de un redirect.
    #[allow(dead_code)]
    pub fn location(&self) -> Option<String> {
        self.headers
            .get(http::header::LOCATION)?
            .to_str()
            .ok()
            .map(str::to_string)
    }

    /// El `Set-Cookie` COMPLETO (con sus atributos) de la cookie con ese nombre.
    #[allow(dead_code)]
    pub fn set_cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|s| s.starts_with(&format!("{name}=")))
            .map(str::to_string)
    }

    /// Solo el valor de esa cookie (vacío = borrado).
    #[allow(dead_code)]
    pub fn cookie_value(&self, name: &str) -> Option<String> {
        let raw = self.set_cookie(name)?;
        let first = raw.split(';').next()?;
        Some(first[name.len() + 1..].to_string())
    }

    /// Extrae el valor de la cookie `ff_session` del `Set-Cookie` de la respuesta.
    pub fn session_cookie(&self) -> Option<String> {
        for v in self.headers.get_all(http::header::SET_COOKIE).iter() {
            let s = v.to_str().ok()?;
            for part in s.split(';') {
                let part = part.trim();
                if let Some(rest) = part.strip_prefix("ff_session=") {
                    return Some(format!("ff_session={rest}"));
                }
            }
        }
        None
    }
}
