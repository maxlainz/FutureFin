//! Sirve el `index.html` de la SPA con el base path inyectado por request.
//!
//! Por qué un handler y no `ServeFile`: el prefijo público es **por request** (la
//! misma imagen sirve compose en `/` y el Ingress de Home Assistant bajo
//! `/api/hassio_ingress/<token>` a la vez), así que ni un `base` de Vite en build
//! ni un placeholder reescrito al arrancar valen. El HTML del disco se lee una vez;
//! por request se reescriben los refs absolutos (`src="/…"`, `href="/…"`) y se
//! inyecta `window.__FF_BASE__` / `window.__FF_SSO__` para la SPA.
//!
//! Invariante maestro: sin prefijo y sin SSO la respuesta es el fichero **byte a
//! byte** (`Cow::Borrowed`) — el modo compose no cambia ni un carácter.

use crate::prefix::PeerIp;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

/// Header de identidad del proxy de confianza (el Supervisor de HA lo stripea si
/// viene del cliente, así que detrás del ingress es fiable).
pub const X_REMOTE_USER_ID: &str = "x-remote-user-id";

/// `index.html` leído del disco una vez al arrancar.
pub struct SpaIndex {
    html: String,
}

/// Lee `index.html` de la raíz estática. `None` si no existe o no se puede leer
/// (el caller degrada a API-only con un warn, igual que cuando falta la raíz).
pub fn load_index(root: &Path) -> Option<SpaIndex> {
    let path = root.join("index.html");
    match std::fs::read_to_string(&path) {
        Ok(html) => Some(SpaIndex { html }),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "cannot read SPA index.html");
            None
        }
    }
}

pub type SpaIndexState = (Arc<AppState>, Arc<SpaIndex>);

/// Fallback SPA: toda ruta que no es API ni un asset existente devuelve el shell.
pub async fn serve_index(
    State((state, index)): State<SpaIndexState>,
    PeerIp(peer): PeerIp,
    headers: HeaderMap,
) -> Response {
    let prefix = state.request_prefix(&headers);
    let sso = state.trusted_header_auth
        && state.trusted_peers.allows(peer)
        && headers.contains_key(X_REMOTE_USER_ID);

    let body = inject(&index.html, &prefix, sso);
    let modified = matches!(body, Cow::Owned(_));

    let mut response = (StatusCode::OK, body.into_owned()).into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    if modified {
        // El HTML depende de headers de proxy: que ningún caché intermedio sirva
        // el shell con el prefijo (o el flag SSO) de otro despliegue.
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        h.insert(
            header::VARY,
            HeaderValue::from_static("X-Ingress-Path, X-Forwarded-Prefix"),
        );
    } else {
        // El shell de una SPA se revalida siempre (los assets hasheados sí cachean).
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    response
}

/// Reescritura pura del shell. Sin prefijo y sin SSO devuelve `Cow::Borrowed` —
/// los bytes exactos del disco. Con prefijo: los atributos `src="/…"` y
/// `href="/…"` del HTML de entrada (los 4 refs estáticos + los assets que emite
/// Vite) pasan a `{prefix}/…`; no hay `url()` en el CSS del proyecto y los chunks
/// dinámicos resuelven relativos a su módulo importador, así que el HTML es el
/// único punto de reescritura. El `<script>` con `__FF_BASE__`/`__FF_SSO__` va
/// inmediatamente después de `<head>` para ejecutarse antes que cualquier módulo.
pub fn inject<'a>(html: &'a str, prefix: &str, sso: bool) -> Cow<'a, str> {
    if prefix.is_empty() && !sso {
        return Cow::Borrowed(html);
    }
    // `prefix` ya pasó `normalize_prefix` (charset sin comillas, ángulos ni
    // backslash), así que interpolarlo en atributos y JS es seguro.
    let mut out = html.to_string();
    if !prefix.is_empty() {
        for attr in ["src=\"/", "href=\"/"] {
            out = rewrite_attr(&out, attr, prefix);
        }
    }
    let bootstrap = format!(
        "<script>window.__FF_BASE__=\"{prefix}\";window.__FF_SSO__={sso};</script>"
    );
    if let Some(pos) = out.find("<head>") {
        out.insert_str(pos + "<head>".len(), &bootstrap);
    } else {
        // Shell sin <head> no existe en este repo; degradar a prepend es inocuo.
        out.insert_str(0, &bootstrap);
    }
    Cow::Owned(out)
}

/// Prefija cada `attr` (`src="/` o `href="/`) salvo URLs protocol-relative (`//`).
fn rewrite_attr(html: &str, attr: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    while let Some(pos) = rest.find(attr) {
        let after = &rest[pos + attr.len()..];
        out.push_str(&rest[..pos]);
        if after.starts_with('/') {
            // Protocol-relative (`href="//cdn…"`): no se toca.
            out.push_str(attr);
        } else {
            // `attr` termina en `/` — se sustituye por `{prefix}/`.
            out.push_str(&attr[..attr.len() - 1]);
            out.push_str(prefix);
            out.push('/');
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: &str = concat!(
        "<!doctype html>\n<html lang=\"es\">\n  <head>\n",
        "    <link rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\" />\n",
        "    <link rel=\"manifest\" href=\"/site.webmanifest\" />\n",
        "    <script type=\"module\" crossorigin src=\"/assets/index-abc123.js\"></script>\n",
        "    <link rel=\"stylesheet\" crossorigin href=\"/assets/index-def456.css\" />\n",
        "  </head>\n  <body><div id=\"root\"></div></body>\n</html>\n",
    );

    #[test]
    fn without_prefix_and_sso_is_borrowed_verbatim() {
        let out = inject(SHELL, "", false);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out.as_ref(), SHELL);
    }

    #[test]
    fn with_prefix_rewrites_all_absolute_refs_and_injects_bootstrap() {
        let p = "/api/hassio_ingress/tok";
        let out = inject(SHELL, p, true);
        assert!(out.contains(&format!("href=\"{p}/favicon.svg\"")));
        assert!(out.contains(&format!("href=\"{p}/site.webmanifest\"")));
        assert!(out.contains(&format!("src=\"{p}/assets/index-abc123.js\"")));
        assert!(out.contains(&format!("href=\"{p}/assets/index-def456.css\"")));
        // Ningún ref absoluto sin prefijar.
        assert!(!out.contains("src=\"/assets"));
        assert!(!out.contains("href=\"/favicon"));
        // Bootstrap inmediatamente tras <head>.
        let head = out.find("<head>").unwrap() + "<head>".len();
        assert!(out[head..].starts_with(&format!(
            "<script>window.__FF_BASE__=\"{p}\";window.__FF_SSO__=true;</script>"
        )));
    }

    #[test]
    fn sso_without_prefix_still_injects_flag() {
        let out = inject(SHELL, "", true);
        assert!(out.contains("window.__FF_BASE__=\"\";window.__FF_SSO__=true;"));
        // Sin prefijo no se reescribe ningún asset.
        assert!(out.contains("src=\"/assets/index-abc123.js\""));
    }

    #[test]
    fn protocol_relative_and_external_urls_untouched() {
        let html = "<head></head><a href=\"//cdn.example/x\"></a><a href=\"https://x.example/\"></a>";
        let out = inject(html, "/p", false);
        assert!(out.contains("href=\"//cdn.example/x\""));
        assert!(out.contains("href=\"https://x.example/\""));
    }
}
