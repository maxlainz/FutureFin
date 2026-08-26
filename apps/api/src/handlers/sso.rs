//! Identidad delegada a un proxy de confianza (`POST /v1/auth/sso`).
//!
//! El add-on de Home Assistant corre detrás del Ingress del Supervisor, que ya autenticó a la
//! persona antes de que la petición llegue aquí y añade `X-Remote-User-Id` (stripeando el que
//! venga del cliente). Este endpoint convierte esa identidad en una **sesión normal** de
//! FutureFin: la misma fila en `sessions`, la misma cookie `ff_session`, el mismo gate de
//! instalación. A partir del 200 no hay nada especial en el usuario salvo que su
//! `password_hash` es NULL.
//!
//! La ruta se monta SIEMPRE — la forma del router no puede depender del entorno, o los tests
//! dejan de describir el binario que se despliega. Lo que decide es el estado: sin
//! `FUTUREFIN_TRUSTED_PROXY_AUTH` responde `sso_disabled`, y desde un peer que no está en
//! `FUTUREFIN_TRUSTED_PROXY_IPS`, `sso_untrusted_peer`. **Las dos comprobaciones son la
//! frontera de seguridad entera**: una cabecera de identidad es una afirmación sin prueba, así
//! que solo vale la palabra de un peer que el operador ha nombrado.

use crate::error::ApiError;
use crate::handlers::auth::{session_cookie, session_cookie_path, user_row_to_response, UserRow, UserResponse};
use crate::handlers::installation::{
    bootstrap_installation_as_owner_if_empty, require_installation_member,
};
use crate::handlers::projection::warm_up_household_projection;
use crate::handlers::spa::X_REMOTE_USER_ID;
use crate::prefix::PeerIp;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{Duration, Utc};
use std::sync::Arc;
use uuid::Uuid;

/// Nombre de cuenta del proveedor (p. ej. `maria`). Opcional.
pub const X_REMOTE_USER_NAME: &str = "x-remote-user-name";
/// Nombre para mostrar del proveedor (p. ej. `María Ñandú`). Opcional; tiene precedencia sobre
/// el anterior porque es lo que la persona reconoce como suyo.
pub const X_REMOTE_USER_DISPLAY_NAME: &str = "x-remote-user-display-name";

/// Tope del slug antes de sufijar. 60 + `-2` cabe de sobra en el máximo de 64 del `username`.
const USERNAME_SLUG_MAX: usize = 60;

#[utoipa::path(
    post,
    path = "/v1/auth/sso",
    security(()),
    tag = "auth",
    responses(
        (status = 200, description = "Sesión creada a partir de la identidad del proxy; cookie ff_session puesta", body = UserResponse),
        (status = 400, description = "La cabecera X-Remote-User-Id falta o no es un UUID (`sso_bad_identity`)"),
        (status = 401, description = "SSO desactivado (`sso_disabled`) o peer no de confianza (`sso_untrusted_peer`)"),
    ),
    description = "SOLO funciona detrás de un proxy de confianza. Requiere \
                   `FUTUREFIN_TRUSTED_PROXY_AUTH=1`, que la IP del peer esté en \
                   `FUTUREFIN_TRUSTED_PROXY_IPS`, y la cabecera `X-Remote-User-Id` (UUID). \
                   El primer usuario que entra por aquí crea el hogar y queda como owner, igual \
                   que el primer registro por contraseña; los siguientes quedan pendientes de \
                   aprobación."
)]
pub async fn sso_login(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    PeerIp(peer): PeerIp,
    headers: http::HeaderMap,
) -> Result<(CookieJar, Json<UserResponse>), ApiError> {
    if !state.trusted_header_auth {
        return Err(ApiError::UnauthorizedWith(
            "sso_disabled: header-based identity is not enabled on this server".into(),
        ));
    }
    if !state.trusted_peers.allows(peer) {
        return Err(ApiError::UnauthorizedWith(
            "sso_untrusted_peer: this peer is not a trusted proxy".into(),
        ));
    }

    let external_user_id = headers
        .get(X_REMOTE_USER_ID)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "sso_bad_identity: X-Remote-User-Id must be present and a UUID".into(),
            )
        })?;

    // El nombre para mostrar manda sobre el de cuenta: es el que la persona reconoce. Los dos
    // son opcionales — el Supervisor no siempre los manda — y de ahí el fallback del slug.
    let raw_name = header_text(&headers, X_REMOTE_USER_DISPLAY_NAME)
        .or_else(|| header_text(&headers, X_REMOTE_USER_NAME))
        .unwrap_or_default();

    let user_id = resolve_or_provision(&state, external_user_id, &raw_name).await?;

    // A partir de aquí es un login corriente: fila en `sessions` + cookie acotada al prefijo.
    let expires_at = Utc::now() + Duration::days(state.session_ttl_days);
    let sid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO sessions (user_id, expires_at) VALUES ($1, $2) RETURNING id"#,
    )
    .bind(user_id)
    .bind(expires_at)
    .fetch_one(&state.pool)
    .await?;
    let jar = jar.add(session_cookie(&state, sid, session_cookie_path(&state, &headers)));

    let row: UserRow = sqlx::query_as(r#"SELECT id, username, birth_date FROM users WHERE id = $1"#)
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?;

    // Mismo warm-up en background que `login` (D7: no se espera al recompute). Usuario pending
    // ⇒ no hay nada que calentar y se salta en silencio.
    if let Ok((iid, _)) = require_installation_member(&state.pool, user_id).await {
        let state_clone = state.clone();
        tokio::spawn(async move {
            warm_up_household_projection(state_clone, iid, user_id).await;
        });
    }

    Ok((jar, Json(user_row_to_response(row))))
}

fn header_text(headers: &http::HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(name)?;
    let text = String::from_utf8_lossy(raw.as_bytes()).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Busca al usuario por `external_user_id` y, si no existe, lo crea. Devuelve su id.
async fn resolve_or_provision(
    state: &Arc<AppState>,
    external_user_id: Uuid,
    raw_name: &str,
) -> Result<Uuid, ApiError> {
    if let Some(id) = find_by_external_id(state, external_user_id).await? {
        return Ok(id);
    }

    let base = username_slug(raw_name);
    // Candidatos en orden: el slug, cuatro sufijos numéricos y, si todo eso choca, un nombre
    // derivado del propio id externo — que es único por construcción y cierra el bucle.
    let mut candidates: Vec<String> = vec![base.clone()];
    for n in 2..=5 {
        candidates.push(format!("{base}-{n}"));
    }
    candidates.push(format!(
        "ha-{}",
        &external_user_id.simple().to_string()[..8]
    ));

    for username in candidates {
        match try_provision(state, external_user_id, &username).await? {
            Provision::Created(id) => {
                tracing::info!(user_id = %id, %username, "provisioned account from trusted proxy identity");
                return Ok(id);
            }
            Provision::Existing(id) => return Ok(id),
            Provision::UsernameTaken => continue,
        }
    }

    Err(ApiError::ConflictWith(
        "sso_username_unavailable: could not derive a free username for this identity".into(),
    ))
}

enum Provision {
    Created(Uuid),
    /// Otra petición concurrente provisionó la misma identidad externa entremedias.
    Existing(Uuid),
    UsernameTaken,
}

/// Un intento de alta, en UNA transacción: usuario + bootstrap del hogar si es el primero.
///
/// Cada intento abre su propia transacción a propósito: en Postgres una violación de unique
/// aborta la transacción entera, así que reintentar dentro de la misma no es posible.
async fn try_provision(
    state: &Arc<AppState>,
    external_user_id: Uuid,
    username: &str,
) -> Result<Provision, ApiError> {
    let mut tx = state.pool.begin().await?;
    let inserted: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        r#"INSERT INTO users (username, password_hash, birth_date, external_user_id)
           VALUES ($1, NULL, NULL, $2)
           RETURNING id"#,
    )
    .bind(username)
    .bind(external_user_id)
    .fetch_one(&mut *tx)
    .await;

    let user_id = match inserted {
        Ok(id) => id,
        Err(e) => {
            let constraint = match &e {
                sqlx::Error::Database(db) => db.constraint().map(str::to_string),
                _ => None,
            };
            drop(tx);
            return match constraint.as_deref() {
                Some("users_username_key") => Ok(Provision::UsernameTaken),
                // Doble provisión simultánea de la MISMA identidad: la otra ganó. No es un
                // error para quien llama — su usuario existe, que es lo que pedía.
                Some("users_external_user_id_key") => find_by_external_id(state, external_user_id)
                    .await?
                    .map(Provision::Existing)
                    .ok_or_else(|| ApiError::from(e)),
                _ => Err(ApiError::from(e)),
            };
        }
    };

    match bootstrap_installation_as_owner_if_empty(&mut tx, &user_id).await {
        Ok(()) => {}
        Err(ApiError::Conflict) => {
            return Err(ApiError::ConflictWith(
                "installation_race: another registration created the installation first".into(),
            ));
        }
        Err(e) => return Err(e),
    }
    tx.commit().await?;
    Ok(Provision::Created(user_id))
}

async fn find_by_external_id(
    state: &Arc<AppState>,
    external_user_id: Uuid,
) -> Result<Option<Uuid>, ApiError> {
    Ok(
        sqlx::query_scalar(r#"SELECT id FROM users WHERE external_user_id = $1"#)
            .bind(external_user_id)
            .fetch_optional(&state.pool)
            .await?,
    )
}

/// Convierte un nombre humano en un `username` válido (`^[a-z0-9._-]{3,64}$`).
///
/// Sin dependencias de Unicode: para un slug basta con plegar los diacríticos del español (el
/// idioma de la UI) y mandar todo lo demás a `-`. Añadir un crate de normalización por «José»
/// sería pagar una dependencia de la cadena de suministro por siete letras.
pub(crate) fn username_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_dash = false;
    for c in raw.chars().flat_map(fold_char) {
        let c = c.to_ascii_lowercase();
        let keep = c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
        let c = if keep { c } else { '-' };
        // Colapsa las rachas de '-' (un nombre con dos espacios no da dos guiones).
        if c == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(c);
        if out.chars().count() >= USERNAME_SLUG_MAX {
            break;
        }
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '.').to_string();
    if trimmed.is_empty() {
        // Sin nombre utilizable (cabeceras ausentes, o un nombre entero en otro alfabeto): un
        // nombre fijo y legible; los choques los resuelven los sufijos de `resolve_or_provision`.
        return "ha-user".to_string();
    }
    if trimmed.chars().count() < 3 {
        return format!("{trimmed}-ha");
    }
    trimmed
}

/// Pliega los diacríticos del español a ASCII. Lo que no reconoce lo deja pasar tal cual (y el
/// filtro de charset lo convertirá en `-`).
fn fold_char(c: char) -> std::vec::IntoIter<char> {
    let folded: Vec<char> = match c {
        'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => vec!['a'],
        'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => vec!['e'],
        'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => vec!['i'],
        'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => vec!['o'],
        'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => vec!['u'],
        'ñ' | 'Ñ' => vec!['n'],
        'ç' | 'Ç' => vec!['c'],
        'ß' => vec!['s', 's'],
        other => vec![other],
    };
    folded.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_folds_spanish_diacritics_and_spaces() {
        assert_eq!(username_slug("José Ñandú García"), "jose-nandu-garcia");
        assert_eq!(username_slug("María"), "maria");
        assert_eq!(username_slug("  Pepe   Viyuela  "), "pepe-viyuela");
    }

    #[test]
    fn slug_is_always_a_valid_username() {
        for raw in ["", "!!!", "---", "私", "a", "..x..", &"z".repeat(200)] {
            let s = username_slug(raw);
            let len = s.chars().count();
            assert!((3..=64).contains(&len), "{raw:?} → {s:?} (len {len})");
            assert!(
                s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')),
                "{raw:?} → {s:?}"
            );
        }
    }

    #[test]
    fn slug_without_usable_characters_falls_back() {
        assert_eq!(username_slug(""), "ha-user");
        assert_eq!(username_slug("!!!"), "ha-user");
        assert_eq!(username_slug("私"), "ha-user");
        // Dos caracteres útiles se rellenan hasta el mínimo de 3.
        assert_eq!(username_slug("Al"), "al-ha");
    }
}
