//! Prefijo público bajo el que un proxy inverso sirve FutureFin.
//!
//! El servidor sigue montando TODAS sus rutas en la raíz: los proxies con subpath
//! (el Ingress de Home Assistant, un nginx con `location /futurefin/`) **quitan el
//! prefijo** antes de entregar la petición. Lo que sí depende del prefijo es lo que
//! resuelve el **navegador**: assets del HTML, URLs de fetch y el `Path` de la cookie.
//! Este módulo decide qué prefijo aplica a cada request; `handlers/spa.rs` lo inyecta
//! en el HTML y `handlers/auth.rs` lo usa para acotar la cookie.
//!
//! La detección NO exige peer de confianza a propósito: un `X-Forwarded-Prefix`
//! falsificado solo deforma la respuesta del propio atacante (assets que no cargan).
//! Lo que sí exige peer de confianza es relajar el anti-clickjacking
//! (`handlers/frame.rs`) y aceptar identidad por cabeceras (`handlers/sso.rs`).

use axum::extract::{ConnectInfo, FromRequestParts};
use http::request::Parts;
use http::HeaderMap;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, OnceLock};

/// IP del peer TCP, si el servidor arrancó `with_connect_info` (en tests con
/// `oneshot` no hay `ConnectInfo` y vale `None`). Extractor infalible: la política
/// de confianza decide qué hacer con un peer desconocido, no el framework.
pub struct PeerIp(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(PeerIp(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ci| ci.0.ip()),
        ))
    }
}

/// Header que envía el Ingress de Home Assistant (`/api/hassio_ingress/<token>`).
pub const X_INGRESS_PATH: &str = "x-ingress-path";
/// Header genérico de proxies inversos con subpath (nginx, Traefik, Caddy).
pub const X_FORWARDED_PREFIX: &str = "x-forwarded-prefix";

/// Prefijo efectivo de la request. Precedencia:
/// `X-Ingress-Path` > `X-Forwarded-Prefix` > `FUTUREFIN_BASE_PATH` > `""`.
/// Un header presente pero inválido se ignora (con un `warn` deduplicado) y se
/// sigue con la fuente siguiente. `""` = raíz (el caso de siempre).
pub fn request_prefix(base_path: &str, headers: &HeaderMap) -> String {
    for name in [X_INGRESS_PATH, X_FORWARDED_PREFIX] {
        if let Some(raw) = headers.get(name).and_then(|v| v.to_str().ok()) {
            match normalize_prefix(raw) {
                Some(p) => return p,
                None => warn_invalid_header_once(name, raw),
            }
        }
    }
    base_path.to_string()
}

/// Normaliza un prefijo candidato. Devuelve `None` si es inválido.
/// Acepta: `/` o vacío (⇒ raíz, `""`), o un path que empieza por `/`, sin `//`,
/// sin segmentos `.`/`..`, charset `[A-Za-z0-9._~/%-]`, ≤128 chars. Se tolera una
/// barra final (se recorta) porque algunos proxies la añaden.
pub fn normalize_prefix(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Some(String::new());
    }
    let stripped = trimmed.strip_suffix('/').unwrap_or(trimmed);
    if !stripped.starts_with('/') || stripped.len() > 128 {
        return None;
    }
    if !stripped
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '/' | '%'))
    {
        return None;
    }
    // Sin segmentos vacíos ("//") ni relativos ("." / ".."). `skip(1)`: el primer
    // split de un path que empieza por '/' es siempre vacío y es legítimo.
    if stripped
        .split('/')
        .skip(1)
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return None;
    }
    Some(stripped.to_string())
}

/// `FUTUREFIN_BASE_PATH` (opcional): prefijo fijo para despliegues tras un proxy
/// que no manda `X-Forwarded-Prefix`. Fail-loud como `FUTUREFIN_PUBLIC_URL`: un
/// valor presente pero inválido aborta el arranque en vez de servir HTML roto.
pub fn validate_base_path_env(raw: &str) -> String {
    normalize_prefix(raw).unwrap_or_else(|| {
        panic!(
            "invalid FUTUREFIN_BASE_PATH ({raw}): must start with '/', no '//', no '.'/'..' \
             segments, charset [A-Za-z0-9._~/%-], max 128 chars"
        )
    })
}

/// Política de peers de confianza (`FUTUREFIN_TRUSTED_PROXY_IPS`).
/// - Sin definir/vacía ⇒ `Disabled`: nadie es de confianza (el default seguro).
/// - `any` ⇒ `Any`: todo peer es de confianza — para tests y redes privadas donde
///   el proxy es el único camino hasta el proceso.
/// - Lista de IPs separadas por comas ⇒ `List` (el add-on usa `172.30.32.2`).
#[derive(Debug, Clone)]
pub enum PeerPolicy {
    Disabled,
    Any,
    List(Vec<IpAddr>),
}

impl PeerPolicy {
    /// Parse fail-loud (estilo `CORS_ORIGINS`): una entrada inválida aborta el arranque.
    pub fn from_env_value(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::Disabled;
        }
        if trimmed.eq_ignore_ascii_case("any") {
            return Self::Any;
        }
        let ips = trimmed
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<IpAddr>().unwrap_or_else(|_| {
                    panic!("invalid FUTUREFIN_TRUSTED_PROXY_IPS entry: {s}")
                })
            })
            .collect::<Vec<_>>();
        if ips.is_empty() {
            panic!("FUTUREFIN_TRUSTED_PROXY_IPS resolved empty — unset it or list IPs / 'any'");
        }
        Self::List(ips)
    }

    /// ¿Es este peer de confianza? Peer desconocido (`None`, p.ej. tests con
    /// `oneshot` sin `ConnectInfo`) solo pasa con `Any`.
    pub fn allows(&self, peer: Option<IpAddr>) -> bool {
        match self {
            Self::Disabled => false,
            Self::Any => true,
            Self::List(ips) => peer.is_some_and(|p| ips.contains(&p)),
        }
    }
}

/// `warn!` una vez por valor de header inválido distinto, acotado a 8 entradas para
/// que un atacante no convierta el log en un canal de flood.
fn warn_invalid_header_once(name: &str, raw: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut seen = seen.lock().expect("prefix warn set poisoned");
    if seen.len() < 8 && seen.insert(format!("{name}:{raw}")) {
        tracing::warn!(header = name, value = raw, "ignoring invalid proxy prefix header");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_valid_prefixes() {
        assert_eq!(normalize_prefix("/f").as_deref(), Some("/f"));
        assert_eq!(
            normalize_prefix("/api/hassio_ingress/AbC123-_~").as_deref(),
            Some("/api/hassio_ingress/AbC123-_~")
        );
        // Barra final tolerada (algunos proxies la añaden).
        assert_eq!(normalize_prefix("/finanzas/").as_deref(), Some("/finanzas"));
        // Raíz explícita y vacío = sin prefijo.
        assert_eq!(normalize_prefix("/").as_deref(), Some(""));
        assert_eq!(normalize_prefix("").as_deref(), Some(""));
        assert_eq!(normalize_prefix("  ").as_deref(), Some(""));
    }

    #[test]
    fn normalize_rejects_invalid_prefixes() {
        assert_eq!(normalize_prefix("sin-barra"), None);
        assert_eq!(normalize_prefix("/doble//barra"), None);
        assert_eq!(normalize_prefix("/punto/./x"), None);
        assert_eq!(normalize_prefix("/sube/../x"), None);
        assert_eq!(normalize_prefix("/con espacio"), None);
        assert_eq!(normalize_prefix("/con\"comilla"), None);
        assert_eq!(normalize_prefix("/con<angulo"), None);
        assert_eq!(normalize_prefix(&format!("/{}", "a".repeat(200))), None);
    }

    #[test]
    fn request_prefix_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PREFIX, "/fwd".parse().unwrap());
        headers.insert(X_INGRESS_PATH, "/api/hassio_ingress/tok".parse().unwrap());
        // X-Ingress-Path gana sobre X-Forwarded-Prefix.
        assert_eq!(request_prefix("/env", &headers), "/api/hassio_ingress/tok");
        // Sin ingress: gana X-Forwarded-Prefix sobre la env.
        headers.remove(X_INGRESS_PATH);
        assert_eq!(request_prefix("/env", &headers), "/fwd");
        // Sin headers: la env.
        headers.remove(X_FORWARDED_PREFIX);
        assert_eq!(request_prefix("/env", &headers), "/env");
        assert_eq!(request_prefix("", &headers), "");
    }

    #[test]
    fn request_prefix_ignores_invalid_header_and_falls_through() {
        let mut headers = HeaderMap::new();
        headers.insert(X_INGRESS_PATH, "no-empieza-por-barra".parse().unwrap());
        headers.insert(X_FORWARDED_PREFIX, "/valido".parse().unwrap());
        assert_eq!(request_prefix("", &headers), "/valido");
    }

    #[test]
    fn peer_policy_semantics() {
        let none = PeerPolicy::Disabled;
        let any = PeerPolicy::Any;
        let list = PeerPolicy::from_env_value("172.30.32.2, 10.0.0.1");
        let supervisor: IpAddr = "172.30.32.2".parse().unwrap();
        let other: IpAddr = "192.168.1.50".parse().unwrap();
        assert!(!none.allows(Some(supervisor)));
        assert!(!none.allows(None));
        assert!(any.allows(Some(other)));
        assert!(any.allows(None));
        assert!(list.allows(Some(supervisor)));
        assert!(!list.allows(Some(other)));
        assert!(!list.allows(None));
        assert!(matches!(PeerPolicy::from_env_value(""), PeerPolicy::Disabled));
        assert!(matches!(PeerPolicy::from_env_value("any"), PeerPolicy::Any));
    }
}
