//! «Entrar con Home Assistant» — HA como proveedor de identidad estilo OAuth (4.3.1).
//!
//! Este módulo es la parte **pura y testeable** del flujo: el trait que abstrae al proveedor,
//! la construcción de la URL de autorización, el códec de la cookie de estado y el saneado del
//! `next`. El cliente real (HTTP + WebSocket) vive en `client.rs`; los handlers, en
//! `handlers/ha_sso.rs`.
//!
//! **Por qué existe además del SSO de cabeceras** (`handlers/sso.rs`): aquel solo funciona
//! *dentro* del Ingress del Supervisor, que ya autenticó a la persona y pone
//! `X-Remote-User-Id`. Este funciona **donde no hay proxy de confianza** — el add-on abierto en
//! una pestaña normal, `http://homeassistant.local:8123` de por medio — porque la prueba de
//! identidad no es una cabecera sino un round-trip por el navegador contra el propio HA.
//!
//! **La piedra angular de la paridad**: el `result.id` que devuelve HA es
//! `uuid4().hex` — 32 hexadecimales **sin guiones** —, y `Uuid::parse_str` acepta esa forma
//! «simple» igual que la canónica. Es exactamente la misma normalización que hace
//! `handlers/sso.rs::external_identity` con `X-Remote-User-Id`, así que la MISMA persona entra
//! por los dos caminos a la MISMA fila de `users`. Si alguien cambia una de las dos
//! normalizaciones, el usuario se duplica en silencio y su hogar se parte en dos.
//!
//! ## Notas de dependencias (leer antes de tocar `Cargo.toml`)
//!
//! - `reqwest` y `tokio-tungstenite` van con **rustls**, no con `default-tls`: la imagen
//!   runtime no lleva OpenSSL. La variante `rustls-tls-webpki-roots` empaqueta el almacén de
//!   certificados, así que tampoco depende del `ca-certificates` del sistema.
//! - Una sola major de rustls en el árbol — gate: `cargo tree -d | grep -i rustls` no debe
//!   listar `rustls` duplicado.
//! - `tokio-tungstenite` y no `async-tungstenite`: este repo es tokio-only.
//! - **Nunca** `danger_accept_invalid_certs`: un HA con certificado autofirmado no está
//!   soportado por diseño (se accede por http en la LAN, o con un certificado válido).

pub mod client;

use base64::Engine as _;
use uuid::Uuid;

/// Identidad devuelta por Home Assistant (`auth/current_user` sobre el WebSocket).
///
/// `external_user_id` es el MISMO `User.id` que el Supervisor pone en `X-Remote-User-Id`, y
/// por tanto la misma fila de `users` (columna `external_user_id`). Ver la nota de paridad
/// de la cabecera del módulo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaIdentity {
    pub external_user_id: Uuid,
    pub name: String,
}

/// Tokens del canje del código. El `refresh_token` es opcional: HA lo emite en el flujo de
/// código de autorización, pero no se depende de que esté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

/// Por qué falló una llamada al proveedor. Deliberadamente **grueso**: lo que el usuario ve es
/// un código de redirect, no un diagnóstico, y detallar más solo daría a un atacante una sonda
/// sobre la instalación de HA de la víctima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaIdpError {
    /// El canje del código falló (HA respondió != 2xx, o el cuerpo no era el esperado).
    Exchange,
    /// El canje fue bien pero la identidad no se pudo obtener (auth rechazada, respuesta
    /// inesperada, `success: false`).
    Identity,
    /// No se pudo hablar con HA (DNS, TCP, TLS, timeout).
    Transport,
}

/// El proveedor de identidad de Home Assistant, abstraído para poder falsificarlo en los tests
/// de integración sin levantar un HA (`tests/common::FakeHaIdp`).
#[async_trait::async_trait]
pub trait HaIdp: Send + Sync {
    /// Canjea el `code` del redirect por tokens. `client_id` debe ser **byte a byte** el mismo
    /// que se envió al autorizar: HA indexa su almacén de códigos por esa cadena cruda.
    async fn exchange_code(&self, code: &str, client_id: &str) -> Result<HaTokens, HaIdpError>;
    /// Resuelve la identidad del dueño del `access_token`.
    async fn identity(&self, access_token: &str) -> Result<HaIdentity, HaIdpError>;
    /// Revoca el refresh token. Best-effort e **infalible por firma**: un fallo aquí no puede
    /// impedir un login que ya está probado (se registra en el log y se sigue).
    async fn revoke(&self, refresh_token: &str);
}

/// URL de autorización de HA.
///
/// Se construye con `url::Url` y `query_pairs_mut` — nunca concatenando: `origin` viene de las
/// cabeceras del request y `nonce` es generado, pero la regla del repo es que ningún redirect
/// se arma a mano (de ahí nacen los open-redirect).
///
/// Detalles del protocolo de HA (verificados contra su código):
/// - `client_id` = `{origin}/` **con** la barra final, y `redirect_uri` del mismo origen. HA
///   exige que sean del mismo origen, y en ese caso **no** hace fetch de nuestra URL.
/// - Nada de PKCE (HA lo ignora), ni `client_secret`, ni `scope`, ni `response_type`.
///
/// `ha_base` ya pasó la validación de arranque (`FUTUREFIN_HA_SSO_URL` = origen desnudo), así
/// que el parse no puede fallar; si alguien llama con basura, es un bug de programación.
pub fn authorize_url(ha_base: &str, origin: &str, nonce: &str) -> String {
    let mut url = url::Url::parse(&format!("{}/auth/authorize", ha_base.trim_end_matches('/')))
        .expect("FUTUREFIN_HA_SSO_URL ya validada al arrancar");
    url.query_pairs_mut()
        .append_pair("client_id", &client_id_for(origin))
        .append_pair("redirect_uri", &redirect_uri_for(origin))
        .append_pair("state", nonce);
    url.to_string()
}

/// `client_id` canónico de esta instalación: el origen público **con barra final**. Punto único
/// para que autorización y canje no puedan divergir (HA compara la cadena cruda).
pub fn client_id_for(origin: &str) -> String {
    format!("{}/", origin.trim_end_matches('/'))
}

/// `redirect_uri` de esta instalación. Mismo origen que el `client_id`, que es lo que evita
/// que HA vaya a buscar nuestro HTML.
pub fn redirect_uri_for(origin: &str) -> String {
    format!("{}/v1/auth/ha/callback", origin.trim_end_matches('/'))
}

/// URL del WebSocket de HA a partir de su origen HTTP (`https` → `wss`, `http` → `ws`).
/// La identidad SOLO se puede leer por WebSocket: HA no expone `auth/current_user` por REST.
pub fn ws_url_from_base(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let rest = if let Some(r) = base.strip_prefix("https://") {
        return format!("wss://{r}/api/websocket");
    } else if let Some(r) = base.strip_prefix("http://") {
        r
    } else {
        base
    };
    format!("ws://{rest}/api/websocket")
}

// ---------------------------------------------------------------------------------------
// Cookie de estado
// ---------------------------------------------------------------------------------------

/// Nombre de la cookie que ata el `/start` con su `/callback`.
pub const HA_STATE_COOKIE: &str = "ff_ha_state";

/// Tope del valor de la cookie. Un valor más largo es basura o un intento de inflar la
/// petición: se rechaza antes de decodificar nada.
const STATE_COOKIE_MAX: usize = 2048;

/// Lo que el `/start` necesita recordar hasta que vuelve el navegador.
///
/// - `nonce`: se compara con el `state` que devuelve HA (anti-CSRF). Es la ÚNICA prueba de que
///   este callback pertenece a este navegador.
/// - `origin`: el origen público con el que se construyó el `client_id`. Se guarda en vez de
///   recalcularlo porque HA exige la MISMA cadena al canjear, y las cabeceras de la segunda
///   petición podrían derivar otra.
/// - `next`: ruta de la app a la que volver, **sin prefijo** (forma canónica; el prefijo se
///   aplica en el callback, que es quien conoce el de SU request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCookie {
    pub nonce: String,
    pub origin: String,
    pub next: String,
}

/// `1.<nonce>.<b64url(origin)>.<b64url(next)>`.
///
/// Base64url **sin padding** para que el valor quepa en una cookie sin comillas (`=` no está en
/// el charset de un cookie-value sin entrecomillar). El `1.` es la versión: un formato futuro
/// puede coexistir con las cookies vivas de 10 minutos en vez de reventarlas.
pub fn encode_state_cookie(state: &StateCookie) -> String {
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "1.{}.{}.{}",
        state.nonce,
        b64.encode(state.origin.as_bytes()),
        b64.encode(state.next.as_bytes())
    )
}

/// Inversa de `encode_state_cookie`. `None` ante cualquier desviación: versión distinta, número
/// de segmentos distinto, tamaño excesivo, base64 inválido o bytes que no son UTF-8.
pub fn decode_state_cookie(raw: &str) -> Option<StateCookie> {
    if raw.len() > STATE_COOKIE_MAX {
        return None;
    }
    let mut parts = raw.split('.');
    if parts.next()? != "1" {
        return None;
    }
    let nonce = parts.next()?;
    let origin = parts.next()?;
    let next = parts.next()?;
    if parts.next().is_some() || nonce.is_empty() {
        return None;
    }
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let origin = String::from_utf8(b64.decode(origin).ok()?).ok()?;
    let next = String::from_utf8(b64.decode(next).ok()?).ok()?;
    Some(StateCookie {
        nonce: nonce.to_string(),
        origin,
        next,
    })
}

/// Comparación en tiempo constante. El `state` es un secreto de un solo uso; compararlo con
/// `==` filtra por el reloj cuántos caracteres iniciales acertó quien lo intenta.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------------------
// `next` — a dónde volver tras el login
// ---------------------------------------------------------------------------------------

/// Tope del `next`. 512 caracteres cubren de sobra `/oauth/authorize?...` con su `client_id`.
const NEXT_MAX: usize = 512;

/// Saneado de la ruta de retorno **sin prefijo** (forma canónica de la cookie).
///
/// Un `next` es un open-redirect esperando a ocurrir, así que se acepta por lista blanca de
/// forma y cualquier duda cae a `/`. Reglas, cada una con su porqué:
///
/// 1. **Debe empezar por `/`** — solo rutas de esta app, nunca una URL absoluta.
/// 2. **No `//…`** — `//evil.test` es una URL *protocol-relative*: el navegador la resuelve
///    como otro host aunque empiece por barra.
/// 3. **Ningún `\` en ninguna posición** — varios navegadores tratan `\` como `/`, así que
///    `/\evil.test` es `//evil.test` para ellos (y de paso cubre `/\` inicial).
/// 4. **Ni `://` ni `@` en la parte de PATH** (antes del primer `?`) — son las dos piezas con
///    las que se construye una autoridad (`//user@host`). En la **query** ambas se permiten a
///    propósito: `/oauth/authorize?client_id=https://claude.ai&state=y` es un retorno legítimo
///    de esta misma app, y prohibirlas ahí rompería la pantalla de consentimiento OAuth. Una
///    query no puede cambiar el destino de un `Location` que ya empieza por `/`.
/// 6. **Ningún carácter de control** en ninguna posición — un `\r\n` en un `Location:` parte la
///    respuesta en dos.
/// 7. **Fragmento descartado** — el `#` nunca llega al servidor; guardarlo solo ocupa cookie.
/// 8. **≤ 512 caracteres**.
///
/// Cualquier violación ⇒ `/`.
pub fn canonical_next(raw: Option<&str>) -> String {
    let raw = raw.map(str::trim).unwrap_or("");
    // El fragmento se recorta ANTES de medir: no viaja al servidor y no debe gastar presupuesto.
    let candidate = raw.split('#').next().unwrap_or("");
    if candidate.is_empty() || candidate.len() > NEXT_MAX {
        return "/".to_string();
    }
    if !candidate.starts_with('/') || candidate.starts_with("//") {
        return "/".to_string();
    }
    if candidate.contains('\\') {
        return "/".to_string();
    }
    if candidate.chars().any(|c| c.is_control()) {
        return "/".to_string();
    }
    let path_part = candidate.split('?').next().unwrap_or("");
    if path_part.contains('@') || path_part.contains("://") {
        return "/".to_string();
    }
    candidate.to_string()
}

/// Ruta de retorno **con** el prefijo público aplicado, idempotente: si el cliente ya mandó el
/// `next` prefijado (la SPA lo hace, porque es lo que ve en la barra del navegador) no se
/// duplica el prefijo.
///
/// Con `prefix` vacío es la identidad sobre `canonical_next`.
pub fn sanitize_next(prefix: &str, raw: Option<&str>) -> String {
    let next = canonical_next(raw);
    apply_prefix(prefix, &strip_prefix(prefix, &next))
}

/// Quita el prefijo público de una ruta ya saneada. `/ff/movimientos` → `/movimientos`;
/// `/ff` → `/`. Sin prefijo, la identidad.
pub fn strip_prefix(prefix: &str, next: &str) -> String {
    if prefix.is_empty() {
        return next.to_string();
    }
    if next == prefix {
        return "/".to_string();
    }
    match next.strip_prefix(prefix) {
        Some(rest) if rest.starts_with('/') => rest.to_string(),
        _ => next.to_string(),
    }
}

/// Pega el prefijo público a una ruta canónica (que siempre empieza por `/`).
pub fn apply_prefix(prefix: &str, next: &str) -> String {
    if prefix.is_empty() {
        next.to_string()
    } else {
        format!("{prefix}{next}")
    }
}

/// ¿Ofrece esta instalación «Entrar con Home Assistant»? Predicado **único**: lo consultan el
/// handler de `/start` y `handlers/spa.rs` para decidir `window.__FF_HA_LOGIN__`.
///
/// A diferencia del SSO de cabeceras (`sso::sso_available`), **no depende del peer ni de
/// ninguna cabecera**: el botón existe precisamente donde NO hay proxy de confianza — el add-on
/// abierto fuera del Ingress —, así que su única condición es que el operador haya configurado
/// el origen público de HA.
pub fn ha_login_available(state: &crate::state::AppState) -> bool {
    state.ha_sso.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://futurefin.test";
    const HA: &str = "https://ha.example.org";

    #[test]
    fn authorize_url_carries_exactly_the_three_params_ha_expects() {
        let url = authorize_url(HA, ORIGIN, "abc123");
        let parsed = url::Url::parse(&url).expect("url válida");
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("ha.example.org"));
        assert_eq!(parsed.path(), "/auth/authorize");
        assert_eq!(parsed.fragment(), None);
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("client_id".into(), "http://futurefin.test/".to_string()),
                (
                    "redirect_uri".into(),
                    "http://futurefin.test/v1/auth/ha/callback".to_string()
                ),
                ("state".into(), "abc123".to_string()),
            ]
        );
        // Percent-encoding real en el texto de la URL (no concatenación cruda).
        assert!(url.contains("client_id=http%3A%2F%2Ffuturefin.test%2F"), "{url}");
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Ffuturefin.test%2Fv1%2Fauth%2Fha%2Fcallback"),
            "{url}"
        );
    }

    #[test]
    fn authorize_url_tolerates_a_trailing_slash_in_the_base() {
        let a = authorize_url("https://ha.example.org", ORIGIN, "n");
        let b = authorize_url("https://ha.example.org/", ORIGIN, "n");
        assert_eq!(a, b);
    }

    #[test]
    fn ws_url_maps_the_scheme() {
        assert_eq!(
            ws_url_from_base("https://ha.example.org"),
            "wss://ha.example.org/api/websocket"
        );
        assert_eq!(
            ws_url_from_base("http://homeassistant.local:8123"),
            "ws://homeassistant.local:8123/api/websocket"
        );
        // Barra final tolerada: no se duplica.
        assert_eq!(
            ws_url_from_base("http://ha.local:8123/"),
            "ws://ha.local:8123/api/websocket"
        );
    }

    #[test]
    fn state_cookie_roundtrips() {
        let original = StateCookie {
            nonce: "0123456789abcdef0123456789abcdef".into(),
            origin: "https://finanzas.example.org:8443".into(),
            next: "/oauth/authorize?client_id=https://claude.ai&state=x".into(),
        };
        let encoded = encode_state_cookie(&original);
        // Charset apto para un cookie-value sin comillas.
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
            "{encoded}"
        );
        assert_eq!(decode_state_cookie(&encoded).as_ref(), Some(&original));
    }

    #[test]
    fn state_cookie_rejects_garbage() {
        assert_eq!(decode_state_cookie(""), None);
        assert_eq!(decode_state_cookie("basura"), None);
        // Versión desconocida.
        let ok = encode_state_cookie(&StateCookie {
            nonce: "n".into(),
            origin: "o".into(),
            next: "/".into(),
        });
        assert_eq!(decode_state_cookie(&ok.replacen("1.", "2.", 1)), None);
        // Segmentos de más y de menos.
        assert_eq!(decode_state_cookie(&format!("{ok}.extra")), None);
        assert_eq!(decode_state_cookie("1.nonce.b3JpZ2Vu"), None);
        // Base64 inválido.
        assert_eq!(decode_state_cookie("1.nonce.!!!.!!!"), None);
        // Nonce vacío.
        assert_eq!(decode_state_cookie("1..b3JpZ2Vu.Lw"), None);
        // Desmesurado.
        assert_eq!(decode_state_cookie(&format!("1.n.b3JpZ2Vu.{}", "A".repeat(4000))), None);
    }

    #[test]
    fn ct_eq_agrees_with_equality() {
        assert!(ct_eq("abc", "abc"));
        assert!(ct_eq("", ""));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "ab"));
        assert!(!ct_eq("", "a"));
    }

    #[test]
    fn sanitize_next_accepts_app_paths_and_preserves_the_query() {
        assert_eq!(sanitize_next("", Some("/movimientos")), "/movimientos");
        // Un retorno legítimo de esta misma app: la query lleva una URL entera con `://` y `@`.
        // Lo que NO puede es estar en la parte de path.
        assert_eq!(
            sanitize_next("", Some("/oauth/authorize?client_id=https://x&state=y")),
            "/oauth/authorize?client_id=https://x&state=y"
        );
        assert_eq!(
            sanitize_next("", Some("/oauth/authorize?u=a@b&state=y")),
            "/oauth/authorize?u=a@b&state=y"
        );
    }

    #[test]
    fn sanitize_next_refuses_everything_that_leaves_the_app() {
        for hostile in [
            "https://evil.test",
            "//evil.test",
            "/\\evil.test",
            "/ruta\\rara",
            "javascript:alert(1)",
            "/https://evil.test",
            "/@evil.test/path",
            "sin-barra",
            "",
            " ",
        ] {
            assert_eq!(sanitize_next("", Some(hostile)), "/", "{hostile:?} pasó el filtro");
        }
        // Controles (un `\r\n` partiría la cabecera `Location`).
        assert_eq!(sanitize_next("", Some("/ok\r\nX-Evil: 1")), "/");
        assert_eq!(sanitize_next("", Some("/ok\u{0}")), "/");
        // Longitud.
        assert_eq!(sanitize_next("", Some(&format!("/{}", "a".repeat(1024)))), "/");
        // Sin `next` en absoluto.
        assert_eq!(sanitize_next("", None), "/");
        // El fragmento se descarta, no invalida.
        assert_eq!(sanitize_next("", Some("/resumen#kpi")), "/resumen");
    }

    #[test]
    fn sanitize_next_prefixes_once_and_only_once() {
        assert_eq!(sanitize_next("/ff", Some("/movimientos")), "/ff/movimientos");
        // Ya venía prefijado (es lo que la SPA lee de la barra del navegador).
        assert_eq!(sanitize_next("/ff", Some("/ff/movimientos")), "/ff/movimientos");
        // El prefijo pelado es la raíz de la app.
        assert_eq!(sanitize_next("/ff", Some("/ff")), "/ff/");
        assert_eq!(sanitize_next("/ff", None), "/ff/");
        // Un prefijo que es prefijo de texto pero no de ruta NO se recorta.
        assert_eq!(sanitize_next("/ff", Some("/ffx")), "/ff/ffx");
        // Hostil bajo prefijo: cae a la raíz de la app, no a la del dominio.
        assert_eq!(sanitize_next("/ff", Some("//evil.test")), "/ff/");
    }

    #[test]
    fn canonical_next_is_prefix_free() {
        assert_eq!(canonical_next(Some("/movimientos")), "/movimientos");
        assert_eq!(strip_prefix("/ff", "/ff/movimientos"), "/movimientos");
        assert_eq!(strip_prefix("/ff", "/ff"), "/");
        assert_eq!(strip_prefix("", "/movimientos"), "/movimientos");
        assert_eq!(apply_prefix("/ff", "/movimientos"), "/ff/movimientos");
        assert_eq!(apply_prefix("", "/movimientos"), "/movimientos");
    }

    /// La piedra angular: el id de HA viene en forma «simple» (32 hex sin guiones) y
    /// `Uuid::parse_str` lo normaliza al MISMO UUID que la forma canónica que llega por
    /// `X-Remote-User-Id`. Sin esto, la misma persona sería dos filas de `users`.
    #[test]
    fn ha_user_id_without_dashes_parses_to_the_same_uuid() {
        let simple = "1234567890abcdef1234567890abcdef";
        let hyphenated = "12345678-90ab-cdef-1234-567890abcdef";
        assert_eq!(
            Uuid::parse_str(simple).expect("forma simple"),
            Uuid::parse_str(hyphenated).expect("forma canónica")
        );
    }
}
