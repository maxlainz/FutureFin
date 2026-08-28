use futurefin_api::db;
use futurefin_api::handlers::{fallback, frame, spa};
use futurefin_api::prefix;
use futurefin_api::routes;
use futurefin_api::state::AppState;
use axum::extract::Extension;
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
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
    // El `?` de siempre imprimía el error con `Debug` (lo que hace `Termination` con un
    // `Box<dyn Error>`): el banner multilínea de la guarda de downgrade salía como una sola
    // línea con `\n` escapados, ilegible justo cuando más falta hace entenderlo. `Display` a
    // stderr, sin el formateo de `tracing`, y salida 1 igual que antes.
    if let Err(e) = db::run_migrations(&pool).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
    tracing::info!("migrations applied");

    let cookie_secure = parse_bool_env("COOKIE_SECURE").unwrap_or(false);
    let session_ttl_days = std::env::var("SESSION_TTL_DAYS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|&d| (1..=400).contains(&d))
        .unwrap_or(30);
    let mcp_enabled = parse_bool_env("FUTUREFIN_MCP_ENABLED").unwrap_or(true);
    let public_url = public_url();
    let base_path = base_path();
    let trusted_peers = trusted_peers();
    let trusted_header_auth = parse_bool_env("FUTUREFIN_TRUSTED_PROXY_AUTH").unwrap_or(false);
    if trusted_header_auth
        && matches!(
            trusted_peers,
            prefix::PeerPolicy::Disabled | prefix::PeerPolicy::Any
        )
    {
        // Fail-loud: SSO sin una lista EXPLÍCITA de peers acepta `X-Remote-User-Id` de
        // cualquiera que alcance al proceso. `Disabled` es evidente; `any` lo parece menos y es
        // igual de grave: tras un proxy que reenvía sin filtrar (o con el puerto publicado por
        // error), un `X-Remote-User-Id` inventado provisiona el PRIMER usuario y se lleva el
        // hogar entero como owner. Si de verdad nadie más puede llegar, la lista es una línea.
        panic!(
            "FUTUREFIN_TRUSTED_PROXY_AUTH=1 requires an explicit FUTUREFIN_TRUSTED_PROXY_IPS \
             list (comma-separated IPs, e.g. 172.30.32.2); 'any' and an unset value are \
             refused because header identity would be accepted from any peer"
        );
    }

    // «Entrar con Home Assistant»: exclusivo del add-on. La URL sola no basta — el entrypoint
    // del add-on exporta `FUTUREFIN_HA_ADDON=1`, y fuera de ahí la variable se rechaza en vez
    // de ignorarse: una instalación compose que la configurara creería tener un login que no
    // puede funcionar (el `client_id` que HA acepta es el origen de ESTA app, y HA solo lo
    // acepta cuando ambos comparten origen a través de su propio Ingress).
    let ha_sso_url = ha_sso_url();
    let ha_addon = parse_bool_env("FUTUREFIN_HA_ADDON").unwrap_or(false);
    if ha_sso_url.is_some() && !ha_addon {
        panic!(
            "FUTUREFIN_HA_SSO_URL is only honoured inside the Home Assistant add-on \
             (FUTUREFIN_HA_ADDON=1): el login con HA es exclusivo del add-on"
        );
    }
    let ha_sso = ha_sso_url.clone().map(|base| futurefin_api::state::HaSso {
        idp: Arc::new(futurefin_api::ha_idp::client::HttpHaIdp::new(base.clone())),
        base_url: base,
    });

    let shutdown_pool = pool.clone();
    let state = Arc::new(
        AppState::new(
            env!("CARGO_PKG_VERSION"),
            pool,
            cookie_secure,
            session_ttl_days,
            mcp_enabled,
            public_url.clone(),
        )
        .with_trusted_proxy(base_path.clone(), trusted_peers, trusted_header_auth)
        .with_ha_idp(ha_sso),
    );

    tracing::info!(
        port = port(),
        session_ttl_days,
        cookie_secure,
        mcp_enabled,
        public_url = public_url.as_deref().unwrap_or("(derived from request)"),
        base_path = if base_path.is_empty() { "(root)" } else { base_path.as_str() },
        trusted_header_auth,
        ha_sso_url = ha_sso_url.as_deref().unwrap_or("(disabled)"),
        "server config"
    );

    // Clon para la tarea periódica: `state` se mueve al Extension del router justo debajo,
    // y el barrido necesita el AppState (no solo el pool) para invalidar la cache de proyección.
    let sweep_state = state.clone();

    // La capa CORS ya no vive aquí: la aplica `routes::app_router`, con una lista para el API
    // (con cookie) y otra para `/mcp` (sin credenciales) — ver el hallazgo 4 del issue #85.
    // Así los tests montan exactamente la misma forma de router que el binario.
    let api = Router::new()
        .merge(routes::app_router(&state))
        .layer(Extension(state.clone()))
        // gzip para responses >1 KB. Reduce ~10× el JSON de /v1/projection/series
        // (260 KB → 30 KB). El cliente lo descomprime sin cambios.
        .layer(CompressionLayer::new().gzip(true))
        .layer(TraceLayer::new_for_http());

    let router = match web_static_root() {
        // El fallback estático se monta con el MISMO helper que usan los tests (spa.rs):
        // el `ServeDir` es justo la pieza que hacía que el binario publicado se comportara
        // distinto del router de laboratorio.
        Some(root) if root.exists() => spa::mount_static_spa(api, &root, state.clone()),
        Some(root) => {
            tracing::warn!(
                root = %root.display(),
                "WEB_STATIC_ROOT set but path missing — API only"
            );
            Router::new().merge(api).fallback(fallback::not_found)
        }
        None => Router::new().merge(api).fallback(fallback::not_found),
    };

    // Anti-clickjacking global (protege sobre todo la pantalla de consentimiento OAuth,
    // servida por el fallback SPA — por eso la capa va en el router final, no en `api`).
    // Nada de FutureFin se embebe legítimamente en iframes, con una excepción atada a un peer
    // de confianza: el Ingress de Home Assistant, que embebe el add-on same-origin (frame.rs).
    let app = frame::with_frame_policy(router, state.clone());

    let reconcile_sweep = spawn_reconcile_sweep(sweep_state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port()));
    tracing::info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Con Postgres en el mismo contenedor, drenar y cerrar el pool ANTES de que el
    // supervisor pare el postmaster es parte del contrato de apagado ordenado.
    // `with_connect_info`: la IP del peer alimenta la política de confianza
    // (anti-clickjacking condicional y SSO por cabeceras — ver `prefix::PeerPolicy`).
    // Si el bind fuera dual-stack, el peer IPv4 llegaría mapeado (`::ffff:…`);
    // `PeerPolicy::allows` canonicaliza ambos lados, así que la lista se escribe en IPv4.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
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
fn spawn_reconcile_sweep(state: std::sync::Arc<AppState>) -> Option<tokio::task::JoinHandle<()>> {
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
            match futurefin_api::handlers::transactions::reconcile::sweep_all_owners(&state).await {
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

/// `FUTUREFIN_PUBLIC_URL` (opcional): URL pública canónica para el issuer OAuth.
/// Fail-loud como CORS_ORIGINS: un valor presente pero inválido aborta el arranque en
/// vez de emitir metadata OAuth rota en silencio.
///
/// **Admite subpath desde el issue #85 (hallazgo 2)**: `https://ejemplo.com/futurefin` es
/// válido, y de ahí cuelgan el issuer, el `resource` MCP y los cuatro endpoints anunciados.
/// Antes se rechazaba cualquier path, y el resultado era que un despliegue tras un proxy con
/// subpath —el que el propio `prefix.rs` documenta como soportado, un nginx con
/// `location /futurefin/`— tenía el OAuth roto **sin ninguna configuración que lo arreglara**:
/// el cliente descubría URLs que el proxy no enruta y fallaba con un 404 que no dice por qué.
///
/// El path se valida con `prefix::normalize_prefix` (la MISMA función que valida
/// `FUTUREFIN_BASE_PATH`, ya probada): debe empezar por `/`, sin `//`, sin `.`/`..`, charset
/// `[A-Za-z0-9._~/-]` (el `%` está prohibido a propósito), ≤128 chars; una barra final se
/// recorta y `/` a secas equivale a raíz. Query y fragmento siguen prohibidos.
///
/// El prefijo NO se deriva de `X-Forwarded-Prefix`: el porqué está en la cabecera de
/// `oauth/url.rs`. Nota para el add-on: `handlers/ha_sso.rs` usa esta misma URL como
/// `client_id` ante Home Assistant, así que declarar un subpath aquí también lo cambia allí —
/// no es un problema en la práctica porque el login con HA vive tras el Ingress, donde nadie
/// declara `FUTUREFIN_PUBLIC_URL`.
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
    if parsed.query().is_some() || parsed.fragment().is_some() {
        panic!("FUTUREFIN_PUBLIC_URL must not carry a query or fragment: {raw}");
    }
    let prefix = prefix::normalize_prefix(parsed.path()).unwrap_or_else(|| {
        panic!(
            "invalid path in FUTUREFIN_PUBLIC_URL ({raw}): must start with '/', no '//', \
             no '.'/'..' segments, charset [A-Za-z0-9._~/-], max 128 chars"
        )
    });
    Some(format!("{}{prefix}", parsed.origin().ascii_serialization()))
}

/// `FUTUREFIN_HA_SSO_URL` (opcional): origen público de Home Assistant para «Entrar con Home
/// Assistant». Mismas reglas y mismo fail-loud que `FUTUREFIN_PUBLIC_URL` — un origen desnudo,
/// http(s), con host, sin path/query/fragmento — porque de él cuelgan la URL de autorización y
/// la del WebSocket, y una URL deforme se manifestaría como un login que redirige a ninguna
/// parte en vez de como un error de arranque.
fn ha_sso_url() -> Option<String> {
    let raw = std::env::var("FUTUREFIN_HA_SSO_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let parsed = url::Url::parse(&raw)
        .unwrap_or_else(|e| panic!("invalid FUTUREFIN_HA_SSO_URL ({raw}): {e}"));
    if !matches!(parsed.scheme(), "http" | "https") {
        panic!("FUTUREFIN_HA_SSO_URL must be http(s), got: {raw}");
    }
    if parsed.host_str().is_none() {
        panic!("FUTUREFIN_HA_SSO_URL must include a host: {raw}");
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        panic!("FUTUREFIN_HA_SSO_URL must be a bare origin (no path/query/fragment): {raw}");
    }
    Some(parsed.origin().ascii_serialization())
}

/// `FUTUREFIN_BASE_PATH` (opcional): prefijo público fijo (subpath tras proxy).
/// Fail-loud vía `prefix::validate_base_path_env`; `""` = raíz (default histórico).
fn base_path() -> String {
    // Sin filtro de blancos: `normalize_prefix` ya mapea vacío y `/` a `""`.
    std::env::var("FUTUREFIN_BASE_PATH")
        .ok()
        .map(|s| prefix::validate_base_path_env(&s))
        .unwrap_or_default()
}

/// `FUTUREFIN_TRUSTED_PROXY_IPS` (opcional): peers de confianza. Fail-loud en entradas
/// inválidas (estilo CORS_ORIGINS); sin definir ⇒ nadie es de confianza.
fn trusted_peers() -> prefix::PeerPolicy {
    std::env::var("FUTUREFIN_TRUSTED_PROXY_IPS")
        .ok()
        .map(|s| prefix::PeerPolicy::from_env_value(&s))
        .unwrap_or(prefix::PeerPolicy::Disabled)
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
