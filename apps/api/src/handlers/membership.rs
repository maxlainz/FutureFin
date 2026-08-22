use crate::error::ApiError;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MembershipRole {
    Owner,
    Member,
    Viewer,
}

impl MembershipRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MembershipRole::Owner => "owner",
            MembershipRole::Member => "member",
            MembershipRole::Viewer => "viewer",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            "viewer" => Ok(Self::Viewer),
            _ => Err(ApiError::BadRequest("membership_role_invalid: invalid membership role".into())),
        }
    }
}

pub async fn membership_role(
    pool: &PgPool,
    user_id: Uuid,
    installation_id: Uuid,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT role FROM installation_memberships
           WHERE installation_id = $1 AND user_id = $2"#,
    )
    .bind(installation_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Db)
}

/// Used by handlers that distinguish read-only (`viewer`) membership.
#[allow(dead_code)]
pub fn role_can_read(role: &str) -> bool {
    matches!(role, "owner" | "member" | "viewer")
}

pub fn role_can_write(role: &str) -> bool {
    matches!(role, "owner" | "member")
}
