//! Guarda de downgrade: arrancar un binario ANTIGUO sobre una base ya migrada por uno más
//! nuevo debe fallar en alto y con un mensaje accionable — nunca «arreglarse» solo.
//!
//! Lo que se prueba es el contrato completo, no la implementación: sqlx ya devuelve
//! `VersionMissing` cuando `_sqlx_migrations` tiene una versión que el binario no lleva
//! embebida (esa ES la firma del downgrade); lo nuestro es (1) no perder ese fallo y
//! (2) traducirlo a un mensaje que un self-hoster pueda accionar.
//!
//! Simulación: se aplican las migraciones reales en un schema aislado y después se inserta a
//! mano una fila «del futuro» en `_sqlx_migrations`. Es exactamente lo que vería el binario
//! viejo tras un upgrade.

// Este test solo usa `isolated_pool` del harness; el resto (TestApp y sus helpers) no se toca.
#[allow(dead_code)]
mod common;

use futurefin_api::db::{run_migrations, MigrationError};

/// Versión inventada, muy por encima de cualquier `YYYYMMDDHHMMSS` real del repo.
const FUTURE_VERSION: i64 = 99_999_999_999_999;

#[tokio::test]
async fn downgrade_over_a_newer_database_fails_with_an_operator_message() {
    let (pool, _schema) = common::isolated_pool().await;

    // El helper ya aplicó todas las migraciones embebidas en este schema; añadimos una que
    // este binario NO conoce, como si la hubiera escrito una versión posterior.
    sqlx::query(
        "INSERT INTO _sqlx_migrations \
         (version, description, installed_on, success, checksum, execution_time) \
         VALUES ($1, 'future', now(), true, $2, 0)",
    )
    .bind(FUTURE_VERSION)
    .bind(vec![0u8; 48])
    .execute(&pool)
    .await
    .expect("insertar la migración del futuro");

    let err = run_migrations(&pool)
        .await
        .expect_err("con una migración desconocida en la base, migrar debe fallar");

    let MigrationError::Downgrade { version, message } = err else {
        panic!("se esperaba la variante Downgrade, llegó: {err:?}");
    };
    assert_eq!(version, FUTURE_VERSION);

    // El mensaje es el producto de esta guarda: si se degrada, la guarda deja de servir.
    assert!(
        message.contains("TUS DATOS ESTÁN INTACTOS"),
        "el mensaje debe tranquilizar sobre los datos:\n{message}"
    );
    assert!(
        message.contains("pre-migration"),
        "el mensaje debe nombrar el backup pre-migración:\n{message}"
    );
    assert!(
        message.contains(&FUTURE_VERSION.to_string()),
        "el mensaje debe nombrar la migración desconocida:\n{message}"
    );
    assert!(
        message.contains(env!("CARGO_PKG_VERSION")),
        "el mensaje debe nombrar la versión del binario:\n{message}"
    );
}

/// Sin fila del futuro, migrar sobre un schema ya migrado es un no-op verde: la guarda no
/// puede volverse un falso positivo que impida arrancar dos veces seguidas.
#[tokio::test]
async fn running_migrations_twice_is_a_green_no_op() {
    let (pool, _schema) = common::isolated_pool().await;
    run_migrations(&pool)
        .await
        .expect("re-migrar un schema al día no debe fallar");
}
