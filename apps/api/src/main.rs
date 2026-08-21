use futurefin_api::db;
use futurefin_api::handlers::fallback;
use futurefin_api::routes;
use futurefin_api::state::AppState;
use axum::extract::Extension;
use axum::Router;
use http::header::{ACCEPT, CONTENT_TYPE};
use http::Method;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
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

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "futurefin starting");

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set (see .env.example)");
    let connect_timeout = std::env::var("FUTUREFIN_DB_CONNECT_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| (1..=600).contains(&s))
        .unwrap_or(30);
    let pool =
        db::connect_with_retry(&database_url, std::time::Duration::from_secs(connect_timeout))
            .await?;
    tracing::info!("database connected");
    db::run_migrations(&pool).await?;
    tracing::info!("migrations applied");

    let cookie_secure = parse_bool_env("COOKIE_SECURE").unwrap_or(false);
    let session_ttl_days = std::env::var("SESSION_TTL_DAYS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&d| (1..=400).contains(&d))
        .unwrap_or(30);
    let mcp_enabled = parse_bool_env("FUTUREFIN_MCP_ENABLED").unwrap_or(true);
    let public_url = public_url();

    let shutdown_pool = pool.clone();
    let state = Arc::new(AppState::new(
        env!("CARGO_PKG_VERSION"),
        pool,
        cookie_secure,
        session_ttl_days,
        mcp_enabled,
        public_url.clone(),
    ));

    tracing::info!(
        port = port(),
        session_ttl_days,
        cookie_secure,
        mcp_enabled,
        public_url = public_url.as_deref().unwrap_or("(derived from request)"),
        "server config"
    );

    let api = Router::new()
        .merge(routes::app_router(&state))
        .layer(Extension(state))
        .layer(cors_layer())
        // gzip para responses >1 KB. Reduce ~10× el JSON de /v1/projection/series
        // (260 KB → 30 KB). El cliente lo descomprime sin cambios.
        .layer(CompressionLayer::new().gzip(true))
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
    }
    // Anti-clickjacking global (protege sobre todo la pantalla de consentimiento OAuth,
    // servida por el fallback SPA — por eso la capa va en el router final, no en `api`).
    // Nada de FutureFin se embebe legítimamente en iframes.
    .layer(SetResponseHeaderLayer::overriding(
        http::header::X_FRAME_OPTIONS,
        http::HeaderValue::from_static("DENY"),
    ));

    let reconcile_sweep = spawn_reconcile_sweep(shutdown_pool.clone());

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port()));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Con Postgres en el mismo contenedor, drenar y cerrar el pool ANTES de que el
    // supervisor pare el postmaster es parte del contrato de apagado ordenado.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("http server stopped");
    // El barrido se aborta ANTES de cerrar el pool: si no, una pasada en vuelo consultaría un
    // pool cerrado y ensuciaría el apagado con un error que no significa nada.
    if let Some(task) = reconcile_sweep {
        task.abort();
        tracing::info!("reconcile sweep stopped");
    }
    shutdown_pool.close().await;
    tracing::info!("database pool closed");
    Ok(())
}

/// Horas entre barridos de conciliación. `FUTUREFIN_RECONCILE_SWEEP_HOURS`, default 24,
/// **0 = desactivado**. Fuera de 0..=168 se ignora y se usa el default (misma política laxa que
/// `SESSION_TTL_DAYS`: un valor absurdo no debe tumbar el arranque de una app de escritorio).
fn reconcile_sweep_hours() -> u64 {
    std::env::var("FUTUREFIN_RECONCILE_SWEEP_HOURS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&h| h <= 168)
        .unwrap_or(24)
}

/// Barrido periódico de conciliación de transferencias — la **primera tarea periódica** del
/// binario.
///
/// Por qué existe: los pases post-mutación son best-effort y se tragan sus errores para no
/// convertir una escritura ya persistida en un 5xx. El precio es que un fallo puntual deja el par
/// sin conciliar de forma permanente y **silenciosa**. Esto lo reintenta.
///
/// La primera pasada corre **tras el primer intervalo, no al arrancar**: en el arranque no ha
/// pasado nada que conciliar (el estado quedó como lo dejó el último proceso) y competir con las
/// migraciones y el warm-up por la BD no compra nada.
fn spawn_reconcile_sweep(pool: sqlx::PgPool) -> Option<tokio::task::JoinHandle<()>> {
    let hours = reconcile_sweep_hours();
    if hours == 0 {
        tracing::info!("reconcile sweep disabled (FUTUREFIN_RECONCILE_SWEEP_HOURS=0)");
        return None;
    }
    tracing::info!(every_hours = hours, "reconcile sweep scheduled");
    Some(tokio::spawn(async move {
        let period = std::time::Duration::from_secs(hours * 3600);
        let mut ticker = tokio::time::interval(period);
        // `interval` dispara inmediatamente en el primer tick: se consume aquí para que la
        // primera pasada real sea a `period`.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match futurefin_api::handlers::transactions::reconcile::sweep_all_owners(&pool).await {
                Ok(o) if o.pairs_created > 0 || o.owners_failed > 0 => tracing::info!(
                    owners_scanned = o.owners_scanned,
                    pairs_created = o.pairs_created,
                    owners_failed = o.owners_failed,
                    "reconcile sweep done"
                ),
                // El caso normal en una instalación sana: nada que conciliar. A `debug` para no
                // llenar el log de una línea diaria que no dice nada.
                Ok(o) => tracing::debug!(owners_scanned = o.owners_scanned, "reconcile sweep: nothing to do"),
                Err(e) => tracing::warn!(error = ?e, "reconcile sweep failed; retrying next run"),
            }
        }
    }))
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {},
        _ = int.recv() => {},
    }
    tracing::info!("shutdown signal received — draining connections");
}

/// `FUTUREFIN_PUBLIC_URL` (opcional): origen público canónico para el issuer OAuth.
/// Fail-loud como CORS_ORIGINS: un valor presente pero inválido aborta el arranque en
/// vez de emitir metadata OAuth rota en silencio. Se normaliza al origen (sin path,
/// query ni fragmento, sin barra final).
fn public_url() -> Option<String> {
    let raw = std::env::var("FUTUREFIN_PUBLIC_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let parsed = url::Url::parse(&raw)
        .unwrap_or_else(|e| panic!("invalid FUTUREFIN_PUBLIC_URL ({raw}): {e}"));
    if !matches!(parsed.scheme(), "http" | "https") {
        panic!("FUTUREFIN_PUBLIC_URL must be http(s), got: {raw}");
    }
    if parsed.host_str().is_none() {
        panic!("FUTUREFIN_PUBLIC_URL must include a host: {raw}");
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        panic!("FUTUREFIN_PUBLIC_URL must be a bare origin (no path/query/fragment): {raw}");
    }
    Some(parsed.origin().ascii_serialization())
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
        // AUTHORIZATION + Mcp-Session-Id: clientes MCP de navegador (p.ej. MCP Inspector)
        // mandan el Bearer y la sesión legacy por header y necesitan pasar el preflight.
        .allow_headers([
            CONTENT_TYPE,
            ACCEPT,
            http::header::AUTHORIZATION,
            http::HeaderName::from_static("mcp-session-id"),
        ])
        .expose_headers([http::HeaderName::from_static("mcp-session-id")])
}
