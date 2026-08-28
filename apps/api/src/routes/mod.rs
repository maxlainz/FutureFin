use crate::handlers::allocation_rules::allocation_rules_router;
use crate::handlers::api_tokens::api_tokens_router;
use crate::handlers::assets::assets_router;
use crate::handlers::auth::{change_password, login, logout, me, patch_me, register};
use crate::handlers::backup_user::{
    export_user_backup, import_user_backup_apply, import_user_backup_preview,
};
use crate::handlers::budget::budget_router;
use crate::handlers::categories::categories_router;
use crate::handlers::changes::changes_router;
use crate::handlers::fallback;
use crate::handlers::ha_sso::{ha_callback, ha_start};
use crate::handlers::health::{health_check, ready_check};
use crate::handlers::history::history_router;
use crate::handlers::installation::{
    get_installation_session_context, get_my_installation, patch_my_installation,
    setup_installation,
};
use crate::handlers::liabilities::liabilities_router;
use crate::handlers::members::members_router;
use crate::handlers::pending_users::pending_users_router;
use crate::handlers::planning::planning_router;
use crate::handlers::projection::projection_router;
use crate::handlers::sso::sso_login;
use crate::handlers::summary::summary_router;
use crate::handlers::transactions::transactions_router;
use crate::openapi::openapi_json;
use crate::state::AppState;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::Method;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Tope global del body en endpoints normales. Endpoints que reciben backups crecen su tope.
const DEFAULT_BODY_LIMIT_BYTES: usize = 1 * 1024 * 1024;
/// Tope para los endpoints de import de backup (`.ffbackup` en base64 → ~33% inflado).
/// Reutilizado por las rutas de import de transacciones (CSV en base64).
pub(crate) const BACKUP_IMPORT_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

pub fn app_router(state: &Arc<AppState>) -> Router {
    let v1 = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(ready_check))
        .nest(
            "/auth",
            Router::new()
                .route("/register", post(register))
                .route("/login", post(login))
                .route("/logout", post(logout))
                .route("/password", post(change_password))
                // Se monta SIEMPRE: la forma del router no depende del entorno. Con el SSO
                // apagado (el default) el handler devuelve 401 `sso_disabled`.
                .route("/sso", post(sso_login))
                // «Entrar con Home Assistant». También SIEMPRE montadas: la forma del router
                // no depende del entorno. Sin `FUTUREFIN_HA_SSO_URL`, `/ha/start` responde
                // 401 `ha_sso_disabled`.
                .route("/ha/start", get(ha_start))
                .route("/ha/callback", get(ha_callback))
                .route("/me", get(me).patch(patch_me)),
        )
        .route(
            "/installation/session-context",
            get(get_installation_session_context),
        )
        .route(
            "/installation",
            get(get_my_installation).patch(patch_my_installation),
        )
        .route("/installation/setup", post(setup_installation))
        .nest("/installation/pending-users", pending_users_router())
        .nest("/installation/members", members_router())
        .nest("/api-tokens", api_tokens_router())
        .nest(
            "/oauth",
            crate::handlers::oauth_consent::oauth_consent_router(state.mcp_enabled),
        )
        .nest("/categories", categories_router())
        .nest("/changes", changes_router())
        .nest("/assets", assets_router())
        .nest("/allocation-rules", allocation_rules_router())
        .nest("/liabilities", liabilities_router())
        .nest("/summary", summary_router())
        .nest("/budget", budget_router())
        .nest("/planning", planning_router())
        .nest("/projection", projection_router())
        .nest("/history", history_router())
        .nest("/transactions", transactions_router())
        .route("/backup/user-export", post(export_user_backup))
        .route(
            "/backup/user-import/preview",
            post(import_user_backup_preview)
                .layer(DefaultBodyLimit::max(BACKUP_IMPORT_BODY_LIMIT_BYTES)),
        )
        .route(
            "/backup/user-import",
            post(import_user_backup_apply)
                .layer(DefaultBodyLimit::max(BACKUP_IMPORT_BODY_LIMIT_BYTES)),
        )
        .fallback(fallback::v1_not_found);

    let origins = cors_origins();

    // El endpoint MCP vive en el nivel raíz (como /health y /openapi.json), dentro del
    // router api → gana siempre al fallback SPA de main.rs. **Se monta pase lo que pase**:
    // con `FUTUREFIN_MCP_ENABLED=0` responde un JSON `mcp_disabled` (ver `mcp/mod.rs`), no
    // desaparece. Trae su propia capa CORS, sin credenciales.
    let mcp = crate::mcp::mcp_router(state.clone(), &origins);
    // Protocolo OAuth (metadata .well-known, register, token, revoke): mismo criterio y
    // mismo kill-switch que /mcp — OAuth hoy solo sirve al conector MCP. OJO: el panel
    // /v1/oauth/connections NO va aquí; se monta siempre (ver oauth_consent_router).
    let oauth_protocol = crate::oauth::oauth_protocol_router(state.mcp_enabled);

    Router::new()
        .route("/health", get(health_check))
        .route("/openapi.json", get(openapi_json))
        .nest("/v1", v1)
        .merge(oauth_protocol)
        // La capa CORS del API va AQUÍ y no en `main.rs` por dos razones: los tests montan el
        // mismo router que el binario (la forma no puede depender del entorno), y `merge` de
        // `mcp` DESPUÉS de este `layer` es lo que deja a `/mcp` fuera de ella — `Router::layer`
        // solo envuelve las rutas ya registradas.
        .layer(api_cors_layer(&origins))
        .merge(mcp)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT_BYTES))
}

/// Orígenes de navegador permitidos (`CORS_ORIGINS`, separados por comas). Fail-loud: una
/// entrada que no es un `HeaderValue` aborta el arranque, igual que antes.
///
/// **Una lista, dos superficies con privilegios distintos** (issue #85, hallazgo 4): la capa del
/// API la usa con `allow_credentials(true)` (su credencial es la cookie) y la de `/mcp` sin
/// credenciales (la suya es el header `Authorization`). Antes había una sola capa sobre el router
/// entero, así que añadir un origen para que funcionara un cliente MCP de navegador concedía de
/// paso acceso **con cookie** a `/v1/backup/user-export`, `/v1/api-tokens` y `/v1/installation`.
/// La misma lista alimenta además la validación de `Origin` de rmcp (hallazgo 3).
pub fn cors_origins() -> Vec<String> {
    let raw = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| {
        "http://127.0.0.1:5173,http://localhost:5173,http://127.0.0.1:8080,http://localhost:8080"
            .into()
    });
    let origins: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<http::HeaderValue>()
                .unwrap_or_else(|_| panic!("invalid CORS_ORIGINS entry: {s}"));
            s.to_string()
        })
        .collect();
    if origins.is_empty() {
        panic!("CORS_ORIGINS resolved empty — set at least one origin when credentials are used");
    }
    origins
}

/// CORS del API propio (`/v1/*`, `/health`, `/openapi.json` y el protocolo OAuth). Lleva
/// `allow_credentials(true)` porque su credencial es la cookie `ff_session`. **No cubre `/mcp`**.
fn api_cors_layer(origins: &[String]) -> CorsLayer {
    let values: Vec<http::HeaderValue> = origins
        .iter()
        .map(|s| s.parse().expect("CORS_ORIGINS entry validated above"))
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(values))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        // AUTHORIZATION sigue aquí por el protocolo OAuth: `client_secret_basic` autentica el
        // cliente con `Authorization: Basic …` en `/oauth/token` y `/oauth/revoke`. Las
        // cabeceras propias de MCP ya NO están en esta lista — viven en la capa de `/mcp`.
        .allow_headers([CONTENT_TYPE, ACCEPT, AUTHORIZATION])
}
