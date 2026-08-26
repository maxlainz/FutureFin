//! Anti-clickjacking **condicionado al peer**.
//!
//! La invariante histórica era absoluta: «nada de FutureFin se embebe en iframes», implementada
//! como un `X-Frame-Options: DENY` fijo sobre el router final (protege sobre todo la pantalla de
//! consentimiento OAuth, que sirve el fallback SPA — por eso la capa va fuera, no en `api`).
//!
//! La enmienda: el **Ingress de Home Assistant** pinta el add-on dentro de un iframe del **mismo
//! origen** que HA. Con `DENY` la app sale en blanco. Basta con `frame-ancestors 'self'`, que sigue
//! prohibiendo el embebido cross-origin — el vector real del clickjacking.
//!
//! La relajación está **atada al peer de confianza** (`FUTUREFIN_TRUSTED_PROXY_IPS`, ver
//! `crate::prefix::PeerPolicy`): sin él, mandar `X-Ingress-Path` a mano bastaría para desactivar
//! la protección desde fuera. Con peer no confiable —el default— la respuesta lleva `DENY` aunque
//! el header venga presente.

use crate::prefix::X_INGRESS_PATH;
use crate::state::AppState;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::Router;
use http::{header, HeaderValue};
use std::net::SocketAddr;
use std::sync::Arc;

/// Aplica la política al router **final** (el que ya incluye el fallback SPA).
///
/// Se expone como «envuelve este router» y no como un `Layer` suelto porque el tipo que devuelve
/// `from_fn_with_state` no es nombrable; así main.rs y los tests montan exactamente lo mismo.
pub fn with_frame_policy(router: Router, state: Arc<AppState>) -> Router {
    router.layer(axum::middleware::from_fn_with_state(state, frame_policy))
}

async fn frame_policy(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    // El peer se lee de las extensiones (`with_connect_info`); en tests con `oneshot` no hay
    // ninguno y vale `None` — solo `PeerPolicy::Any` lo acepta.
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    let embedded = state.trusted_peers.allows(peer) && req.headers().contains_key(X_INGRESS_PATH);

    let mut response = next.run(req).await;
    let h = response.headers_mut();
    if embedded {
        // Same-origin: es lo que el Ingress necesita y nada más. Sin `X-Frame-Options`, porque
        // `DENY` gana sobre la CSP en los navegadores que miran los dos.
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'self'"),
        );
        h.remove(header::X_FRAME_OPTIONS);
    } else {
        // `insert` (no `append`): sobrescribe, igual que hacía `SetResponseHeaderLayer::overriding`.
        h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    response
}
