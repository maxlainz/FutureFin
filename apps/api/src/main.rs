mod auth;
mod db;
mod error;
mod handlers;
mod openapi;
mod routes;
mod state;

use crate::handlers::fallback;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Router;
use http::header::{ACCEPT, CONTENT_TYPE};
use http::Method;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    load_env();

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(
                "futurefin_api=info,tower_http=info,sqlx=warn",
            )
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set (see .env.example)");
    let pool = db::connect(&database_url).await?;
    db::run_migrations(&pool).await?;

    let cookie_secure = parse_bool_env("COOKIE_SECURE").unwrap_or(false);
    let session_ttl_days = std::env::var("SESSION_TTL_DAYS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&d| (1..=400).contains(&d))
        .unwrap_or(30);

    let state = Arc::new(AppState {
        version: env!("CARGO_PKG_VERSION"),
        pool,
        cookie_secure,
        session_ttl_days,
    });

    let api = Router::new()
        .merge(routes::app_router())
        .layer(Extension(state))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http());

    let app = match web_static_root() {
        Some(root) if root.exists() => {
            tracing::info!(root = %root.display(), "serving web UI and API on one port");
            let index = root.join("index.html");
            Router::new()
                .merge(api)
                .fallback_service(ServeDir::new(root).fallback(ServeFile::new(index)))
        }
        Some(root) => {
            tracing::warn!(
                root = %root.display(),
                "WEB_STATIC_ROOT set but path missing — API only"
            );
            Router::new().merge(api).fallback(fallback::not_found)
        }
        None => Router::new().merge(api).fallback(fallback::not_found),
    };

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port()));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn web_static_root() -> Option<PathBuf> {
    std::env::var("WEB_STATIC_ROOT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

fn load_env() {
    let repo_env = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    let _ = dotenvy::from_filename(repo_env).ok();
    let _ = dotenvy::dotenv().ok();
}

fn parse_bool_env(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080)
}

fn cors_layer() -> CorsLayer {
    let raw = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| {
        "http://127.0.0.1:5173,http://localhost:5173,http://127.0.0.1:8080,http://localhost:8080"
            .into()
    });
    let origins: Vec<http::HeaderValue> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<http::HeaderValue>()
                .unwrap_or_else(|_| panic!("invalid CORS_ORIGINS entry: {s}"))
        })
        .collect();
    if origins.is_empty() {
        panic!("CORS_ORIGINS resolved empty — set at least one origin when credentials are used");
    }

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([CONTENT_TYPE, ACCEPT])
}
