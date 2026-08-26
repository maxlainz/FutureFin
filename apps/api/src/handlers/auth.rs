use crate::auth::password;
use crate::error::ApiError;
use crate::handlers::installation::{
    bootstrap_installation_as_owner_if_empty, require_installation_member,
};
use crate::handlers::projection::warm_up_household_projection;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use cookie::{SameSite, time::Duration as CookieDuration};
use futurefin_domain::UserId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "ff_session";

/// `Path` de la cookie de sesión para una request: el prefijo público bajo el que el navegador
/// ve la app, o `/` cuando no hay ninguno.
///
/// Por qué acotarla: bajo el Ingress de Home Assistant **todos los add-ons comparten origen**
/// (`http://homeassistant.local:8123`), así que un `Path=/` emitiría `ff_session` también hacia
/// `/api/hassio_ingress/<token-de-otro-add-on>`. Acotarla al prefijo propio la deja donde debe.
///
/// Invariante maestro: sin cabeceras de proxy el prefijo es `""` y la cookie sale con `Path=/`,
/// **byte a byte** como siempre — el modo compose no cambia.
pub(crate) fn session_cookie_path(state: &AppState, headers: &http::HeaderMap) -> String {
    let p = state.request_prefix(headers);
    if p.is_empty() {
        "/".to_string()
    } else {
        p
    }
}

/// Cookie de sesión (`ff_session`) con los atributos de siempre y el `Path` dado.
pub(crate) fn session_cookie(state: &AppState, sid: Uuid, path: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, sid.to_string()))
        .path(path)
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::days(state.session_ttl_days))
        .secure(state.cookie_secure)
        .build()
}

/// Cookie «plantilla» para el borrado. El navegador solo casa un `Set-Cookie` de borrado con la
/// cookie viva si **nombre y `Path` coinciden**: con el `Path=/` fijo de antes, un logout bajo
/// Ingress dejaba la cookie acotada viva y el usuario seguía «dentro».
pub(crate) fn session_cookie_removal(path: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, "")).path(path).build()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterBody {
    pub username: String,
    pub password: String,
    /// Fecha de nacimiento obligatoria (`YYYY-MM-DD`).
    #[schema(value_type = String, format = "date")]
    pub birth_date: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: UserId,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub birth_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchMeBody {
    /// `null` borra la fecha; `"YYYY-MM-DD"` la fija. Omitir el campo no actualiza.
    #[serde(default)]
    #[schema(nullable = true, value_type = Object)]
    pub birth_date: Option<Value>,
}

#[derive(Debug, FromRow)]
struct UserAuthRow {
    id: Uuid,
    #[allow(dead_code)]
    username: String,
    /// `None` = cuenta SSO sin contraseña (ver la migración `users_trusted_header_identity`).
    password_hash: Option<String>,
    #[allow(dead_code)]
    birth_date: Option<NaiveDate>,
}

#[derive(Debug, FromRow)]
pub(crate) struct UserRow {
    pub id: Uuid,
    pub username: String,
    pub birth_date: Option<NaiveDate>,
}

/// Error común a los dos sitios donde una cuenta sin contraseña se topa con el flujo de
/// contraseña: `POST /v1/auth/login` y `POST /v1/auth/password`. Es un 401 **hablado** a
/// propósito (ver `ApiError::UnauthorizedWith`): quien tiene una cuenta creada por el proxy no
/// tiene ninguna contraseña que probar, y un 401 mudo lo dejaría tecleando para siempre.
pub(crate) fn sso_account_no_password() -> ApiError {
    ApiError::UnauthorizedWith(
        "sso_account_no_password: this account signs in through the trusted proxy (Home Assistant)"
            .into(),
    )
}

pub(crate) fn validate_username(username: &str) -> Result<(), ApiError> {
    let trimmed = username.trim();
    if trimmed != username {
        return Err(ApiError::BadRequest(
            "username_whitespace: username must not have leading or trailing whitespace".into(),
        ));
    }
    let len = username.chars().count();
    if !(3..=64).contains(&len) {
        return Err(ApiError::BadRequest(
            "username_length: username must be between 3 and 64 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ApiError::BadRequest(
            "username_charset: username may only contain letters, digits, '.', '_' and '-'".into(),
        ));
    }
    Ok(())
}

fn parse_me_birth_patch(v: &Value) -> Result<Option<NaiveDate>, ApiError> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
            .map(Some)
            .map_err(|_| ApiError::BadRequest("birth_date_format: birth_date must be YYYY-MM-DD".into())),
        _ => Err(ApiError::BadRequest(
            "birth_date_type: birth_date must be null or a date string".into(),
        )),
    }
}

fn validate_birth_date(d: NaiveDate) -> Result<(), ApiError> {
    let today = Utc::now().date_naive();
    if d > today {
        return Err(ApiError::BadRequest(
            "birth_date_future: birth_date cannot be in the future".into(),
        ));
    }
    if d.year() < 1900 {
        return Err(ApiError::BadRequest(
            "birth_date_too_old: birth_date year must be >= 1900".into(),
        ));
    }
    Ok(())
}

pub(crate) fn user_row_to_response(row: UserRow) -> UserResponse {
    UserResponse {
        id: UserId(row.id),
        username: row.username,
        birth_date: row.birth_date,
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/register",
    security(()),
    tag = "auth",
    request_body = RegisterBody,
    responses(
        (status = 201, description = "Account created", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Username already taken"),
    )
)]
pub async fn register(
    Extension(state): Extension<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> Result<(axum::http::StatusCode, Json<UserResponse>), ApiError> {
    validate_username(&body.username)?;
    let birth_date = {
        let d = NaiveDate::parse_from_str(body.birth_date.trim(), "%Y-%m-%d")
            .map_err(|_| ApiError::BadRequest("birth_date_format: birth_date must be YYYY-MM-DD".into()))?;
        validate_birth_date(d)?;
        d
    };
    let hash = password::hash_password_blocking(&body.password).await?;
    let mut tx = state.pool.begin().await?;
    let row: UserRow = sqlx::query_as(
        r#"INSERT INTO users (username, password_hash, birth_date)
           VALUES ($1, $2, $3)
           RETURNING id, username, birth_date"#,
    )
    .bind(&body.username)
    .bind(&hash)
    .bind(birth_date)
    .fetch_one(&mut *tx)
    .await
    // El `?` normal mapearía el SQLSTATE 23505 a un `Conflict` pelado, que llega a la SPA como
    // «resource conflict»: cuando sabemos QUÉ colisionó hay que decirlo. Y desde el SSO por
    // cabeceras `username` **ya no es el único unique de la tabla** (`users_external_user_id_key`
    // también lo es), así que se comprueba el nombre de la restricción antes de traducir: sin
    // eso, una colisión de identidad externa se anunciaría como «ese nombre ya está registrado»
    // y mandaría al usuario a cambiar algo que no tiene nada que ver.
    .map_err(|e| {
        let is_username = matches!(&e, sqlx::Error::Database(db)
            if db.constraint() == Some("users_username_key"));
        match (ApiError::from(e), is_username) {
            (ApiError::Conflict, true) => ApiError::ConflictWith(
                "username_taken: that username is already registered".into(),
            ),
            (other, _) => other,
        }
    })?;

    match bootstrap_installation_as_owner_if_empty(&mut tx, &row.id).await
    {
        Ok(()) => {}
        // Dos primeros registros a la vez: otro creó la instalación entremedias. No es culpa de
        // quien lo intenta y se resuelve reintentando, así que hay que poder decirlo.
        Err(ApiError::Conflict) => {
            return Err(ApiError::ConflictWith(
                "installation_race: another registration created the installation first".into(),
            ));
        }
        Err(e) => return Err(e),
    }

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(user_row_to_response(row)),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    security(()),
    tag = "auth",
    request_body = LoginBody,
    responses(
        (status = 200, description = "Logged in; session cookie set", body = UserResponse),
        (status = 401, description = "Invalid credentials"),
    )
)]
pub async fn login(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: http::HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<(CookieJar, Json<UserResponse>), ApiError> {
    validate_username(&body.username)?;
    let user: Option<UserAuthRow> = sqlx::query_as(
        r#"SELECT id, username, password_hash, birth_date FROM users WHERE username = $1"#,
    )
    .bind(&body.username)
    .fetch_optional(&state.pool)
    .await?;
    // Se verifica SIEMPRE, exista el usuario o no —y también cuando existe SIN contraseña—: las
    // tres ramas pasan por el mismo coste de Argon2id, así que el 401 no delata quién tiene
    // cuenta por el reloj (ver `password.rs`).
    let stored = user.as_ref().and_then(|u| u.password_hash.clone());
    let verified = password::verify_password_blocking(&body.password, stored).await;
    let Some(user) = user else {
        verified?;
        return Err(ApiError::Unauthorized);
    };
    if user.password_hash.is_none() {
        // Cuenta creada por el proxy de confianza: no hay contraseña que acertar. Decirlo revela
        // que ese nombre existe como cuenta SSO, y es un intercambio buscado: sin el mensaje, el
        // único usuario del add-on de Home Assistant se queda encallado en un 401 mudo tecleando
        // una contraseña que nunca se fijó.
        return Err(sso_account_no_password());
    }
    verified?;
    let expires_at = Utc::now() + Duration::days(state.session_ttl_days);
    let sid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO sessions (user_id, expires_at) VALUES ($1, $2) RETURNING id"#,
    )
    .bind(user.id)
    .bind(expires_at)
    .fetch_one(&state.pool)
    .await?;
    let jar = jar.add(session_cookie(&state, sid, session_cookie_path(&state, &headers)));
    let row: UserRow = sqlx::query_as(
        r#"SELECT id, username, birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id)
    .fetch_one(&state.pool)
    .await?;

    // Warm-up del cache de proyección en background. Si el usuario no es
    // miembro de ningún installation (caso pending), skip silencioso. El
    // login responde inmediatamente sin esperar al recompute.
    if let Ok((iid, _)) = require_installation_member(&state.pool, user.id).await {
        let state_clone = state.clone();
        let user_id = user.id;
        tokio::spawn(async move {
            warm_up_household_projection(state_clone, iid, user_id).await;
        });
    }

    Ok((jar, Json(user_row_to_response(row))))
}

#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Session cleared"),
    )
)]
pub async fn logout(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: http::HeaderMap,
) -> Result<(CookieJar, axum::http::StatusCode), ApiError> {
    let mut user_id_to_invalidate: Option<Uuid> = None;
    if let Some(c) = jar.get(SESSION_COOKIE) {
        if let Ok(sid) = Uuid::parse_str(c.value()) {
            // Recupera el user_id antes de borrar la sesión para poder
            // limpiar sus entries `view=mine` del cache de proyección.
            user_id_to_invalidate = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT user_id FROM sessions WHERE id = $1"#,
            )
            .bind(sid)
            .fetch_optional(&state.pool)
            .await?;
            sqlx::query(r#"DELETE FROM sessions WHERE id = $1"#)
                .bind(sid)
                .execute(&state.pool)
                .await?;
        }
    }
    if let Some(uid) = user_id_to_invalidate {
        state.invalidate_projection_by_user(uid).await;
    }
    let jar = jar.remove(session_cookie_removal(session_cookie_path(&state, &headers)));
    Ok((jar, axum::http::StatusCode::NO_CONTENT))
}

#[utoipa::path(
    get,
    path = "/v1/auth/me",
    tag = "auth",
    responses(
        (status = 200, description = "Current user", body = UserResponse),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn me(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<UserResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let row: UserRow = sqlx::query_as(
        r#"SELECT id, username, birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(user_row_to_response(row)))
}

#[utoipa::path(
    patch,
    path = "/v1/auth/me",
    tag = "auth",
    request_body = PatchMeBody,
    responses(
        (status = 200, description = "Perfil actualizado", body = UserResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn patch_me(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<PatchMeBody>,
) -> Result<Json<UserResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;

    let mut birth_changed = false;
    if let Some(ref raw) = body.birth_date {
        let parsed = parse_me_birth_patch(raw)?;
        if let Some(d) = parsed {
            validate_birth_date(d)?;
        }
        sqlx::query(
            r#"UPDATE users SET birth_date = $1 WHERE id = $2"#,
        )
        .bind(parsed)
        .bind(user.id.0)
        .execute(&state.pool)
        .await?;
        birth_changed = true;
    }

    let row: UserRow = sqlx::query_as(
        r#"SELECT id, username, birth_date FROM users WHERE id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&state.pool)
    .await?;

    // birth_date afecta el eje de edad y horizonte de la proyección → invalida.
    if birth_changed {
        if let Ok((iid, _)) = require_installation_member(&state.pool, user.id.0).await {
            crate::handlers::projection::refresh_projection_after_mutation(
                &state,
                iid,
                user.id.0,
            )
            .await;
        }
    }

    Ok(Json(user_row_to_response(row)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordBody {
    pub current_password: String,
    pub new_password: String,
}

/// Cambio de contraseña — y, con él, el corte de todo lo que la contraseña vieja sostenía.
///
/// Hasta 4.0.0 `hash_password` solo se llamaba en `register`: no había forma de rotar la
/// contraseña. Una cookie esnifada en la wifi de casa (`COOKIE_SECURE=false` por defecto, tras
/// proxy), una sesión abierta en un equipo compartido o una filtración en otro servicio daban
/// `SESSION_TTL_DAYS` de acceso completo sin que la víctima pudiera hacer nada — y `SECURITY.md`
/// describía el comportamiento de este endpoint como si existiera.
///
/// Cambiar la contraseña **revoca las otras tres credenciales** en la misma transacción: las
/// demás sesiones, los tokens de API (`ffp_…`) y las concesiones OAuth. Es el default seguro:
/// si la razón del cambio es un compromiso, dejar viva una credencial que no caduca haría el
/// cambio decorativo. La sesión desde la que se llama sobrevive, para no echar al usuario de la
/// app al terminar.
///
/// AVISO documentado en `SECURITY.md`: los `.ffbackup` ya exportados siguen atados a la
/// contraseña con la que se generaron. No se recifran.
#[utoipa::path(
    post,
    path = "/v1/auth/password",
    tag = "auth",
    request_body = ChangePasswordBody,
    responses(
        (status = 204, description = "Contraseña cambiada; el resto de credenciales revocadas"),
        (status = 400, description = "La contraseña actual no es correcta, o la nueva no cumple la política"),
        (status = 401, description = "Sin sesión válida"),
    )
)]
pub async fn change_password(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<ChangePasswordBody>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let current_sid = jar
        .get(SESSION_COOKIE)
        .and_then(|c| Uuid::parse_str(c.value()).ok());

    let stored: Option<String> =
        sqlx::query_scalar(r#"SELECT password_hash FROM users WHERE id = $1"#)
            .bind(user.id.0)
            .fetch_one(&state.pool)
            .await?;
    // Cuenta SSO: no hay contraseña actual que verificar, y FIJAR una desde aquí crearía una
    // segunda vía de acceso a una cuenta cuya autenticación pertenece al proveedor. Fuera de
    // alcance en esta release; el 401 lo dice en vez de fallar con «contraseña incorrecta».
    let Some(stored) = stored else {
        return Err(sso_account_no_password());
    };
    // 400 y no 401: la sesión es válida: lo que falla es el dato del formulario. Un 401 haría
    // que la SPA echara al usuario al login por escribir mal su propia contraseña.
    // Solo el 401 significa «la contraseña no es la suya»; un `Unavailable` (pánico del task,
    // runtime cerrándose) se propaga tal cual. Traducirlo también a «contraseña incorrecta»
    // mandaría al usuario a dudar de su memoria por un fallo de infraestructura.
    match password::verify_password_blocking(&body.current_password, Some(stored)).await {
        Ok(()) => {}
        Err(ApiError::Unauthorized) => {
            return Err(ApiError::BadRequest(
                "current_password_invalid: la contraseña actual no es correcta".into(),
            ))
        }
        Err(other) => return Err(other),
    }

    // Repetir la contraseña actual no es una rotación: sería revocar las otras tres
    // credenciales sin cambiar nada, con la apariencia de haber rotado.
    if body.new_password == body.current_password {
        return Err(ApiError::BadRequest(
            "password_unchanged: la contraseña nueva es la misma que la actual".into(),
        ));
    }
    let hash = password::hash_password_blocking(&body.new_password).await?;

    let mut tx = state.pool.begin().await?;
    sqlx::query(r#"UPDATE users SET password_hash = $2 WHERE id = $1"#)
        .bind(user.id.0)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"DELETE FROM sessions WHERE user_id = $1 AND ($2::uuid IS NULL OR id <> $2)"#,
    )
    .bind(user.id.0)
    .bind(current_sid)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE api_tokens SET revoked_at = now()
           WHERE user_id = $1 AND revoked_at IS NULL"#,
    )
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE oauth_grants SET revoked_at = now(), revoked_reason = 'password_change'
           WHERE user_id = $1 AND revoked_at IS NULL"#,
    )
    .bind(user.id.0)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(user_id = %user.id.0, "password changed; other sessions, api tokens and oauth grants revoked");
    Ok(axum::http::StatusCode::NO_CONTENT)
}
