//! `POST /v1/auth/sso` — identidad delegada a un proxy de confianza (fase B del add-on de
//! Home Assistant).
//!
//! Lo que estos tests fijan, en orden de importancia:
//!  1. **La puerta está cerrada por defecto.** El endpoint se monta siempre, pero sin
//!     `trusted_header_auth` y sin un peer de confianza responde 401 — una cabecera de identidad
//!     es una afirmación sin prueba y no puede valer por sí sola.
//!  2. La identidad externa es **estable**: el mismo `X-Remote-User-Id` siempre devuelve el
//!     mismo usuario, sin duplicar filas.
//!  3. El alta por SSO entra por las **mismas puertas** que el registro por contraseña: el
//!     primero crea el hogar y es owner; los siguientes quedan pendientes de aprobación.
//!  4. Una cuenta sin contraseña **no puede colarse por el login normal** — y el 401 lo explica
//!     en vez de dejar a la persona probando contraseñas que no existen.

mod common;

use common::{TestApp, TestConfig};

const HA_USER: &str = "11111111-2222-3333-4444-555555555555";

fn sso_headers(id: &str, display_name: Option<&str>) -> Vec<(&'static str, String)> {
    let mut h = vec![("x-remote-user-id", id.to_string())];
    if let Some(n) = display_name {
        h.push(("x-remote-user-display-name", n.to_string()));
    }
    h
}

/// `post_with_headers` toma `&[(&str, &str)]`; esto adapta el vector de arriba.
async fn post_sso(app: &TestApp, headers: &[(&'static str, String)]) -> common::ResponseParts {
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    app.post_with_headers("/v1/auth/sso", &borrowed, None, None)
        .await
}

async fn trusted_app() -> TestApp {
    TestApp::spawn_with(TestConfig {
        trusted_header_auth: true,
        trusted_peers_any: true,
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn sso_is_off_by_default_even_with_perfect_headers() {
    let app = TestApp::spawn().await;
    let r = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    assert_eq!(r.status, http::StatusCode::UNAUTHORIZED, "{r:?}");
    assert_eq!(r.json()["code"], "sso_disabled");
    // Y no ha creado nada.
    assert_eq!(app.count_rows("users").await, 0);
}

#[tokio::test]
async fn sso_rejects_a_peer_outside_the_trust_policy() {
    // Auth activada pero política `Disabled`: en `oneshot` no hay `ConnectInfo`, así que el peer
    // es `None` y solo `Any` lo aceptaría. Es exactamente el caso «alguien llega por otro camino».
    let app = TestApp::spawn_with(TestConfig {
        trusted_header_auth: true,
        trusted_peers_any: false,
        ..Default::default()
    })
    .await;
    let r = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    assert_eq!(r.status, http::StatusCode::UNAUTHORIZED, "{r:?}");
    assert_eq!(r.json()["code"], "sso_untrusted_peer");
    assert_eq!(app.count_rows("users").await, 0);
}

#[tokio::test]
async fn sso_without_a_uuid_identity_is_a_bad_request() {
    let app = trusted_app().await;
    // Sin cabecera.
    let r = post_sso(&app, &[]).await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "sso_bad_identity");
    // Con cabecera que no es un UUID.
    let r = post_sso(&app, &sso_headers("no-soy-un-uuid", None)).await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "sso_bad_identity");
}

#[tokio::test]
async fn a_repeated_identity_header_is_refused() {
    // Un proxy que AÑADE su cabecera sin stripear la del cliente deja dos valores y el primero
    // —el del cliente— sería el que gana con un `get` normal. Identidad ambigua ⇒ 400, y nada
    // provisionado.
    let app = trusted_app().await;
    let r = post_sso(
        &app,
        &[
            ("x-remote-user-id", "99999999-9999-9999-9999-999999999999".to_string()),
            ("x-remote-user-id", HA_USER.to_string()),
        ],
    )
    .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "sso_bad_identity");
    assert_eq!(app.count_rows("users").await, 0);
}

#[tokio::test]
async fn first_sso_user_bootstraps_the_installation_as_owner() {
    let app = trusted_app().await;
    let r = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let cookie = r.session_cookie().expect("sso sets ff_session");
    assert_eq!(r.json()["username"], "maria");
    // Cuenta sin contraseña, con la identidad externa guardada.
    let hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE username = 'maria'")
            .fetch_one(&app.pool)
            .await
            .expect("user row");
    assert!(hash.is_none(), "una cuenta SSO no guarda contraseña");

    // Mismo camino que el primer registro por contraseña: hogar creado y rol owner.
    let ctx = app
        .get_with_cookie("/v1/installation/session-context", &cookie)
        .await;
    assert_eq!(ctx.status, http::StatusCode::OK, "{ctx:?}");
    let body = ctx.json();
    assert_eq!(body["installation_initialized"], true);
    assert_eq!(body["access"]["role"], "owner");
}

#[tokio::test]
async fn a_second_sso_user_is_pending_until_the_owner_approves() {
    let app = trusted_app().await;
    let owner = app.register_and_login_owner("propietaria").await;

    let r = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let cookie = r.session_cookie().expect("sso sets ff_session");
    let sso_user_id = r.json()["id"].as_str().expect("id").to_string();

    let ctx = app
        .get_with_cookie("/v1/installation/session-context", &cookie)
        .await;
    assert_eq!(ctx.json()["installation_initialized"], true);
    assert!(
        ctx.json()["access"].is_null(),
        "sin aprobar, la cuenta SSO no ve nada: {ctx:?}"
    );

    let approve = app
        .post_json_with_cookie(
            &format!("/v1/installation/pending-users/{sso_user_id}/approve"),
            serde_json::json!({"role": "member"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(approve.status, http::StatusCode::NO_CONTENT, "{approve:?}");

    let ctx = app
        .get_with_cookie("/v1/installation/session-context", &cookie)
        .await;
    assert_eq!(ctx.json()["access"]["role"], "member", "{ctx:?}");
}

#[tokio::test]
async fn the_same_external_identity_always_lands_on_the_same_user() {
    let app = trusted_app().await;
    let first = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    assert_eq!(first.status, http::StatusCode::OK, "{first:?}");
    let c1 = first.session_cookie().expect("cookie");

    // Segunda entrada: el nombre para mostrar puede haber cambiado, la identidad no.
    let second = post_sso(&app, &sso_headers(HA_USER, Some("María la de Antes"))).await;
    assert_eq!(second.status, http::StatusCode::OK, "{second:?}");
    let c2 = second.session_cookie().expect("cookie");

    let me1 = app.get_with_cookie("/v1/auth/me", &c1).await;
    let me2 = app.get_with_cookie("/v1/auth/me", &c2).await;
    assert_eq!(me1.json()["id"], me2.json()["id"]);
    assert_eq!(me1.json()["username"], me2.json()["username"]);
    assert_eq!(
        app.count_rows("users").await,
        1,
        "la segunda entrada no puede crear un usuario nuevo"
    );
    // Dos sesiones distintas sí: cada entrada es un login.
    assert_eq!(app.count_rows("sessions").await, 2);
}

#[tokio::test]
async fn display_names_with_diacritics_become_valid_usernames() {
    let app = trusted_app().await;
    let r = post_sso(&app, &sso_headers(HA_USER, Some("José Ñandú García"))).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let username = r.json()["username"].as_str().expect("username").to_string();
    assert_eq!(username, "jose-nandu-garcia");
    assert!(
        username
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')),
        "username fuera del charset: {username}"
    );
}

#[tokio::test]
async fn a_taken_username_gets_a_suffix_and_both_accounts_survive() {
    let app = trusted_app().await;
    // Alguien ya registró a mano el nombre que saldría del slug.
    let owner = app.register_and_login_owner("jose-nandu-garcia").await;

    let r = post_sso(&app, &sso_headers(HA_USER, Some("José Ñandú García"))).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["username"], "jose-nandu-garcia-2");
    assert_ne!(r.json()["id"].as_str().unwrap(), owner.user_id.to_string());
    assert_eq!(app.count_rows("users").await, 2);
}

#[tokio::test]
async fn an_sso_account_cannot_log_in_with_a_password() {
    let app = trusted_app().await;
    let r = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    let login = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "maria", "password": "lo que sea, da igual"}),
        )
        .await;
    assert_eq!(login.status, http::StatusCode::UNAUTHORIZED, "{login:?}");
    assert_eq!(login.json()["code"], "sso_account_no_password");
    assert!(
        login.session_cookie().is_none(),
        "un login fallido no puede dejar sesión"
    );
}

#[tokio::test]
async fn an_sso_account_cannot_set_a_password_from_the_app() {
    let app = trusted_app().await;
    let r = post_sso(&app, &sso_headers(HA_USER, Some("María"))).await;
    let cookie = r.session_cookie().expect("cookie");

    let change = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({
                "current_password": "no existe ninguna",
                "new_password": "una contraseña larguísima",
            }),
            &cookie,
        )
        .await;
    assert_eq!(change.status, http::StatusCode::UNAUTHORIZED, "{change:?}");
    assert_eq!(change.json()["code"], "sso_account_no_password");
}

#[tokio::test]
async fn password_accounts_are_untouched_by_the_sso_column() {
    // El contrato de siempre: registrar, entrar y rotar la contraseña sigue funcionando con la
    // columna `password_hash` ya nullable y con `external_user_id` presente.
    let app = trusted_app().await;
    let owner = app.register_and_login_owner("propietaria").await;
    let change = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({
                "current_password": "correct horse battery staple",
                "new_password": "otra contraseña bien larga",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(change.status, http::StatusCode::NO_CONTENT, "{change:?}");
    let relogin = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "propietaria", "password": "otra contraseña bien larga"}),
        )
        .await;
    assert_eq!(relogin.status, http::StatusCode::OK, "{relogin:?}");
    let external: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT external_user_id FROM users WHERE username = 'propietaria'")
            .fetch_one(&app.pool)
            .await
            .expect("user row");
    assert!(external.is_none());
}
