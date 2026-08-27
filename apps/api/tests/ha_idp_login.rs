//! «Entrar con Home Assistant» (`GET /v1/auth/ha/start` + `/v1/auth/ha/callback`).
//!
//! Lo que estos tests fijan, en orden de importancia:
//!  1. **La puerta está cerrada por defecto.** Las rutas se montan siempre, pero sin
//!     `FUTUREFIN_HA_SSO_URL` responden `ha_sso_disabled` y no provisionan nada.
//!  2. **Paridad con el SSO de cabeceras.** El mismo `User.id` de Home Assistant —en su forma
//!     de 32 hex sin guiones— entra por los dos caminos a la MISMA fila de `users`. Si esto se
//!     rompe, la persona se duplica en silencio y su hogar se parte en dos.
//!  3. **El `state` es la frontera de seguridad entera.** Sin cookie, con cookie ajena o con
//!     cookie repetida no se llama siquiera al proveedor.
//!  4. **El orden de las llamadas**: canje → identidad → revocación, y la revocación ANTES de
//!     tocar la base de datos.
//!  5. **El `next` no es un open-redirect.**

mod common;

use common::{FakeCall, FakeHaIdp, TestApp, TestConfig};
use futurefin_api::ha_idp::{decode_state_cookie, HA_STATE_COOKIE};
use std::sync::Arc;
use uuid::Uuid;

/// El id tal y como lo devuelve Home Assistant: `uuid4().hex`, 32 hexadecimales SIN guiones.
const HA_ID_HEX: &str = "1234567890abcdef1234567890abcdef";
const HA_ID_HYPHENATED: &str = "12345678-90ab-cdef-1234-567890abcdef";

fn ha_uuid() -> Uuid {
    Uuid::parse_str(HA_ID_HEX).expect("uuid en forma simple")
}

async fn app_with(fake: Arc<FakeHaIdp>) -> TestApp {
    TestApp::spawn_with(TestConfig {
        ha_idp: Some(fake),
        ..Default::default()
    })
    .await
}

/// Arranca el flujo y devuelve `(cookie completa para reenviar, state, next guardado)`.
async fn start(app: &TestApp, next: Option<&str>) -> (String, String, String) {
    let uri = match next {
        Some(n) => format!("/v1/auth/ha/start?next={}", urlencode(n)),
        None => "/v1/auth/ha/start".to_string(),
    };
    let r = app.get(&uri).await;
    assert_eq!(r.status, http::StatusCode::FOUND, "{r:?}");
    let value = r
        .cookie_value(HA_STATE_COOKIE)
        .expect("/start pone la cookie de estado");
    let decoded = decode_state_cookie(&value).expect("la cookie decodifica");
    (
        format!("{HA_STATE_COOKIE}={value}"),
        decoded.nonce,
        decoded.next,
    )
}

/// Percent-encoding mínimo para meter un `next` en la query.
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn callback(app: &TestApp, cookie: Option<&str>, query: &str) -> common::ResponseParts {
    let uri = format!("/v1/auth/ha/callback?{query}");
    match cookie {
        Some(c) => app.get_with_cookie(&uri, c).await,
        None => app.get(&uri).await,
    }
}

fn assert_error_redirect(r: &common::ResponseParts, code: &str) {
    assert_eq!(r.status, http::StatusCode::FOUND, "{r:?}");
    assert_eq!(
        r.location().as_deref(),
        Some(format!("/?ha_error={code}").as_str()),
        "{r:?}"
    );
    assert!(r.session_cookie().is_none(), "un fallo no abre sesión: {r:?}");
}

// ---------------------------------------------------------------------------------------
// 1. Apagado por defecto
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn ha_login_is_off_by_default_but_the_route_exists() {
    let app = TestApp::spawn().await;
    let r = app.get("/v1/auth/ha/start").await;
    // Existe (no es un 404 del fallback de /v1) y dice por qué no funciona.
    assert_eq!(r.status, http::StatusCode::UNAUTHORIZED, "{r:?}");
    assert_eq!(r.json()["code"], "ha_sso_disabled");
    assert!(r.set_cookie(HA_STATE_COOKIE).is_none(), "no pone cookie: {r:?}");
    assert_eq!(app.count_rows("users").await, 0);
}

// ---------------------------------------------------------------------------------------
// 2. Forma del redirect de arranque
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn start_redirects_to_home_assistant_with_the_exact_parameters() {
    let app = app_with(FakeHaIdp::happy(ha_uuid(), "María")).await;
    let r = app.get("/v1/auth/ha/start").await;
    assert_eq!(r.status, http::StatusCode::FOUND, "{r:?}");
    assert_eq!(
        r.headers.get(http::header::CACHE_CONTROL).unwrap(),
        "no-store"
    );

    let location = url::Url::parse(&r.location().expect("Location")).expect("URL válida");
    assert_eq!(location.host_str(), Some("ha.test"));
    assert_eq!(location.path(), "/auth/authorize");
    let params: std::collections::HashMap<_, _> = location
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    // El `Host` que inyecta el harness es `futurefin.test` sobre http.
    assert_eq!(params["client_id"], "http://futurefin.test/");
    assert_eq!(
        params["redirect_uri"],
        "http://futurefin.test/v1/auth/ha/callback"
    );
    let state = &params["state"];
    assert_eq!(state.len(), 32, "el nonce es un uuid4 en forma simple: {state}");
    assert!(state.chars().all(|c| c.is_ascii_hexdigit()), "{state}");

    // La cookie: HttpOnly, Lax, acotada y de vida corta.
    let raw = r.set_cookie(HA_STATE_COOKIE).expect("Set-Cookie de estado");
    assert!(raw.contains("HttpOnly"), "{raw}");
    assert!(raw.contains("SameSite=Lax"), "{raw}");
    assert!(raw.contains("Max-Age=600"), "{raw}");
    assert!(raw.contains("Path=/"), "{raw}");
    assert!(!raw.contains("Secure"), "cookie_secure=false en tests: {raw}");
    // Y lleva dentro el mismo nonce que viajó a HA.
    let decoded = decode_state_cookie(&r.cookie_value(HA_STATE_COOKIE).unwrap()).unwrap();
    assert_eq!(&decoded.nonce, state);
    assert_eq!(decoded.origin, "http://futurefin.test");
}

#[tokio::test]
async fn under_a_base_path_the_cookie_is_scoped_without_polluting_the_client_id() {
    // El prefijo acota la cookie (bajo Ingress todos los add-ons comparten origen), pero el
    // `client_id` es el ORIGEN: meterle el prefijo lo desalinearía del que HA verá al canjear.
    let app = TestApp::spawn_with(TestConfig {
        base_path: "/ff".into(),
        trusted_peers_any: true,
        ha_idp: Some(FakeHaIdp::happy(ha_uuid(), "María")),
        ..Default::default()
    })
    .await;
    let r = app.get("/v1/auth/ha/start").await;
    assert_eq!(r.status, http::StatusCode::FOUND, "{r:?}");
    let raw = r.set_cookie(HA_STATE_COOKIE).expect("Set-Cookie");
    assert!(raw.contains("Path=/ff"), "{raw}");
    let location = url::Url::parse(&r.location().unwrap()).unwrap();
    let params: std::collections::HashMap<_, _> = location.query_pairs().collect();
    assert_eq!(params["client_id"], "http://futurefin.test/");
    assert_eq!(
        params["redirect_uri"],
        "http://futurefin.test/v1/auth/ha/callback"
    );
}

// ---------------------------------------------------------------------------------------
// 3. Camino feliz y paridad
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn the_happy_path_opens_a_real_session() {
    let fake = FakeHaIdp::happy(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;
    let (cookie, state, next) = start(&app, None).await;
    assert_eq!(next, "/");

    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert_eq!(r.status, http::StatusCode::FOUND, "{r:?}");
    assert_eq!(r.location().as_deref(), Some("/"));
    let session = r.session_cookie().expect("el callback abre sesión");
    // Y la cookie de estado queda retirada.
    assert_eq!(r.cookie_value(HA_STATE_COOKIE).as_deref(), Some(""));

    // La sesión sirve de verdad.
    let me = app.get_with_cookie("/v1/auth/me", &session).await;
    assert_eq!(me.status, http::StatusCode::OK, "{me:?}");
    assert_eq!(me.json()["username"], "maria");

    // Un solo usuario, y es el owner del hogar recién creado.
    assert_eq!(app.count_rows("users").await, 1);
    let ctx = app
        .get_with_cookie("/v1/installation/session-context", &session)
        .await;
    assert_eq!(ctx.json()["access"]["role"], "owner", "{ctx:?}");
}

/// La piedra angular: entrar por cabeceras y entrar por el flujo de HA con el MISMO id externo
/// (una forma con guiones, la otra sin ellos) es la misma persona.
#[tokio::test]
async fn header_sso_and_ha_login_resolve_to_the_same_user() {
    let app = TestApp::spawn_with(TestConfig {
        trusted_header_auth: true,
        trusted_peers_any: true,
        ha_idp: Some(FakeHaIdp::happy(ha_uuid(), "María")),
        ..Default::default()
    })
    .await;

    // Primero por cabeceras, con la forma canónica del UUID.
    let sso = app
        .post_with_headers(
            "/v1/auth/sso",
            &[("x-remote-user-id", HA_ID_HYPHENATED)],
            None,
            None,
        )
        .await;
    assert_eq!(sso.status, http::StatusCode::OK, "{sso:?}");
    let via_headers = sso.json()["id"].as_str().unwrap().to_string();

    // Y ahora por el flujo de HA, cuyo `result.id` viene en 32 hex sin guiones.
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    let session = r.session_cookie().expect("sesión");
    let me = app.get_with_cookie("/v1/auth/me", &session).await;
    assert_eq!(me.json()["id"].as_str().unwrap(), via_headers);

    // Una sola fila: la normalización de `Uuid::parse_str` es la misma en los dos caminos.
    assert_eq!(app.count_rows("users").await, 1);
}

// ---------------------------------------------------------------------------------------
// 4. Orden de las llamadas al proveedor
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn the_provider_is_called_in_order_and_the_refresh_token_is_revoked() {
    let fake = FakeHaIdp::happy(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(&app, Some(&cookie), &format!("code=el-codigo&state={state}")).await;
    assert!(r.session_cookie().is_some(), "{r:?}");

    assert_eq!(
        fake.calls(),
        vec![
            FakeCall::Exchange {
                code: "el-codigo".into(),
                // El `client_id` del canje es EXACTAMENTE el de la autorización.
                client_id: "http://futurefin.test/".into(),
            },
            FakeCall::Identity {
                access_token: "ha-access-token".into(),
            },
            FakeCall::Revoke {
                refresh_token: "ha-refresh-token".into(),
            },
        ]
    );
}

#[tokio::test]
async fn without_a_refresh_token_there_is_nothing_to_revoke() {
    let fake = FakeHaIdp::without_refresh(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert!(r.session_cookie().is_some(), "{r:?}");
    assert!(
        !fake
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::Revoke { .. })),
        "{:?}",
        fake.calls()
    );
}

#[tokio::test]
async fn a_revocation_that_changes_nothing_still_lets_the_user_in() {
    // `revoke` es infalible por firma: el trait lo impone justamente para que un fallo de
    // higiene no pueda tirar un login ya probado. El doble registra la llamada y no hace nada
    // más — que es exactamente lo que ocurre cuando HA no contesta.
    let fake = FakeHaIdp::happy(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert!(r.session_cookie().is_some(), "{r:?}");
    assert!(fake
        .calls()
        .iter()
        .any(|c| matches!(c, FakeCall::Revoke { .. })));
    assert_eq!(app.count_rows("users").await, 1);
}

// ---------------------------------------------------------------------------------------
// 5. El `state`: cuatro formas de no tenerlo
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_callback_without_a_valid_state_never_reaches_the_provider() {
    for (caso, cookie, query) in [
        ("state equivocado", true, "code=abc&state=noesmio"),
        ("sin state", true, "code=abc"),
        ("sin cookie", false, "code=abc&state=loquesea"),
        ("cookie basura", false, "code=abc&state=loquesea"),
    ] {
        let fake = FakeHaIdp::happy(ha_uuid(), "María");
        let app = app_with(fake.clone()).await;
        let real = start(&app, None).await.0;
        let sent = match (cookie, caso) {
            (true, _) => Some(real),
            (false, "cookie basura") => Some(format!("{HA_STATE_COOKIE}=basura-total")),
            _ => None,
        };
        let r = callback(&app, sent.as_deref(), query).await;
        assert_error_redirect(&r, "ha_state_mismatch");
        assert_eq!(app.count_rows("users").await, 0, "caso: {caso}");
        assert!(fake.calls().is_empty(), "caso: {caso} — el doble fue llamado");
    }
}

#[tokio::test]
async fn the_state_cookie_is_single_use() {
    let fake = FakeHaIdp::happy(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;
    let (cookie, state, _) = start(&app, None).await;
    let first = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert!(first.session_cookie().is_some(), "{first:?}");
    // El navegador ya no tiene la cookie (el callback la retiró); reenviarla a mano tampoco
    // sirve porque el `code` de HA es de un solo uso — aquí se comprueba lo que el servidor
    // controla: la cookie se ha retirado en la respuesta.
    assert_eq!(first.cookie_value(HA_STATE_COOKIE).as_deref(), Some(""));
    // Y un segundo callback sin cookie es un desconocido.
    let second = callback(&app, None, &format!("code=abc&state={state}")).await;
    assert_error_redirect(&second, "ha_state_mismatch");
    assert_eq!(app.count_rows("users").await, 1, "no se creó un segundo usuario");
}

// ---------------------------------------------------------------------------------------
// 6. Fallos del proveedor
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_failed_exchange_comes_back_as_ha_exchange_failed() {
    let app = app_with(FakeHaIdp::exchange_fails()).await;
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert_error_redirect(&r, "ha_exchange_failed");
    assert_eq!(app.count_rows("users").await, 0);
}

#[tokio::test]
async fn a_failed_identity_comes_back_as_ha_identity_failed() {
    let app = app_with(FakeHaIdp::identity_fails()).await;
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert_error_redirect(&r, "ha_identity_failed");
    assert_eq!(app.count_rows("users").await, 0);
}

#[tokio::test]
async fn when_the_person_refuses_home_assistant_sends_error_and_no_code() {
    let fake = FakeHaIdp::happy(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;
    let (cookie, state, _) = start(&app, None).await;
    let r = callback(
        &app,
        Some(&cookie),
        &format!("error=access_denied&state={state}"),
    )
    .await;
    assert_error_redirect(&r, "ha_exchange_failed");
    // No hay código que canjear: el proveedor no se toca.
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    assert_eq!(app.count_rows("users").await, 0);
}

// ---------------------------------------------------------------------------------------
// 7. El `next`
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn the_next_parameter_is_honoured_only_when_it_stays_inside_the_app() {
    for (raw, esperado) in [
        ("/movimientos", "/movimientos"),
        // Un retorno legítimo de esta misma app: la query lleva una URL entera.
        (
            "/oauth/authorize?client_id=https://x&state=y",
            "/oauth/authorize?client_id=https://x&state=y",
        ),
        ("https://evil.test", "/"),
        ("//evil.test", "/"),
        ("/\\evil", "/"),
    ] {
        let app = app_with(FakeHaIdp::happy(Uuid::new_v4(), "María")).await;
        let (cookie, state, _) = start(&app, Some(raw)).await;
        let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
        assert_eq!(r.location().as_deref(), Some(esperado), "next={raw:?}");
    }

    // Un `next` kilométrico también cae a la raíz.
    let app = app_with(FakeHaIdp::happy(Uuid::new_v4(), "María")).await;
    let largo = format!("/{}", "a".repeat(1024));
    let (cookie, state, _) = start(&app, Some(&largo)).await;
    let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    assert_eq!(r.location().as_deref(), Some("/"));
}

#[tokio::test]
async fn under_a_base_path_the_next_is_prefixed_exactly_once() {
    for enviado in ["/movimientos", "/ff/movimientos"] {
        let app = TestApp::spawn_with(TestConfig {
            base_path: "/ff".into(),
            trusted_peers_any: true,
            ha_idp: Some(FakeHaIdp::happy(Uuid::new_v4(), "María")),
            ..Default::default()
        })
        .await;
        // En la cookie viaja la forma canónica, SIN prefijo.
        let (cookie, state, next) = start(&app, Some(enviado)).await;
        assert_eq!(next, "/movimientos", "enviado={enviado}");
        let r = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
        assert_eq!(
            r.location().as_deref(),
            Some("/ff/movimientos"),
            "enviado={enviado}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// 8. Colisión de nombre
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn two_identities_with_the_same_display_name_get_distinct_usernames() {
    let fake = FakeHaIdp::happy(ha_uuid(), "María");
    let app = app_with(fake.clone()).await;

    let (cookie, state, _) = start(&app, None).await;
    let first = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    let s1 = first.session_cookie().expect("primera sesión");
    assert_eq!(
        app.get_with_cookie("/v1/auth/me", &s1).await.json()["username"],
        "maria"
    );

    // Otra persona de Home Assistant (otro id externo) con el MISMO nombre para mostrar: el
    // slug choca y la cascada de `resolve_or_provision` sufija.
    fake.set_identity(Uuid::new_v4(), "María");
    let (cookie, state, _) = start(&app, None).await;
    let second = callback(&app, Some(&cookie), &format!("code=abc&state={state}")).await;
    let s2 = second.session_cookie().expect("segunda sesión");
    assert_eq!(
        app.get_with_cookie("/v1/auth/me", &s2).await.json()["username"],
        "maria-2"
    );
    assert_eq!(app.count_rows("users").await, 2);
}

// ---------------------------------------------------------------------------------------
// 9. El shell lo anuncia
// ---------------------------------------------------------------------------------------

const SHELL: &str = "<!doctype html><html><head></head><body></body></html>";

#[tokio::test]
async fn the_shell_announces_whether_ha_login_exists() {
    let con = TestApp::spawn_with(TestConfig {
        with_spa_index: Some(SHELL.to_string()),
        ha_idp: Some(FakeHaIdp::happy(ha_uuid(), "María")),
        ..Default::default()
    })
    .await;
    let html = String::from_utf8(con.get("/cualquier-ruta").await.body).unwrap();
    assert!(html.contains("window.__FF_HA_LOGIN__=true"), "{html}");

    let sin = TestApp::spawn_with(TestConfig {
        with_spa_index: Some(SHELL.to_string()),
        ..Default::default()
    })
    .await;
    let r = sin.get("/cualquier-ruta").await;
    // Sin ninguna bandera y sin prefijo, el shell sale byte a byte como está en disco.
    assert_eq!(String::from_utf8(r.body).unwrap(), SHELL);
}
