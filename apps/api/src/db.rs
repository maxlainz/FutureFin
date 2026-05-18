use sqlx::migrate::MigrateError;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(1800))
        .connect(database_url)
        .await
}

/// Aplica migraciones embebidas (`sqlx::migrate!`). Si una migración pre-existente cambia su
/// checksum (por ej. tras un squash), el error queda visible — repararlo manualmente con
/// `DELETE FROM _sqlx_migrations WHERE version = X` desde `psql` y reintentar.
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
