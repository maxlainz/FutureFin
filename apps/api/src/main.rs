use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    version: &'static str,
}

#[derive(Serialize)]
struct HealthBody {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthBody> {
    Json(HealthBody {
        status: "ok",
        service: "futurefin-api",
        version: state.version,
    })
}

async fn readiness(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Placeholder: extend with DB ping when persistence lands.
    Json(HealthBody {
        status: "ok",
        service: "futurefin-api",
        version: state.version,
    })
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(
            |_| tracing_subscriber::EnvFilter::new("futurefin_api=info,tower_http=info"),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(AppState {
        version: env!("CARGO_PKG_VERSION"),
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/health", get(health))
        .route("/v1/ready", get(readiness))
        .fallback(not_found)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port()));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

fn port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080)
}
