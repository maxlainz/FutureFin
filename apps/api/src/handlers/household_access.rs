use crate::error::ApiError;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn membership_role(
    pool: &PgPool,
    user_id: Uuid,
    household_id: Uuid,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT role FROM household_memberships
           WHERE household_id = $1 AND user_id = $2"#,
    )
    .bind(household_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Db)
}

pub fn role_can_read(role: &str) -> bool {
    matches!(role, "owner" | "member" | "viewer")
}

pub fn role_can_write(role: &str) -> bool {
    matches!(role, "owner" | "member")
}
