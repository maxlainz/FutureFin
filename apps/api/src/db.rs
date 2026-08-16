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

/// Como `connect`, pero reintenta con backoff (0.5s → 1s → 2s → 4s → 4s…) hasta agotar
/// `max_wait`. En el contenedor único el entrypoint ya espera a `pg_isready`, pero en el
/// modo compat con base EXTERNA no existe `depends_on: service_healthy` que garantice el
/// orden de arranque — sin esto, cada recreate contra una base lenta sería un crash-loop.
pub async fn connect_with_retry(
    database_url: &str,
    max_wait: Duration,
) -> Result<PgPool, sqlx::Error> {
    let start = std::time::Instant::now();
    let mut delay = Duration::from_millis(500);
    loop {
        match connect(database_url).await {
            Ok(pool) => return Ok(pool),
            Err(err) => {
                if start.elapsed() + delay > max_wait {
                    return Err(err);
                }
                tracing::warn!(error = %err, retry_in_s = delay.as_secs_f32(), "database not ready yet — retrying");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(4));
            }
        }
    }
}

/// Aplica migraciones embebidas (`sqlx::migrate!`). Si una migración pre-existente cambia su
/// checksum (por ej. tras un squash), el error queda visible — repararlo manualmente con
/// `DELETE FROM _sqlx_migrations WHERE version = X` desde `psql` y reintentar.
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
