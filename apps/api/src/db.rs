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

/// Error de arranque al aplicar migraciones.
///
/// Existe por un único motivo: convertir el `VersionMissing` crudo de sqlx —la firma exacta de
/// «imagen vieja arrancada sobre datos nuevos»— en un mensaje que un self-hoster pueda accionar,
/// al nivel de las guardas del entrypoint (`PGDATA was created by PostgreSQL N, NEWER than …`).
/// Cualquier otro error de sqlx pasa **tal cual**: el desajuste de checksum conserva su semántica
/// y su mensaje, y **nunca** se auto-repara (ver skill `futurefin-change-control` §2.7).
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// La base tiene aplicada una migración que este binario no lleva embebida.
    #[error("{message}")]
    Downgrade { version: i64, message: String },
    /// Todo lo demás, sin reinterpretar.
    #[error(transparent)]
    Other(MigrateError),
}

/// Mensaje de operador para el caso downgrade. Va al log que lee quien se autohospeda, así que
/// dice qué ha pasado, tranquiliza sobre los datos y da los dos caminos de salida.
fn downgrade_message(version: i64) -> String {
    format!(
        "\n\
         ─────────────────────────────────────────────────────────────────────────────\n\
         FutureFin NO ARRANCA: esta base de datos viene de una versión MÁS NUEVA.\n\
         ─────────────────────────────────────────────────────────────────────────────\n\
         La base tiene aplicada la migración {version}, que este binario (versión {app})\n\
         no conoce. Es la firma de haber arrancado una imagen antigua sobre datos ya\n\
         migrados por una imagen posterior.\n\
         \n\
         TUS DATOS ESTÁN INTACTOS: no se ha tocado nada. FutureFin prefiere no arrancar\n\
         antes que ejecutar un esquema viejo sobre datos nuevos.\n\
         \n\
         Qué hacer:\n\
           1) Vuelve al tag de imagen que estabas usando (el más nuevo). Es la salida\n\
              normal: basta con corregir FUTUREFIN_TAG y volver a levantar.\n\
           2) Si de verdad quieres quedarte en esta versión, restaura el backup\n\
              pre-migración: el fichero `pre-migration-*.sql.gz` del volumen `ffdata`\n\
              (en el add-on de Home Assistant: /data/state/backups).\n\
         \n\
         Guía completa de backups y restauración: docs/backups.md\n\
         ─────────────────────────────────────────────────────────────────────────────",
        version = version,
        app = env!("CARGO_PKG_VERSION"),
    )
}

/// Aplica migraciones embebidas (`sqlx::migrate!`). Si una migración pre-existente cambia su
/// checksum (por ej. tras un squash), el error queda visible — repararlo manualmente con
/// `DELETE FROM _sqlx_migrations WHERE version = X` desde `psql` y reintentar.
///
/// La guarda de downgrade no añade ninguna comprobación propia: sqlx ya falla con
/// `VersionMissing` cuando la base va por delante del binario; aquí solo se le pone el mensaje.
pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrationError> {
    match sqlx::migrate!("./migrations").run(pool).await {
        Ok(()) => Ok(()),
        Err(MigrateError::VersionMissing(version)) => Err(MigrationError::Downgrade {
            version,
            message: downgrade_message(version),
        }),
        Err(other) => Err(MigrationError::Other(other)),
    }
}
