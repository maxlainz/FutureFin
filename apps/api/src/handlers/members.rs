//! Gestión de las membresías ya concedidas: listar, cambiar rol y **revocar**.
//!
//! Hasta 4.0.0 `installation_memberships` solo recibía `INSERT` (bootstrap del primer usuario,
//! `setup`, y la aprobación de un pendiente): no existía forma de degradar ni de expulsar a
//! nadie desde la aplicación. El mecanismo de revocación *sí* estaba —el rol y la pertenencia
//! se re-resuelven en cada petición, así que quitar la fila corta el acceso al instante—, pero
//! no había ninguna palanca que lo accionara: aprobar al usuario equivocado concedía acceso
//! permanente a todas las finanzas del hogar, y el único remedio era un `DELETE` a mano por
//! `psql`. `SECURITY.md` y `.claude/auth-and-membership.md` prometían lo contrario.
//!
//! Revocar **no borra los datos** de la persona. Sus movimientos, snapshots y reglas siguen
//! ligados a su `owner_user_id`: si se la vuelve a aprobar, los recupera intactos. Lo que se
//! corta es el acceso — y se corta entero: membresía, sesiones abiertas, tokens de API y
//! concesiones OAuth, todo en la misma transacción.

use crate::error::ApiError;
use crate::handlers::installation::{singleton_installation_id, user_is_installation_owner};
use crate::handlers::membership::MembershipRole;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema, FromRow)]
pub struct MemberResponse {
    pub user_id: Uuid,
    pub username: String,
    /// `owner` | `member` | `viewer`.
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

/// Roles asignables desde este endpoint. `owner` está incluido a propósito: un hogar debe poder
/// traspasar la propiedad sin pasar por `psql`. La guardia del último owner impide quedarse sin
/// ninguno.
#[derive(Debug, Deserialize, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssignableRole {
    Owner,
    Member,
    Viewer,
}

impl AssignableRole {
    fn to_membership(self) -> MembershipRole {
        match self {
            AssignableRole::Owner => MembershipRole::Owner,
            AssignableRole::Member => MembershipRole::Member,
            AssignableRole::Viewer => MembershipRole::Viewer,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMemberRoleBody {
    pub role: AssignableRole,
}

/// Owner de la instalación + su id. Todos los endpoints de escritura de este módulo lo exigen.
async fn require_owner(
    state: &Arc<AppState>,
    jar: &axum_extra::extract::cookie::CookieJar,
) -> Result<(Uuid, Uuid), ApiError> {
    let user = require_session_user(jar, &state.pool).await?;
    let Some(iid) = singleton_installation_id(&state.pool).await? else {
        return Err(ApiError::NotFound);
    };
    if !user_is_installation_owner(&state.pool, user.id.0, iid).await? {
        return Err(ApiError::Forbidden);
    }
    Ok((iid, user.id.0))
}

#[utoipa::path(
    get,
    path = "/v1/installation/members",
    tag = "installation",
    responses(
        (status = 200, description = "Miembros de la instalación", body = [MemberResponse]),
        (status = 401, description = "Sin sesión válida"),
        (status = 403, description = "Sin membresía"),
        (status = 404, description = "No hay instalación"),
    )
)]
pub async fn list_members(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
) -> Result<Json<Vec<MemberResponse>>, ApiError> {
    // Lectura abierta a cualquier miembro: todos comparten los mismos datos financieros, así
    // que saber quién más tiene acceso no revela nada nuevo — y es justo lo que permite
    // auditar el hogar.
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) =
        crate::handlers::installation::require_installation_member(&state.pool, user.id.0).await?;

    let rows: Vec<MemberResponse> = sqlx::query_as(
        r#"SELECT m.user_id, u.username, m.role, m.created_at AS joined_at
           FROM installation_memberships m
           JOIN users u ON u.id = m.user_id
           WHERE m.installation_id = $1
           ORDER BY
               CASE m.role WHEN 'owner' THEN 0 WHEN 'member' THEN 1 ELSE 2 END,
               u.username"#,
    )
    .bind(iid)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(rows))
}

/// Bloquea **todas** las filas de owner de la instalación, en orden de `user_id`, y devuelve
/// sus ids. Es el primer paso de cualquier mutación de membresía, y el orden importa por dos
/// razones distintas:
///
/// - **Corrección**: sin `FOR UPDATE`, dos owners degradándose a la vez podrían dejar el hogar
///   sin ninguno. Cada transacción vería al otro y las dos se creerían a salvo.
/// - **Deadlock**: bloquear primero la fila del objetivo y después el resto de owners es el
///   patrón ABBA de manual. Dos owners degradándose simultáneamente se bloqueaban en cruz,
///   Postgres abortaba una con SQLSTATE 40P01 tras un segundo, y eso salía como **500 internal
///   error** — el invariante aguantaba, el contrato de error no. Con un orden total (`ORDER BY
///   user_id`) fijado antes de tocar nada, el ciclo no se puede formar.
async fn lock_owner_ids(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    iid: Uuid,
) -> Result<Vec<Uuid>, ApiError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT user_id FROM installation_memberships
           WHERE installation_id = $1 AND role = 'owner'
           ORDER BY user_id
           FOR UPDATE"#,
    )
    .bind(iid)
    .fetch_all(&mut **tx)
    .await?;
    Ok(ids)
}

/// Reconfirma **dentro de la transacción** que quien llama sigue siendo owner.
///
/// `require_owner` consulta con el pool, fuera de la transacción, así que entre esa lectura y
/// el commit puede haber pasado cualquier cosa: un owner al que acaban de expulsar —con la
/// sesión ya borrada— podía completar igualmente su expulsión de un tercero. La ventana es de
/// milisegundos y hacen falta dos owners hostiles a la vez, pero cerrarla cuesta una consulta.
fn assert_still_owner(me: Uuid, owners: &[Uuid]) -> Result<(), ApiError> {
    if owners.contains(&me) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[utoipa::path(
    patch,
    path = "/v1/installation/members/{user_id}",
    tag = "installation",
    params(("user_id" = Uuid, Path, description = "Id del miembro")),
    request_body = UpdateMemberRoleBody,
    responses(
        (status = 204, description = "Rol actualizado"),
        (status = 400, description = "Rol inválido, o sería el último owner"),
        (status = 401, description = "Sin sesión válida"),
        (status = 403, description = "No es owner de la instalación"),
        (status = 404, description = "El usuario no es miembro"),
    )
)]
pub async fn update_member_role(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path(target_user_id): Path<Uuid>,
    Json(body): Json<UpdateMemberRoleBody>,
) -> Result<axum::http::StatusCode, ApiError> {
    let (iid, _me) = require_owner(&state, &jar).await?;
    let new_role = body.role.to_membership();

    let mut tx = state.pool.begin().await?;
    // Orden fijo: owners primero (ordenados), objetivo después. Ver `lock_owner_ids`.
    let owners = lock_owner_ids(&mut tx, iid).await?;
    assert_still_owner(_me, &owners)?;
    let current: Option<String> = sqlx::query_scalar(
        r#"SELECT role FROM installation_memberships
           WHERE installation_id = $1 AND user_id = $2
           FOR UPDATE"#,
    )
    .bind(iid)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };

    if current == "owner"
        && new_role != MembershipRole::Owner
        && owners.iter().all(|o| *o == target_user_id)
    {
        return Err(ApiError::BadRequest(
            "last_owner: no puedes dejar la instalación sin ningún propietario".into(),
        ));
    }

    sqlx::query(
        r#"UPDATE installation_memberships SET role = $3
           WHERE installation_id = $1 AND user_id = $2"#,
    )
    .bind(iid)
    .bind(target_user_id)
    .bind(new_role.as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/v1/installation/members/{user_id}",
    tag = "installation",
    params(("user_id" = Uuid, Path, description = "Id del miembro")),
    responses(
        (status = 204, description = "Acceso revocado; los datos de la persona se conservan"),
        (status = 400, description = "Sería el último owner"),
        (status = 401, description = "Sin sesión válida"),
        (status = 403, description = "No es owner de la instalación"),
        (status = 404, description = "El usuario no es miembro"),
    )
)]
pub async fn remove_member(
    Extension(state): Extension<Arc<AppState>>,
    jar: axum_extra::extract::cookie::CookieJar,
    Path(target_user_id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let (iid, _me) = require_owner(&state, &jar).await?;

    let mut tx = state.pool.begin().await?;
    let owners = lock_owner_ids(&mut tx, iid).await?;
    assert_still_owner(_me, &owners)?;
    let current: Option<String> = sqlx::query_scalar(
        r#"SELECT role FROM installation_memberships
           WHERE installation_id = $1 AND user_id = $2
           FOR UPDATE"#,
    )
    .bind(iid)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };
    if current == "owner" && owners.iter().all(|o| *o == target_user_id) {
        return Err(ApiError::BadRequest(
            "last_owner: no puedes dejar la instalación sin ningún propietario".into(),
        ));
    }

    // El corte es completo y atómico: sin esto, la persona conservaría acceso por cualquiera de
    // las otras tres credenciales durante días (la sesión dura `SESSION_TTL_DAYS`, y un token
    // `ffp_` puede no caducar nunca).
    sqlx::query(r#"DELETE FROM installation_memberships WHERE installation_id = $1 AND user_id = $2"#)
        .bind(iid)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(r#"DELETE FROM sessions WHERE user_id = $1"#)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"UPDATE api_tokens SET revoked_at = now()
           WHERE user_id = $1 AND revoked_at IS NULL"#,
    )
    .bind(target_user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"UPDATE oauth_grants SET revoked_at = now(), revoked_reason = 'membership_revoked'
           WHERE user_id = $1 AND revoked_at IS NULL"#,
    )
    .bind(target_user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Sus entradas de la cache de proyección llevan SU demografía: fuera.
    state.invalidate_projection_by_user(target_user_id).await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub fn members_router() -> Router {
    Router::new().route("/", get(list_members)).route(
        "/{user_id}",
        axum::routing::patch(update_member_role).delete(remove_member),
    )
}
