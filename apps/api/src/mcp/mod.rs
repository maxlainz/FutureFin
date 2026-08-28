//! Endpoint MCP embebido (`/mcp`, Streamable HTTP).
//!
//! Un único binario: el servicio Tower de rmcp se monta en el mismo router (y puerto)
//! que el API y la SPA. Auth por token Bearer (`ffp_…`/`ffo_…`, ver `mcp/auth.rs`)
//! validada por middleware ANTES del protocolo MCP.
//!
//! **La ruta se monta SIEMPRE** — la forma del router no depende del entorno, misma
//! doctrina que `/v1/auth/sso` (D18). Con `FUTUREFIN_MCP_ENABLED=0` lo que cambia es el
//! *handler*, no la tabla de rutas: `/mcp` responde un JSON `mcp_disabled` en vez de
//! desaparecer. Antes el router no se montaba y el resultado en la imagen publicada era
//! **405 con cuerpo vacío** (el fallback final es un `ServeDir`, que no llama a su
//! fallback para métodos distintos de GET/HEAD): un kill-switch que, al activarse, se
//! diagnostica como avería.
//!
//! ### CORS: capa propia, sin credenciales (issue #85, hallazgo 4)
//! La capa CORS del API lleva `allow_credentials(true)` porque su credencial es la cookie.
//! `/mcp` NO usa cookie —se autentica por `Authorization`—, así que tiene su **propia**
//! capa sin credenciales: añadir un origen para que funcione un cliente MCP de navegador
//! (el Inspector) ya no concede de paso acceso **con cookie** a `/v1/backup/user-export`,
//! `/v1/api-tokens` o `/v1/installation`. La lista de orígenes sigue siendo la misma
//! (`CORS_ORIGINS`); lo que se separa es el privilegio que concede.
//!
//! ### Sesiones Streamable HTTP y la credencial (issue #85, hallazgo 9)
//! El `Mcp-Session-Id` que emite `LocalSessionManager` **no está ligado al Bearer**, y se
//! deja así **a conciencia**:
//! - Hoy no compra nada un atacante. El middleware Bearer corre ANTES del protocolo en
//!   *cada* request, la identidad se re-resuelve viva (D14) y toda tool se ejecuta como el
//!   usuario del token presentado — nunca como «el de la sesión». Un `Mcp-Session-Id`
//!   robado o adivinado sin token válido no pasa del 401.
//! - El servidor **no emite notificaciones ni requests server→cliente**, así que ninguna
//!   sesión transporta datos que no haya pedido esa misma request autenticada. Ese es el
//!   ÚNICO hecho que hace segura la falta de ligadura.
//! - El coste de cerrarlo hoy no es cero: `LocalSessionManager` no tiene punto de enganche
//!   para atar identidad, harían falta un `SessionManager` propio con su mapa
//!   sesión→credencial y su desalojo, y todo eso para una capa que **el propio protocolo
//!   está retirando** (SEP-2567: bajo `2026-07-28` no hay sesiones; los tests de este repo
//!   ya corren por el camino stateless).
//!
//! **Disparador para reabrirlo**: la primera capacidad que emita algo hacia el cliente por
//! iniciativa del servidor (notificaciones, progreso, SSE reanudable con datos). En ese
//! momento hay dos salidas: atar la sesión a la credencial con un `SessionManager` propio,
//! o poner `legacy_session_mode: false` y quedarse solo con el camino stateless. No lo
//! añadas «por si acaso» antes: sería un cambio de comportamiento del transporte sin un
//! riesgo que lo justifique.

pub mod auth;
pub mod server;

use crate::error::ApiError;
use crate::state::AppState;
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use http::{HeaderName, HeaderValue, Method};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Tope de body del POST a `/mcp`, en bytes.
///
/// **Tiene que fijarse aquí explícitamente.** `DefaultBodyLimit` de axum actúa a través de los
/// *extractores*, y `/mcp` es un `route_service`: el servicio de rmcp lee el body por su cuenta
/// con su propio tope, que por defecto es de 4 MiB. Es decir, el «límite global de 1 MiB» que
/// documentan `routes/mod.rs` y las skills **no aplicaba** a la única ruta del binario que no
/// pasa por un extractor. Se iguala al global para que el invariante vuelva a ser cierto.
pub const MCP_MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Router de `/mcp`. Se monta siempre; `state.mcp_enabled` decide el handler, no la ruta.
///
/// `browser_origins` son los orígenes de `CORS_ORIGINS` (ver `routes::cors_origins`). Alimentan
/// dos cosas distintas que casualmente comparten lista: el preflight CORS de esta superficie y
/// la validación de `Origin` de rmcp.
pub fn mcp_router(state: Arc<AppState>, browser_origins: &[String]) -> Router {
    let routes = if state.mcp_enabled {
        enabled_route(state, browser_origins)
    } else {
        Router::new().route("/mcp", any(mcp_disabled))
    };
    // `route_layer`, NO `layer`: `Router::layer` envuelve también el fallback del router, y al
    // hacer `merge` en `routes/mod.rs` ese fallback se lleva por delante el del router de
    // destino — el resultado sería que CUALQUIER ruta desconocida (`/oauth/authorize`, el shell
    // de la SPA) pasaría por la auth Bearer de MCP y devolvería 401. `route_layer` solo corre si
    // la request casa con una ruta de ESTE router, que es exactamente lo que se quiere para una
    // capa de autenticación. Lo pilló `get_oauth_authorize_is_not_handled_by_the_api`.
    routes.route_layer(mcp_cors_layer(browser_origins))
}

fn enabled_route(state: Arc<AppState>, browser_origins: &[String]) -> Router {
    let factory_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(server::FutureFinMcp::new(factory_state.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            // El default de rmcp solo acepta Hosts loopback (anti DNS-rebinding para
            // servidores locales). Aquí el gate es el Bearer y el despliegue objetivo es
            // LAN/Cloudflare Tunnel con Host arbitrario → validación de Host desactivada.
            .disable_allowed_hosts()
            // La OTRA mitad del anti-DNS-rebinding, que sí se puede exigir sin saber el Host:
            // el `Origin`. El default de rmcp es lista vacía = validación APAGADA, y el spec
            // MCP la pide explícitamente para servidores HTTP. No rompe a ningún cliente sin
            // navegador: rmcp deja pasar una request **sin** `Origin` aunque la lista no esté
            // vacía (`validate_origin_header` devuelve `Ok(())` si falta la cabecera), y
            // Claude Desktop / Claude Code no la mandan.
            .with_allowed_origins(browser_origins.to_vec())
            .with_max_request_body_bytes(MCP_MAX_REQUEST_BODY_BYTES),
    );

    Router::new()
        .route_service("/mcp", service)
        .route_layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let state = state.clone();
                async move { auth::mcp_bearer_auth(state, req, next).await }
            },
        ))
}

/// Mensaje (y, por su prefijo, código estable) del kill-switch. Literal COMPLETO en una sola
/// definición: `error_codes_parity` extrae los códigos del fuente y un `format!` los volvería
/// invisibles. Lo comparte `oauth/mod.rs` — es el mismo interruptor.
pub const MCP_DISABLED_MESSAGE: &str =
    "mcp_disabled: el servidor MCP está desactivado en esta instalación \
     (FUTUREFIN_MCP_ENABLED=0); ni /mcp ni el protocolo OAuth que lo sirve están disponibles";

/// Respuesta de `/mcp` con el kill-switch echado. JSON con código estable, cualquier método.
async fn mcp_disabled() -> impl IntoResponse {
    ApiError::NotFoundWith(MCP_DISABLED_MESSAGE.into())
}

/// CORS de `/mcp`: **sin `allow_credentials`** (hallazgo 4) y con el preflight completo que
/// pide un cliente MCP de navegador (hallazgo 5).
///
/// `MCP-Protocol-Version` es obligatoria en toda petición no-`initialize` desde la revisión
/// 2025-06-18 y el SDK la valida; `Last-Event-ID` reanuda un stream SSE cortado;
/// `Mcp-Method`/`Mcp-Name` son el routing espejo de SEP-2243. Sin ellas en `allow_headers` el
/// navegador aborta en el preflight con un error de CORS que no menciona MCP — el síntoma es
/// indistinguible de «el servidor está caído».
///
/// `Mcp-Param-*` (SEP-2243) NO está: es un **prefijo**, no un nombre, y `allow_headers` solo
/// admite nombres. Un cliente de navegador que use esos parámetros por cabecera fallará el
/// preflight; ninguno conocido lo hace hoy, y la alternativa (`AllowHeaders::mirror_request`)
/// cambia una lista auditable por un espejo.
fn mcp_cors_layer(origins: &[String]) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(header_values(origins)))
        // Métodos del transporte Streamable HTTP: POST (JSON-RPC), GET (stream SSE),
        // DELETE (terminar sesión).
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            CONTENT_TYPE,
            ACCEPT,
            AUTHORIZATION,
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("last-event-id"),
            HeaderName::from_static("mcp-method"),
            HeaderName::from_static("mcp-name"),
        ])
        // `WWW-Authenticate` no es una cabecera de respuesta safelisted: sin exponerla, un
        // cliente de navegador no puede leer el `resource_metadata=` del 401 y no descubre
        // nunca el authorization server (RFC 9728 §5.1).
        .expose_headers([
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("mcp-protocol-version"),
            WWW_AUTHENTICATE,
        ])
}

fn header_values(origins: &[String]) -> Vec<HeaderValue> {
    origins
        .iter()
        .map(|s| {
            s.parse::<HeaderValue>()
                .unwrap_or_else(|_| panic!("invalid CORS_ORIGINS entry: {s}"))
        })
        .collect()
}
