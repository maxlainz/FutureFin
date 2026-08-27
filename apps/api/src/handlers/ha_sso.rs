//! «Entrar con Home Assistant» — `GET /v1/auth/ha/start` y `GET /v1/auth/ha/callback`.
//!
//! Un flujo de código de autorización contra el propio Home Assistant, que actúa de IdP. A
//! diferencia del SSO de cabeceras (`handlers/sso.rs`), aquí **no hace falta un proxy de
//! confianza**: la prueba de identidad no es una cabecera que alguien podría inventarse, sino
//! un round-trip por el navegador contra HA más un `state` de un solo uso guardado en una
//! cookie propia.
//!
//! Las dos rutas se montan SIEMPRE — la forma del router no depende del entorno —; lo que
//! decide es el estado (`AppState::ha_sso`).
//!
//! **Los errores no son JSON: son redirects.** El navegador está en mitad de una navegación de
//! nivel superior venida de HA, así que un 4xx con cuerpo dejaría a la persona mirando un JSON.
//! Se vuelve a `{prefijo}/?ha_error=<código>` y la SPA traduce el código. Los cinco códigos
//! existen además como mensajes de `ApiError` para que `tests/error_codes_parity.rs` los
//! recoja y el catálogo en español no se quede corto.

use crate::error::{ApiError, ErrorBody};
use crate::ha_idp::{
    apply_prefix, authorize_url, client_id_for, ct_eq, decode_state_cookie,
    encode_state_cookie, strip_prefix, StateCookie, HA_STATE_COOKIE,
};
use crate::handlers::auth::{establish_session, session_cookie_path};
use crate::handlers::sso::resolve_or_provision;
use crate::prefix::PeerIp;
use crate::state::AppState;
use axum::extract::{Extension, Query};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use cookie::{time::Duration as CookieDuration, SameSite};
use http::HeaderMap;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;

/// Vida de la cookie de estado. Diez minutos es lo que HA da a sus códigos de autorización:
/// más tiempo solo alarga la ventana en la que un `state` robado sigue sirviendo.
const STATE_TTL_SECONDS: i64 = 600;

/// Tope del `code` que devuelve HA (~64 caracteres reales). Un valor descomunal no es un código
/// de HA, y no vale la pena mandárselo de vuelta.
const MAX_CODE_LEN: usize = 512;

/// Los cinco finales posibles de este flujo que no son «sesión abierta».
///
/// Cada variante tiene UN mensaje canónico con prefijo `snake_code:`, y el código del redirect
/// se deriva de ese mismo mensaje: así el código que ve la SPA y el que recoge el test de
/// paridad no pueden divergir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HaLoginError {
    Disabled,
    StateMismatch,
    ExchangeFailed,
    IdentityFailed,
    UsernameUnavailable,
}

impl HaLoginError {
    fn message(self) -> &'static str {
        match self {
            Self::Disabled => {
                "ha_sso_disabled: this installation has no Home Assistant login configured"
            }
            Self::StateMismatch => {
                "ha_state_mismatch: the login state cookie is missing, expired or does not match"
            }
            Self::ExchangeFailed => {
                "ha_exchange_failed: Home Assistant did not accept the authorization code"
            }
            Self::IdentityFailed => {
                "ha_identity_failed: could not read the Home Assistant user identity"
            }
            // Mismo código que emite `handlers/sso.rs` para la misma situación: la persona se
            // topa con lo mismo entre por donde entre, y una sola frase la explica.
            Self::UsernameUnavailable => {
                "sso_username_unavailable: could not derive a free username for this identity"
            }
        }
    }

    /// El prefijo `snake_code:` del mensaje. Mismo criterio que `error::derive_error_code`.
    fn code(self) -> &'static str {
        self.message()
            .split_once(": ")
            .expect("todo mensaje de HaLoginError lleva su prefijo snake_code")
            .0
    }

    fn api_error(self) -> ApiError {
        ApiError::UnauthorizedWith(self.message().into())
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct HaStartQuery {
    /// Ruta de la app a la que volver tras entrar. Se sanea (ver `ha_idp::sanitize_next`);
    /// cualquier cosa que no sea una ruta de esta app cae a la raíz.
    pub next: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct HaCallbackQuery {
    pub code: Option<String>,
    /// El `state` que se le dio a HA al autorizar. Debe casar con el de la cookie.
    pub state: Option<String>,
    /// HA puede volver con `?error=access_denied` si la persona rechaza el permiso.
    pub error: Option<String>,
}

#[utoipa::path(
    get,
    path = "/v1/auth/ha/start",
    security(()),
    tag = "auth",
    params(HaStartQuery),
    responses(
        (status = 302, description = "Redirect a la pantalla de autorización de Home Assistant; cookie ff_ha_state puesta"),
        (status = 401, description = "Esta instalación no tiene configurado el login con Home Assistant (`ha_sso_disabled`)"),
    ),
    description = "Arranca «Entrar con Home Assistant». Solo existe dentro del add-on \
                   (`FUTUREFIN_HA_SSO_URL` + `FUTUREFIN_HA_ADDON=1`). No lleva credencial de \
                   FutureFin: la credencial se construye durante el propio flujo."
)]
pub async fn ha_start(
    Extension(state): Extension<Arc<AppState>>,
    PeerIp(peer): PeerIp,
    jar: CookieJar,
    headers: HeaderMap,
    Query(q): Query<HaStartQuery>,
) -> Result<(CookieJar, Response), ApiError> {
    let Some(ha) = state.ha_sso.as_ref() else {
        return Err(HaLoginError::Disabled.api_error());
    };

    // El origen se congela AQUÍ y viaja en la cookie: HA compara el `client_id` del canje con
    // el de la autorización **byte a byte**, y las cabeceras de la segunda petición podrían
    // derivar otro (otro `X-Forwarded-Host`, otro puerto…). Recalcularlo sería la forma más
    // silenciosa de romper el flujo.
    //
    // Sin código granular a propósito: el único fallo posible es un `Host` ausente o deforme,
    // que no es una situación que el usuario pueda resolver ni que la SPA deba traducir.
    let origin = crate::oauth::url::public_base_url(&state, &headers)
        .map_err(|_| ApiError::BadRequest("missing or malformed Host header".into()))?;

    let prefix = state.request_prefix(&headers);
    // En la cookie va la forma CANÓNICA del `next`: sin prefijo. El prefijo lo pone el
    // callback, que es quien conoce el de SU request — guardarlo ya prefijado obligaría a
    // adivinar si el de vuelta es el mismo.
    let next = strip_prefix(&prefix, &crate::ha_idp::sanitize_next(&prefix, q.next.as_deref()));
    let nonce = uuid::Uuid::new_v4().simple().to_string();

    let value = encode_state_cookie(&StateCookie {
        nonce: nonce.clone(),
        origin: origin.clone(),
        next,
    });
    let jar = jar.add(state_cookie(&state, value, cookie_path(&state, peer, &headers)));

    Ok((jar, redirect(&authorize_url(&ha.base_url, &origin, &nonce))))
}

#[utoipa::path(
    get,
    path = "/v1/auth/ha/callback",
    security(()),
    tag = "auth",
    params(HaCallbackQuery),
    responses(
        (status = 302, description = "Sesión abierta (cookie ff_session) y vuelta a la app; o vuelta a `/?ha_error=<código>` si algo falló"),
    ),
    description = "Vuelta del navegador desde Home Assistant. La credencial es el propio \
                   round-trip más la cookie `ff_ha_state` de un solo uso — no se puede \
                   expresar como securityScheme. Los fallos NO devuelven JSON: redirigen a \
                   `/?ha_error=<código>`."
)]
pub async fn ha_callback(
    Extension(state): Extension<Arc<AppState>>,
    PeerIp(peer): PeerIp,
    jar: CookieJar,
    headers: HeaderMap,
    Query(q): Query<HaCallbackQuery>,
) -> Result<(CookieJar, Response), ApiError> {
    let path = cookie_path(&state, peer, &headers);
    let prefix = state.request_prefix(&headers);
    // Se LEE antes de retirarla: `CookieJar::remove` la borra también de la vista local.
    let incoming = jar_state(&jar);
    // La cookie se retira SIEMPRE, pase lo que pase: es de un solo uso, y dejarla viva tras un
    // fallo permitiría reintentar el mismo `state` (test: `cookie_single_use_replay`).
    let jar = jar.remove(removal_cookie(path.clone()));
    let fail = |e: HaLoginError| Ok((jar.clone(), error_redirect(&prefix, e)));

    // 1-2. El `state` es la única prueba de que este callback pertenece a este navegador.
    let Some(cookie) = incoming else {
        return fail(HaLoginError::StateMismatch);
    };
    let Some(returned) = q.state.as_deref() else {
        return fail(HaLoginError::StateMismatch);
    };
    if !ct_eq(&cookie.nonce, returned) {
        return fail(HaLoginError::StateMismatch);
    }

    // 3. Solo después de validar el estado se mira la configuración: un callback sin cookie es
    //    un callback ajeno, y no merece saber si esta instalación tiene HA configurado.
    let Some(ha) = state.ha_sso.as_ref() else {
        return fail(HaLoginError::Disabled);
    };

    // 4. HA puede volver con `?error=access_denied` (la persona rechazó el permiso). No hay
    //    código que canjear, así que no se llama al proveedor.
    if let Some(err) = q.error.as_deref() {
        tracing::info!(ha_error = err, "home assistant rechazó la autorización");
        return fail(HaLoginError::ExchangeFailed);
    }
    let Some(code) = q.code.as_deref().filter(|c| !c.is_empty() && c.len() <= MAX_CODE_LEN) else {
        return fail(HaLoginError::ExchangeFailed);
    };

    // 5. Canje. El `client_id` sale del origen CONGELADO en la cookie.
    let tokens = match ha
        .idp
        .exchange_code(code, &client_id_for(&cookie.origin))
        .await
    {
        Ok(t) => t,
        Err(_) => return fail(HaLoginError::ExchangeFailed),
    };

    // 6. Identidad.
    let identity = match ha.idp.identity(&tokens.access_token).await {
        Ok(i) => i,
        Err(_) => return fail(HaLoginError::IdentityFailed),
    };

    // 7. Revocar ANTES de tocar la base de datos: el refresh token es una credencial de larga
    //    vida sobre el HA de la persona y aquí ya no hace falta para nada. Si la revocación
    //    falla se registra y se sigue — un login probado no se tira por higiene.
    if let Some(refresh) = tokens.refresh_token.as_deref() {
        ha.idp.revoke(refresh).await;
    }

    // 8. La MISMA función de alta que el SSO de cabeceras, con el MISMO `external_user_id`:
    //    entrar por un camino o por el otro lleva a la misma fila de `users`.
    let row = match resolve_or_provision(&state, identity.external_user_id, &identity.name).await {
        Ok(row) => row,
        Err(e) => {
            if ErrorBody::from_api_error(&e).code == HaLoginError::UsernameUnavailable.code() {
                return fail(HaLoginError::UsernameUnavailable);
            }
            // Todo lo demás (fallo de BD, carrera de instalación) es un error de servidor de
            // verdad: un redirect a la pantalla de login lo escondería.
            return Err(e);
        }
    };

    // 9. A partir de aquí es un login corriente: fila en `sessions`, cookie acotada al prefijo
    //    y warm-up en background (D7).
    let (jar, _profile) = establish_session(&state, jar, path, row).await?;
    Ok((jar, redirect(&apply_prefix(&prefix, &cookie.next))))
}

/// Redirect de fallo: de vuelta a la raíz de la app con el código en la query. Sin `no-store`
/// un caché intermedio podría servir el redirect de error a un intento posterior que sí va bien.
fn error_redirect(prefix: &str, e: HaLoginError) -> Response {
    tracing::info!(code = e.code(), "login con Home Assistant fallido");
    redirect(&format!("{}/?ha_error={}", prefix, e.code()))
}

/// 302 explícito, no `axum::response::Redirect::to` (que emite 303 See Other).
///
/// 302 es lo que emiten los flujos OAuth y lo que este contrato fija; y 303 tiene además la
/// semántica de «convierte el método en GET», que aquí no hace falta porque ya es GET.
///
/// `no-store` en las dos ramas (éxito y error): sin él, un caché intermedio podría servir el
/// redirect de un intento a otro — y el de éxito lleva un `Set-Cookie` de sesión.
fn redirect(location: &str) -> Response {
    let value = http::HeaderValue::from_str(location)
        // Imposible por construcción (`sanitize_next` prohíbe los controles), pero un
        // `Location` inválido no puede tumbar el handler.
        .unwrap_or_else(|_| http::HeaderValue::from_static("/"));
    let mut response = (http::StatusCode::FOUND, axum::body::Body::empty()).into_response();
    let h = response.headers_mut();
    h.insert(http::header::LOCATION, value);
    h.insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store"),
    );
    response
}

fn jar_state(jar: &CookieJar) -> Option<StateCookie> {
    decode_state_cookie(jar.get(HA_STATE_COOKIE)?.value())
}

/// `Path` de la cookie de estado. El MISMO que el de la sesión (`session_cookie_path`): bajo el
/// Ingress todos los add-ons comparten origen, y una cookie con `Path=/` viajaría a los demás.
/// Compartir el helper garantiza además que el borrado del callback case con lo que puso el
/// `/start` — un `Set-Cookie` de borrado con otro `Path` no casa con la cookie viva.
fn cookie_path(state: &AppState, peer: Option<std::net::IpAddr>, headers: &HeaderMap) -> String {
    session_cookie_path(state, peer, headers)
}

/// La cookie de estado.
///
/// `SameSite=Lax` es OBLIGATORIO, no una preferencia: el callback llega como una navegación de
/// nivel superior desde el dominio de Home Assistant, y `Strict` NO manda la cookie en una
/// navegación cross-site — el flujo fallaría siempre con `ha_state_mismatch`. `None` exigiría
/// `Secure`, que no se puede dar por hecho en una LAN por http.
fn state_cookie(state: &AppState, value: String, path: String) -> Cookie<'static> {
    Cookie::build((HA_STATE_COOKIE, value))
        .path(path)
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::seconds(STATE_TTL_SECONDS))
        .secure(state.cookie_secure)
        .build()
}

fn removal_cookie(path: String) -> Cookie<'static> {
    Cookie::build((HA_STATE_COOKIE, "")).path(path).build()
}
