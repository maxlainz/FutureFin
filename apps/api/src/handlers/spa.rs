//! Sirve el `index.html` de la SPA con el base path inyectado por request.
//!
//! Por qué un handler y no `ServeFile`: el prefijo público es **por request** (la
//! misma imagen sirve compose en `/` y el Ingress de Home Assistant bajo
//! `/api/hassio_ingress/<token>` a la vez), así que ni un `base` de Vite en build
//! ni un placeholder reescrito al arrancar valen. El HTML del disco se lee una vez;
//! por request se reescriben los refs absolutos (`src="/…"`, `href="/…"`) y se
//! inyecta `window.__FF_BASE__` / `window.__FF_SSO__` / `window.__FF_HA_LOGIN__`
//! para la SPA.
//!
//! Invariante maestro: sin prefijo y sin ninguna bandera activa la respuesta es el
//! fichero **byte a byte** (`Cow::Borrowed`) — el modo compose no cambia ni un carácter.

use crate::handlers::sso::sso_available;
use crate::prefix::PeerIp;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

/// Cabeceras de las que depende el HTML que sale de aquí: las dos del prefijo y la de
/// identidad, que decide `__FF_SSO__`. Se emite en **todas** las respuestas, también en la
/// verbatim: un caché intermedio que guardara el shell anónimo sin `Vary` se lo devolvería
/// después a una petición que sí trae `X-Remote-User-Id`, y el flag de SSO no llegaría nunca.
/// `__FF_HA_LOGIN__` NO entra en esta lista a propósito: es configuración del PROCESO
/// (`FUTUREFIN_HA_SSO_URL`), igual para todas las requests, así que no genera variantes de
/// caché. Solo se listan las cabeceras que sí cambian el HTML entre peticiones.
const SHELL_VARY: &str = "X-Ingress-Path, X-Forwarded-Prefix, X-Remote-User-Id";

/// Banderas que el shell anuncia a la SPA. `Default` = todas apagadas, que es el shell
/// verbatim.
#[derive(Clone, Copy, Default)]
pub struct ShellFlags {
    /// `window.__FF_SSO__`: esta request puede abrir sesión por cabeceras (proxy de confianza).
    pub sso: bool,
    /// `window.__FF_HA_LOGIN__`: esta instalación ofrece «Entrar con Home Assistant».
    pub ha_login: bool,
}

/// `index.html` leído del disco una vez al arrancar.
pub struct SpaIndex {
    html: String,
}

impl SpaIndex {
    /// Shell a partir de HTML en memoria — para los tests de integración, que montan el fallback
    /// sin tocar el disco (el `ServeDir` de main.rs no participa en la inyección).
    pub fn from_html(html: String) -> Self {
        Self { html }
    }
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

/// Cierra el router del binario: le cuelga el fallback estático (`ServeDir` + shell) que sirve
/// la SPA en el mismo puerto que el API. Si no hay raíz utilizable degrada a API-only con
/// `fallback::not_found`, exactamente como antes.
///
/// **Por qué es una función de la lib y no cinco líneas dentro de `main.rs`** (issue #85,
/// hallazgo 1): el `ServeDir` es la pieza que hace que el binario publicado se comporte distinto
/// del router de laboratorio que montaban los tests — `ServeDir` **no llama a su fallback para
/// métodos distintos de GET/HEAD**, así que una ruta ausente devolvía 405 con cuerpo vacío a un
/// POST, y 200 `text/html` (¡el shell de la SPA!) a un GET. Un test que no monte esto no describe
/// lo que se publica. Ahora `main.rs` y `apps/api/tests/mcp_http.rs` llaman a la MISMA función.
///
/// `root` debe existir; el caller comprueba (`root.exists()`).
pub fn mount_static_spa(api: axum::Router, root: &Path, state: Arc<AppState>) -> axum::Router {
    let Some(index) = load_index(root) else {
        tracing::warn!(
            root = %root.display(),
            "WEB_STATIC_ROOT has no readable index.html — API only"
        );
        return axum::Router::new()
            .merge(api)
            .fallback(crate::handlers::fallback::not_found);
    };
    tracing::info!(root = %root.display(), "serving web UI and API on one port");
    // El index NO lo sirve ServeDir (append_index_html_on_directories(false)):
    // `GET /` cae al fallback, que inyecta el base path por request.
    let index_svc = axum::routing::get(serve_index).with_state((state, Arc::new(index)));
    // `/index.html` explícito TAMBIÉN pasa por el inyector: si no, `ServeDir` encuentra el
    // fichero en disco y lo sirve crudo — sin prefijo reescrito ni `__FF_SSO__`, y con
    // `Cache-Control` de asset estático. Bajo el Ingress esa URL es alcanzable (y algún cliente
    // la pide), así que la SPA saldría rota justo donde el fallback la arregla.
    axum::Router::new()
        .route("/index.html", index_svc.clone())
        .merge(api)
        .fallback_service(
            tower_http::services::ServeDir::new(root)
                .append_index_html_on_directories(false)
                .fallback(index_svc),
        )
}

/// Fallback SPA: toda ruta que no es API ni un asset existente devuelve el shell.
pub async fn serve_index(
    State((state, index)): State<SpaIndexState>,
    PeerIp(peer): PeerIp,
    headers: HeaderMap,
) -> Response {
    let prefix = state.request_prefix(&headers);
    // Predicado único con el endpoint que abrirá la sesión (`handlers/sso.rs`): si el shell
    // dice `__FF_SSO__=true`, el `POST /v1/auth/sso` que la SPA lanzará puede prosperar.
    let flags = ShellFlags {
        sso: sso_available(&state, peer, &headers),
        // Predicado único con `/v1/auth/ha/start`: si el shell pinta el botón, el endpoint que
        // la SPA invocará al pulsarlo puede prosperar.
        ha_login: crate::ha_idp::ha_login_available(&state),
    };

    let body = inject(&index.html, &prefix, flags);
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
    } else {
        // El shell de una SPA se revalida siempre (los assets hasheados sí cachean).
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    // `Vary` en las dos ramas: la respuesta verbatim también es una de las variantes.
    h.insert(header::VARY, HeaderValue::from_static(SHELL_VARY));
    response
}

/// Reescritura pura del shell. Sin prefijo y con todas las banderas apagadas devuelve
/// `Cow::Borrowed` — los bytes exactos del disco. Con prefijo: los atributos `src="/…"` y
/// `href="/…"` del HTML de entrada (los 4 refs estáticos + los assets que emite
/// Vite) pasan a `{prefix}/…`; no hay `url()` en el CSS del proyecto y los chunks
/// dinámicos resuelven relativos a su módulo importador, así que el HTML es el
/// único punto de reescritura. El `<script>` con las banderas va inmediatamente
/// después de `<head>` para ejecutarse antes que cualquier módulo.
pub fn inject<'a>(html: &'a str, prefix: &str, flags: ShellFlags) -> Cow<'a, str> {
    let ShellFlags { sso, ha_login } = flags;
    if prefix.is_empty() && !sso && !ha_login {
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
        "<script>window.__FF_BASE__=\"{prefix}\";window.__FF_SSO__={sso};\
         window.__FF_HA_LOGIN__={ha_login};</script>"
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

    fn sso() -> ShellFlags {
        ShellFlags { sso: true, ha_login: false }
    }

    #[test]
    fn without_prefix_and_without_flags_is_borrowed_verbatim() {
        let out = inject(SHELL, "", ShellFlags::default());
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out.as_ref(), SHELL);
    }

    #[test]
    fn with_prefix_rewrites_all_absolute_refs_and_injects_bootstrap() {
        let p = "/api/hassio_ingress/tok";
        let out = inject(SHELL, p, sso());
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
            "<script>window.__FF_BASE__=\"{p}\";window.__FF_SSO__=true;window.__FF_HA_LOGIN__=false;</script>"
        )));
    }

    #[test]
    fn sso_without_prefix_still_injects_flag() {
        let out = inject(SHELL, "", sso());
        assert!(out.contains(
            "window.__FF_BASE__=\"\";window.__FF_SSO__=true;window.__FF_HA_LOGIN__=false;"
        ));
        // Sin prefijo no se reescribe ningún asset.
        assert!(out.contains("src=\"/assets/index-abc123.js\""));
    }

    /// El login con Home Assistant es config del proceso, no de la request: activa la
    /// inyección él solo, sin prefijo y sin SSO de cabeceras.
    #[test]
    fn ha_login_alone_injects_the_bootstrap() {
        let out = inject(
            SHELL,
            "",
            ShellFlags { sso: false, ha_login: true },
        );
        assert!(matches!(out, Cow::Owned(_)));
        assert!(out.contains(
            "window.__FF_BASE__=\"\";window.__FF_SSO__=false;window.__FF_HA_LOGIN__=true;"
        ));
        assert!(out.contains("src=\"/assets/index-abc123.js\""));
    }

    #[test]
    fn protocol_relative_and_external_urls_untouched() {
        let html = "<head></head><a href=\"//cdn.example/x\"></a><a href=\"https://x.example/\"></a>";
        let out = inject(html, "/p", ShellFlags::default());
        assert!(out.contains("href=\"//cdn.example/x\""));
        assert!(out.contains("href=\"https://x.example/\""));
    }
}
