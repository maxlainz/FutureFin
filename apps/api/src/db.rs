use sqlx::migrate::MigrateError;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
}

/// `ALTER … IF NOT EXISTS` sobre `users.birth_date`: si el checksum en `_sqlx_migrations` no
/// coincide con el archivo (renombres / squash locales), borrar la fila y reaplicar es seguro.
const IDEMPOTENT_MIGRATION_REPAIR_VERSIONS: &[i64] = &[20260502140000, 20260504120000];

/// Ejecuta migraciones; ante checksum incorrecto en versiones idempotentes conocidas, repara y reintenta.
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    const MAX_REPAIR_ROUNDS: usize = 12;
    for round in 0..MAX_REPAIR_ROUNDS {
        match sqlx::migrate!("./migrations").run(pool).await {
            Ok(()) => return Ok(()),
            Err(MigrateError::VersionMismatch(v))
                if IDEMPOTENT_MIGRATION_REPAIR_VERSIONS.contains(&v) =>
            {
                tracing::warn!(
                    version = v,
                    round,
                    "migration checksum mismatch — deleting _sqlx_migrations row for idempotent migration and retrying"
                );
                sqlx::query(r#"DELETE FROM _sqlx_migrations WHERE version = $1"#)
                    .bind(v)
                    .execute(pool)
                    .await
                    .map_err(|e| MigrateError::Execute(e.into()))?;
            }
            Err(e) => return Err(e),
        }
    }
    tracing::error!("migration repair loop exhausted after {MAX_REPAIR_ROUNDS} rounds");
    sqlx::migrate!("./migrations").run(pool).await
}
