use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
    pub pool: PgPool,
    pub cookie_secure: bool,
    pub session_ttl_days: i64,
}
