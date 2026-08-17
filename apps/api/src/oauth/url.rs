//! Derivación de la URL pública (issuer OAuth) y construcción segura de redirects.
//!
//! No existe env var obligatoria (promesa 3.0.0): el origen se deriva de los headers del
//! request — `X-Forwarded-Proto`/`X-Forwarded-Host` (primer valor de cada uno, los pone
//! Cloudflare/un reverse proxy) o `Host`. `FUTUREFIN_PUBLIC_URL` lo fija explícitamente
//! cuando el proxy no manda esos headers. El host pasa un charset estricto: un Host
//! inyectado solo envenenaría la respuesta del propio atacante, pero mejor 400 que
//! metadata deforme.

use super::error::OAuthError;
use crate::state::AppState;
use http::HeaderMap;

/// Origen público (`https://host[:puerto]`, sin barra final).
pub fn public_base_url(state: &AppState, headers: &HeaderMap) -> Result<String, OAuthError> {
    if let Some(u) = &state.public_url {
        return Ok(u.clone());
    }
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

/// URI canónica del recurso MCP (RFC 8707).
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
