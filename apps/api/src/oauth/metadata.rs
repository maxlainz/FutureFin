//! Documentos de descubrimiento OAuth: RFC 9728 (Protected Resource Metadata) y
//! RFC 8414 (Authorization Server Metadata). SELECT-free y mutación-free (D5): solo
//! reflejan la URL pública derivada del request (o `FUTUREFIN_PUBLIC_URL`).

use super::error::OAuthError;
use super::url::{mcp_resource_url, public_base_url};
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use http::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

/// RFC 9728 — apunta al AS (nosotros mismos). Servida en
/// `/.well-known/oauth-protected-resource` y con el sufijo `/mcp` (inserción de path §3.1).
pub async fn protected_resource(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, OAuthError> {
    let base = public_base_url(&state, &headers)?;
    Ok(Json(json!({
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
) -> Result<Json<Value>, OAuthError> {
    let base = public_base_url(&state, &headers)?;
    Ok(Json(json!({
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
