//! Cambio de contraseña y gestión de membresías — las dos palancas que 4.0.0 añade.
//!
//! Ninguna existía antes, y las dos estaban **prometidas por la documentación**: `SECURITY.md`
//! explicaba qué pasa con los `.ffbackup` «si cambias la contraseña después», y tanto
//! `SECURITY.md` como `.claude/auth-and-membership.md` afirmaban que revocar una membresía
//! corta el acceso al instante. El mecanismo de corte sí estaba (rol y pertenencia se
//! re-resuelven en cada petición); lo que faltaba era la palanca. Consecuencia real: aprobar
//! al usuario equivocado concedía acceso permanente a todas las finanzas del hogar, y una
//! contraseña filtrada no se podía rotar — `SESSION_TTL_DAYS` de acceso, más tokens `ffp_` que
//! pueden no caducar nunca.
//!
//! Requiere Postgres en `TEST_DATABASE_URL` (ver `common/mod.rs`).

mod common;

use common::TestApp;

const PW: &str = "correct horse battery staple";
const NEW_PW: &str = "un caballo correcto y una grapa";

// ---------------------------------------------------------------------------
// Cambio de contraseña
// ---------------------------------------------------------------------------

/// El cambio funciona, la sesión que lo pide sobrevive, y a partir de ahí solo vale la nueva.
#[tokio::test]
async fn changing_the_password_keeps_the_calling_session_and_switches_the_credential() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({"current_password": PW, "new_password": NEW_PW}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NO_CONTENT, "cambio de contraseña: {r:?}");

    // La sesión desde la que se pidió sigue viva: echar al usuario de la app al cambiar su
    // propia contraseña sería absurdo.
    let me = app.get_with_cookie("/v1/auth/me", &owner.cookie).await;
    assert_eq!(me.status, http::StatusCode::OK, "la sesión que hizo el cambio debe sobrevivir");

    // La vieja ya no abre; la nueva sí.
    let vieja = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "alice", "password": PW}),
        )
        .await;
    assert_eq!(vieja.status, http::StatusCode::UNAUTHORIZED, "la contraseña vieja no debe valer");
    let nueva = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "alice", "password": NEW_PW}),
        )
        .await;
    assert_eq!(nueva.status, http::StatusCode::OK, "la contraseña nueva debe valer: {nueva:?}");
}

/// La contraseña actual incorrecta da **400**, no 401: la sesión es válida, lo que falla es el
/// dato del formulario. Un 401 haría que la SPA mandara al login a quien se equivoca al teclear.
#[tokio::test]
async fn changing_the_password_with_a_wrong_current_one_is_a_bad_request_not_a_logout() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({"current_password": "esta no es", "new_password": NEW_PW}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "current_password_invalid", "{}", r.json());

    // Y no ha cambiado nada.
    let login = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "alice", "password": PW}),
        )
        .await;
    assert_eq!(login.status, http::StatusCode::OK, "la contraseña original debe seguir valiendo");
}

/// Repetir la contraseña actual no es rotar: revocaría las otras tres credenciales sin cambiar
/// nada, con la apariencia de haber rotado.
#[tokio::test]
async fn changing_the_password_to_the_same_one_is_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({"current_password": PW, "new_password": PW}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "password_unchanged", "{}", r.json());
}

/// La política de longitud se aplica también aquí, no solo en el registro.
#[tokio::test]
async fn changing_the_password_enforces_the_length_policy() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app
        .post_json_with_cookie(
            "/v1/auth/password",
            serde_json::json!({"current_password": PW, "new_password": "corta"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "password_length", "{}", r.json());
}

/// El cambio corta **todo lo que sostenía la contraseña vieja**: las demás sesiones y los
/// tokens de API. Sin esto el cambio sería decorativo justo en el caso que lo motiva —
/// recuperar una cuenta comprometida.
#[tokio::test]
async fn changing_the_password_revokes_other_sessions_and_api_tokens() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Una segunda sesión (otro dispositivo) y un token de API.
    let otra = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "alice", "password": PW}),
        )
        .await;
    assert_eq!(otra.status, http::StatusCode::OK);
    let otra_cookie = otra.session_cookie().expect("cookie de la segunda sesión");
    assert_eq!(
        app.get_with_cookie("/v1/auth/me", &otra_cookie).await.status,
        http::StatusCode::OK,
        "la segunda sesión debe funcionar antes del cambio"
    );

    let tok = app
        .post_json_with_cookie(
            "/v1/api-tokens",
            serde_json::json!({"label": "claude"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(tok.status, http::StatusCode::CREATED, "{tok:?}");

    app.post_json_with_cookie(
        "/v1/auth/password",
        serde_json::json!({"current_password": PW, "new_password": NEW_PW}),
        &owner.cookie,
    )
    .await;

    assert_eq!(
        app.get_with_cookie("/v1/auth/me", &otra_cookie).await.status,
        http::StatusCode::UNAUTHORIZED,
        "la otra sesión DEBE morir con el cambio de contraseña"
    );
    let tokens = app.get_with_cookie("/v1/api-tokens", &owner.cookie).await;
    assert_eq!(tokens.status, http::StatusCode::OK);
    let vivos = tokens
        .json()
        .as_array()
        .expect("lista de tokens")
        .iter()
        .filter(|t| t["revoked_at"].is_null())
        .count();
    assert_eq!(vivos, 0, "ningún token de API debe seguir vivo: {}", tokens.json());
}

// ---------------------------------------------------------------------------
// Membresías
// ---------------------------------------------------------------------------

/// Listar miembros lo puede hacer cualquiera del hogar: todos comparten los mismos datos
/// financieros, así que saber quién más entra no revela nada — y es lo que permite auditarlo.
#[tokio::test]
async fn any_member_can_list_the_household_members() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "viewer").await;

    for (quien, cookie) in [("owner", &owner.cookie), ("viewer", &bob.cookie)] {
        let r = app.get_with_cookie("/v1/installation/members", cookie).await;
        assert_eq!(r.status, http::StatusCode::OK, "{quien}: {r:?}");
        let arr = r.json();
        let arr = arr.as_array().expect("lista de miembros");
        assert_eq!(arr.len(), 2, "{quien}: {}", r.json());
        // Ordenados por rol: primero el owner.
        assert_eq!(arr[0]["role"], "owner", "{}", r.json());
        assert_eq!(arr[0]["username"], "alice");
        assert_eq!(arr[1]["role"], "viewer");
        assert_eq!(arr[1]["username"], "bob");
    }
}

/// Cambiar el rol es de owners, y funciona en los dos sentidos.
#[tokio::test]
async fn only_an_owner_can_change_roles() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "viewer").await;

    // Un viewer no puede promocionarse a sí mismo.
    let intento = app
        .patch_json_with_cookie(
            &format!("/v1/installation/members/{}", bob.user_id),
            serde_json::json!({"role": "owner"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(intento.status, http::StatusCode::FORBIDDEN, "{intento:?}");

    // El owner sí. Y el efecto es inmediato: bob pasa a poder escribir.
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let antes = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "X", "current_value": "1"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(antes.status, http::StatusCode::FORBIDDEN, "un viewer no escribe: {antes:?}");

    let sube = app
        .patch_json_with_cookie(
            &format!("/v1/installation/members/{}", bob.user_id),
            serde_json::json!({"role": "member"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(sube.status, http::StatusCode::NO_CONTENT, "{sube:?}");

    let despues = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "X", "current_value": "1"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(
        despues.status,
        http::StatusCode::CREATED,
        "el rol se re-resuelve en cada petición, sin re-login: {despues:?}"
    );
}

/// Revocar corta las cuatro credenciales a la vez — membresía, sesión, tokens y OAuth — y lo
/// hace **al instante**, sin esperar a que caduque nada.
#[tokio::test]
async fn revoking_a_membership_cuts_access_immediately_and_keeps_the_data() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;

    // Bob tiene sesión, datos y un token de API.
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let suyo = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "El de Bob", "current_value": "1234"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(suyo.status, http::StatusCode::CREATED, "{suyo:?}");
    let tok = app
        .post_json_with_cookie("/v1/api-tokens", serde_json::json!({"label": "de bob"}), &bob.cookie)
        .await;
    assert_eq!(tok.status, http::StatusCode::CREATED, "{tok:?}");
    let bearer = tok.json()["token"].as_str().expect("token en claro").to_string();

    let out = app
        .delete_with_cookie(
            &format!("/v1/installation/members/{}", bob.user_id),
            &owner.cookie,
        )
        .await;
    assert_eq!(out.status, http::StatusCode::NO_CONTENT, "{out:?}");

    // Sesión muerta.
    assert_eq!(
        app.get_with_cookie("/v1/auth/me", &bob.cookie).await.status,
        http::StatusCode::UNAUTHORIZED,
        "la sesión del expulsado debe morir"
    );
    // Token muerto (el MCP lo rechaza).
    let mcp = app.mcp_initialize(Some(&bearer)).await;
    assert_eq!(
        mcp.status,
        http::StatusCode::UNAUTHORIZED,
        "el token de API del expulsado debe dejar de valer: {mcp:?}"
    );
    // Puede volver a entrar, pero ya no ve nada: vuelve a ser un pendiente.
    let relogin = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "bob", "password": "correct horse battery staple"}),
        )
        .await;
    assert_eq!(relogin.status, http::StatusCode::OK, "puede autenticarse: {relogin:?}");
    let cookie2 = relogin.session_cookie().expect("cookie");
    assert_eq!(
        app.get_with_cookie("/v1/assets", &cookie2).await.status,
        http::StatusCode::FORBIDDEN,
        "sin membresía no ve los datos del hogar"
    );

    // Y sus datos siguen ahí: revocar el acceso NO borra el patrimonio del hogar. Se mira con
    // `?view=household` EXPLÍCITO (5.0.0, R2): el default es `mine`, donde las filas del
    // expulsado nunca salieron — y ausencia por scope no prueba que la fila siga viva.
    let assets = app
        .get_with_cookie("/v1/assets?view=household", &owner.cookie)
        .await;
    assert_eq!(assets.status, http::StatusCode::OK);
    let assets_json = assets.json();
    assert!(
        assets_json
            .as_array()
            .expect("assets")
            .iter()
            .any(|a| a["name"] == "El de Bob"),
        "el activo creado por el expulsado debe conservarse: {assets_json}"
    );
}

/// La instalación no se puede quedar sin propietario — ni expulsando ni degradando.
#[tokio::test]
async fn the_last_owner_can_be_neither_removed_nor_demoted() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let baja = app
        .delete_with_cookie(
            &format!("/v1/installation/members/{}", owner.user_id),
            &owner.cookie,
        )
        .await;
    assert_eq!(baja.status, http::StatusCode::BAD_REQUEST, "{baja:?}");
    assert_eq!(baja.json()["code"], "last_owner", "{}", baja.json());

    let degrada = app
        .patch_json_with_cookie(
            &format!("/v1/installation/members/{}", owner.user_id),
            serde_json::json!({"role": "member"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(degrada.status, http::StatusCode::BAD_REQUEST, "{degrada:?}");
    assert_eq!(degrada.json()["code"], "last_owner", "{}", degrada.json());

    // Con un segundo owner, el primero ya puede irse: el traspaso de propiedad es posible.
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let sube = app
        .patch_json_with_cookie(
            &format!("/v1/installation/members/{}", bob.user_id),
            serde_json::json!({"role": "owner"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(sube.status, http::StatusCode::NO_CONTENT, "{sube:?}");
    let ahora = app
        .patch_json_with_cookie(
            &format!("/v1/installation/members/{}", owner.user_id),
            serde_json::json!({"role": "member"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(ahora.status, http::StatusCode::NO_CONTENT, "con dos owners sí se puede: {ahora:?}");
}

/// Un `member` con permiso de escritura sobre el ledger **no** puede tocar membresías. Es el
/// rol que tendría un atacante realista dentro del hogar: el test anterior solo prueba con un
/// `viewer`, que no escribe nada de todas formas.
#[tokio::test]
async fn a_plain_member_cannot_touch_memberships() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;
    let carol = app.register_and_approve_member(&owner, "carol", "viewer").await;

    // Escribe en el ledger sin problema…
    let cat = app.create_category(&owner, "asset", "Cash").await;
    let ok = app
        .post_json_with_cookie(
            "/v1/assets",
            serde_json::json!({"category_id": cat, "name": "X", "current_value": "1"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::CREATED, "un member sí escribe: {ok:?}");

    // …y ninguna de las dos mutaciones de membresía le vale.
    let sube = app
        .patch_json_with_cookie(
            &format!("/v1/installation/members/{}", carol.user_id),
            serde_json::json!({"role": "owner"}),
            &bob.cookie,
        )
        .await;
    assert_eq!(sube.status, http::StatusCode::FORBIDDEN, "{sube:?}");
    let echa = app
        .delete_with_cookie(
            &format!("/v1/installation/members/{}", carol.user_id),
            &bob.cookie,
        )
        .await;
    assert_eq!(echa.status, http::StatusCode::FORBIDDEN, "{echa:?}");

    // Y carol sigue dentro.
    let list = app.get_with_cookie("/v1/installation/members", &owner.cookie).await;
    assert_eq!(list.json().as_array().expect("miembros").len(), 3, "{}", list.json());
}

/// Un usuario que no es miembro devuelve 404, no un 204 silencioso.
#[tokio::test]
async fn removing_a_non_member_is_a_not_found() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .delete_with_cookie(
            &format!("/v1/installation/members/{}", uuid::Uuid::new_v4()),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::NOT_FOUND, "{r:?}");
}
