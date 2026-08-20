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
use futurefin_api::handlers::person_view::LedgerView;
use futurefin_api::state::{AppState, Density, ProjectionCacheKey};
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

    /// Clave de cache de la vista household en densidad `monthly` (la que sirve `GET
    /// /v1/projection/series` sin parámetros).
    pub fn household_key(&self, installation_id: Uuid) -> ProjectionCacheKey {
        ProjectionCacheKey {
            installation_id,
            view: LedgerView::Household,
            owner_user_id: None,
            density: Density::Monthly,
        }
    }

    /// ¿Está la entrada en la cache?
    pub async fn cache_contains(&self, key: &ProjectionCacheKey) -> bool {
        self.state.projection_cache.read().await.contains_key(key)
    }

    /// Calienta la entrada con un GET y comprueba que quedó cacheada.
    ///
    /// **Sin sleep a propósito**: `projection_series_cached` inserta y DESPUÉS responde, y el
    /// cliente de test es in-process, así que al volver el GET la entrada ya está. El sleep que
    /// había aquí no daba margen — se lo daba a una invalidación pendiente para colarse justo
    /// antes del assert (en `current_thread` la tarea spawneada solo corre cuando el test cede, y
    /// el sleep era el único punto donde cedía). Era la causa del flake, no su remedio.
    pub async fn warm_household(&self, cookie: &str, key: &ProjectionCacheKey) {
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
        // El warm-up inserta household × {monthly, hybrid}: se espera a que aparezcan las dos.
        for _ in 0..200 {
            if self.state.projection_cache.read().await.len() >= 2 {
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

    /// Crea una categoría del scope indicado y devuelve su id.
    pub async fn create_category(&self, owner: &LoggedInOwner, scope: &str, name: &str) -> String {
        let resp = self
            .post_json_with_cookie(
                "/v1/categories",
                serde_json::json!({"scope": scope, "name": name}),
                &owner.cookie,
            )
            .await;
        assert_eq!(resp.status, http::StatusCode::CREATED, "create_category failed: {resp:?}");
        resp.json()["id"].as_str().expect("category id is string").to_string()
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

impl TestApp {
    pub async fn spawn() -> Self {
        let (pool, schema) = isolated_pool().await;
        let state = Arc::new(AppState::new(
            env!("CARGO_PKG_VERSION"),
            pool.clone(),
            false,
            30,
            true,
            None,
        ));
        let router = Router::new()
            .merge(routes::app_router(&state))
            .layer(Extension(state.clone()));
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
