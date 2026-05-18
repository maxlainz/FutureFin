//! Infraestructura compartida por los tests de integración.
//!
//! Cada test obtiene su propio schema Postgres aislado (`ff_test_<uuid>`) con todas las
//! migraciones aplicadas. Los schemas se leakean intencionalmente — `make clean-test-schemas`
//! o el script `scripts/clean-test-schemas.sh` los borra en bloque cuando hace falta.
//!
//! Requiere un Postgres accesible en `TEST_DATABASE_URL` (por defecto
//! `postgres://futurefin:futurefin_test@127.0.0.1:5433/futurefin_test`).

use axum::body::Body;
use axum::extract::Extension;
use axum::Router;
use futurefin_api::routes;
use futurefin_api::state::AppState;
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
}

#[allow(dead_code)]
impl TestApp {
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

        let login = self
            .post_json(
                "/v1/auth/login",
                serde_json::json!({"username": username, "password": password}),
            )
            .await;
        assert_eq!(login.status, http::StatusCode::OK, "login failed: {login:?}");
        let cookie = login.session_cookie().expect("login sets ff_session");

        LoggedInOwner {
            username: username.to_string(),
            cookie,
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
        ));
        let router = Router::new()
            .merge(routes::app_router())
            .layer(Extension(state.clone()));
        Self {
            router,
            pool,
            schema,
            state,
        }
    }

    pub async fn request(&self, req: Request<Body>) -> ResponseParts {
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
