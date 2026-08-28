//! Derivación de la URL pública (issuer OAuth) y construcción segura de redirects.
//!
//! No existe env var obligatoria (promesa 3.0.0): el origen se deriva de los headers del
//! request — `X-Forwarded-Proto`/`X-Forwarded-Host` (primer valor de cada uno, los pone
//! Cloudflare/un reverse proxy) o `Host`. `FUTUREFIN_PUBLIC_URL` lo fija explícitamente
//! cuando el proxy no manda esos headers. El host pasa un charset estricto: un Host
//! inyectado solo envenenaría la respuesta del propio atacante, pero mejor 400 que
//! metadata deforme.
//!
//! ## Subpath: el prefijo NO se deriva del request (issue #85, hallazgo 2)
//!
//! Tras un proxy con subpath (`location /futurefin/` en nginx) el issuer tiene que llevar el
//! prefijo, o el cliente descubre URLs que el proxy no enruta. La salida es **declararlo**:
//! `FUTUREFIN_PUBLIC_URL=https://ejemplo.com/futurefin` — desde este cambio la variable
//! admite path (`main.rs::public_url`, normalizado con `prefix::normalize_prefix`) y de él
//! cuelgan issuer, `resource` y los cuatro endpoints anunciados.
//!
//! **Por qué no se compone con `prefix::request_prefix`**, que era la otra opción:
//! 1. El issuer es una **identidad**, no una decoración. `prefix.rs` no le exige peer de
//!    confianza al prefijo porque un `X-Forwarded-Prefix` falsificado solo deforma los assets
//!    de la respuesta del propio atacante; en cuanto ese mismo texto entra en un **documento
//!    de descubrimiento**, deja de ser inocuo por la misma razón por la que el hallazgo 7
//!    existe. Un valor de operador (fail-loud al arrancar) no lo puede mover una cabecera.
//! 2. Bajo el Ingress de Home Assistant el prefijo es `/api/hassio_ingress/<token>`, un token
//!    **efímero de sesión**: componerlo lo hornearía dentro del issuer. Y el Ingress no es
//!    este caso — el add-on documenta que MCP/OAuth van por el puerto directo.
//! 3. Es más barato y reutiliza `normalize_prefix`, que ya está probado.
//!
//! Con prefijo en la request y sin `FUTUREFIN_PUBLIC_URL` el issuer saldría sin prefijo; eso
//! no se adivina, se avisa (`warn_missing_public_url_for_prefix`, una vez por proceso).

use super::error::OAuthError;
use crate::prefix::{normalize_prefix, X_FORWARDED_PREFIX, X_INGRESS_PATH};
use crate::state::AppState;
use http::HeaderMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Origen público (`https://host[:puerto]`, sin barra final) más, si `FUTUREFIN_PUBLIC_URL` lo
/// declara, el prefijo del subpath (`https://host/futurefin`).
pub fn public_base_url(state: &AppState, headers: &HeaderMap) -> Result<String, OAuthError> {
    if let Some(u) = &state.public_url {
        return Ok(u.clone());
    }
    warn_missing_public_url_for_prefix(headers);
    let scheme = match first_header_value(headers, "x-forwarded-proto") {
        Some("https") => "https",
        _ => "http",
    };
    let host = first_header_value(headers, "x-forwarded-host")
        .or_else(|| {
            headers
                .get(http::header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| OAuthError::invalid_request("missing Host header"))?;
    if !is_valid_host(host) {
        return Err(OAuthError::invalid_request("malformed Host header"));
    }
    Ok(format!("{scheme}://{host}"))
}

/// Un aviso, UNA vez por proceso: hay prefijo de proxy en la request y nadie ha declarado
/// `FUTUREFIN_PUBLIC_URL`, así que el issuer que se va a emitir no lo lleva y el cliente
/// descubrirá URLs que el proxy no enruta. El síntoma sin esta línea es un 404 mudo en
/// `/oauth/token` que no dice de dónde viene.
fn warn_missing_public_url_for_prefix(headers: &HeaderMap) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if WARNED.load(Ordering::Relaxed) {
        return;
    }
    let prefixed = [X_INGRESS_PATH, X_FORWARDED_PREFIX].iter().any(|name| {
        headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .and_then(normalize_prefix)
            .is_some_and(|p| !p.is_empty())
    });
    if prefixed && !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "OAuth metadata is being served for a request that carries a proxy path prefix, \
             but FUTUREFIN_PUBLIC_URL is unset: the issuer and the advertised endpoints will \
             NOT carry the prefix and the client will fetch URLs your proxy does not route. \
             Set FUTUREFIN_PUBLIC_URL=https://host/prefix. (Home Assistant's Ingress is a \
             different case: MCP/OAuth go through the direct port, not the ingress.)"
        );
    }
}

/// Primer valor de un header potencialmente multivalor (`a, b` → `a`).
fn first_header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .split(',')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `host[:puerto]`, IPv6 entre corchetes incluida. Sin `/`, `@`, espacios ni controles.
fn is_valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '[' | ']'))
}

/// URI canónica del recurso MCP (RFC 8707). `base` puede llevar subpath
/// (`https://host/futurefin` → `https://host/futurefin/mcp`).
pub fn mcp_resource_url(base: &str) -> String {
    format!("{base}/mcp")
}

/// URL de la Protected Resource Metadata que anuncia el 401 de `/mcp` (RFC 9728 §5.1).
/// Con la inserción de path de §3.1: el recurso es `{base}/mcp`, luego la metadata vive
/// en `…/oauth-protected-resource/mcp`.
pub fn resource_metadata_url(base: &str) -> String {
    format!("{base}/.well-known/oauth-protected-resource/mcp")
}

/// Añade query params a un `redirect_uri` que puede traer ya su propia query, con el
/// escaping de `url::Url` (aquí es donde nacen los open-redirect si se concatena a mano).
pub fn append_query(redirect_uri: &str, params: &[(&str, &str)]) -> Result<String, OAuthError> {
    let mut u = url::Url::parse(redirect_uri).map_err(|_| OAuthError::server_error())?;
    for (k, v) in params {
        u.query_pairs_mut().append_pair(k, v);
    }
    Ok(u.to_string())
}
