//! Documentos de descubrimiento OAuth: RFC 9728 (Protected Resource Metadata) y
//! RFC 8414 (Authorization Server Metadata). SELECT-free y mutación-free (D5): solo
//! reflejan la URL pública derivada del request (o `FUTUREFIN_PUBLIC_URL`).
//!
//! **Cabeceras de caché (issue #85, hallazgo 7).** `.claude/api-routes.md` afirmaba que
//! «toda respuesta OAuth lleva `Cache-Control: no-store`»; era cierto para `OAuthError`, para
//! el token endpoint y para la revocación, pero **no para la metadata**, que es justo la que
//! puede depender de cabeceras de proxy. Ahora las dos van con `no-store` + `Vary` sobre las
//! cabeceras que gobiernan el issuer.
//!
//! Ese `Vary` + `no-store` es lo que cierra el vector de envenenamiento por
//! `X-Forwarded-Host`, que se honra **sin peer de confianza** — y se sigue honrando, a
//! propósito. La asimetría de D17/D18 (peer obligatorio para relajar el anti-clickjacking y
//! para aceptar identidad) es sobre **autoridad**: una cabecera que concede algo. Aquí no se
//! concede nada, se **refleja**: un `X-Forwarded-Host` falsificado solo deforma la respuesta
//! del propio atacante — el mismo argumento que `prefix.rs` da para no exigirle peer al
//! prefijo. Lo único que convertía esa reflexión en algo más era la cacheabilidad, y eso es
//! lo que quitan estas dos líneas. Exigir peer aquí, en cambio, rompería el despliegue
//! corriente (Cloudflare Tunnel, nginx) en cuanto el operador no configurase además
//! `FUTUREFIN_TRUSTED_PROXY_IPS`, una variable nacida para el add-on de Home Assistant: sería
//! fail-closed contra el caso mayoritario para cerrar algo que ya no está abierto.

use super::error::OAuthError;
use super::url::{mcp_resource_url, public_base_url};
use crate::state::AppState;
use axum::extract::Extension;
use axum::response::{IntoResponse, Response};
use axum::Json;
use http::{HeaderMap, HeaderValue};
use serde_json::json;
use std::sync::Arc;

/// Cabeceras de las que depende el issuer que sale de aquí (`oauth/url.rs::public_base_url`).
/// `Host` no entra: los cachés ya distinguen por host de forma implícita.
const ISSUER_VARY: &str = "X-Forwarded-Proto, X-Forwarded-Host";

/// Envuelve un documento de descubrimiento con sus cabeceras de caché.
fn discovery_response(body: serde_json::Value) -> Response {
    let mut resp = Json(body).into_response();
    let h = resp.headers_mut();
    h.insert(
        http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    h.insert(http::header::VARY, HeaderValue::from_static(ISSUER_VARY));
    resp
}

/// RFC 9728 — apunta al AS (nosotros mismos). Servida en
/// `/.well-known/oauth-protected-resource` y con el sufijo `/mcp` (inserción de path §3.1).
pub async fn protected_resource(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, OAuthError> {
    let base = public_base_url(&state, &headers)?;
    Ok(discovery_response(json!({
        "resource": mcp_resource_url(&base),
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
    })))
}

/// RFC 8414. **Sigue sin `scopes_supported`, a propósito** — y desde la Fase 3 (issue #84) la
/// razón se ha afinado, porque el argumento original («no hay scopes con función») ya no es
/// literalmente cierto: los tokens de API sí tienen scope (`api_tokens.scope`, `read_write` |
/// `read_only`, comprobado en `require_mcp_write`).
///
/// El motivo por el que ese scope NO se extiende aquí es otro, y es de fondo: un scope solo
/// restringe **si lo elige la persona**. En un token de API lo elige ella, con cookie de sesión,
/// en Ajustes → Integraciones; el agente no interviene. En OAuth, en cambio, el `scope` viaja en
/// el authorization request, es decir, **lo elige la propia aplicación cliente** — el lado del
/// agente. Un cliente que quiera escribir pedirá `read_write` y lo obtendrá, así que anunciar
/// `scopes_supported` sin una pantalla de consentimiento que deje a la persona estrechar lo
/// pedido no restringe nada: solo añade un campo que un cliente puede negociar consigo mismo.
///
/// Hacerlo bien exigiría scope en `oauth_grants`, validación en `/oauth/authorize` y `/oauth/token`,
/// eco del `scope` concedido en la respuesta de token y un control en la pantalla de
/// consentimiento de la SPA. Mientras eso no exista, anunciarlo sería mentir en la metadata —
/// peor que callar, porque un cliente que lea `scopes_supported` esperará que el token respete lo
/// que pidió. Entretanto el techo de una conexión OAuth sigue siendo el rol vivo del usuario + el
/// toggle `installation.mcp_write_enabled`, comprobados en cada request.
///
/// `S256` es el único PKCE admitido (claude.ai rechaza `plain`).
pub async fn authorization_server(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, OAuthError> {
    let base = public_base_url(&state, &headers)?;
    Ok(discovery_response(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "revocation_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "authorization_response_iss_parameter_supported": true,
    })))
}
