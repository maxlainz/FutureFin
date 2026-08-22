use crate::error::ApiError;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;
use std::sync::OnceLock;

pub fn hash_password(password: &str) -> Result<String, ApiError> {
    validate_password_strength(password)?;
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    Ok(argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| ApiError::BadRequest("password_hash_failed: could not hash password".into()))?
        .to_string())
}

pub fn verify_password(password: &str, stored: &str) -> Result<(), ApiError> {
    let parsed = PasswordHash::new(stored)
        .map_err(|_| ApiError::Unauthorized)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| ApiError::Unauthorized)?;
    Ok(())
}

/// Argon2id cuesta ~40-80 ms de CPU **pura**. Ejecutarlo en un worker del reactor de Tokio
/// deja ese hilo bloqueado: con `num_cpus` peticiones concurrentes a `/v1/auth/register` —
/// endpoint sin autenticación por diseño — se para el proceso entero, `GET /v1/ready`
/// incluido, y el healthcheck del contenedor lo marca unhealthy. `spawn_blocking` lo manda al
/// pool de blocking, que está acotado y aislado del reactor. Mismo patrón que el motor de
/// proyección (`handlers/projection.rs`), que ya lo hacía siendo bastante más barato.
pub async fn hash_password_blocking(password: &str) -> Result<String, ApiError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|_| ApiError::Unavailable)?
}

/// Verifica fuera del reactor y **sin oráculo de timing**: cuando el usuario no existe
/// (`stored = None`) se verifica igualmente contra un hash constante antes de devolver 401.
/// Sin eso, un usuario inexistente responde en ~1 ms y uno existente con contraseña mala en
/// ~40-80 ms — dos órdenes de magnitud, medibles por red, que enumeran quién tiene cuenta.
pub async fn verify_password_blocking(
    password: &str,
    stored: Option<String>,
) -> Result<(), ApiError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || match stored {
        Some(hash) => verify_password(&password, &hash),
        None => {
            let _ = verify_password(&password, dummy_hash());
            Err(ApiError::Unauthorized)
        }
    })
    .await
    .map_err(|_| ApiError::Unavailable)?
}

/// Hash PHC de descarte con los mismos parámetros que los reales, para que la rama
/// «usuario inexistente» cueste lo mismo que la rama «contraseña incorrecta».
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"timing-equalizer, never a real credential", &salt)
            .expect("hashing a constant with default params cannot fail")
            .to_string()
    })
}

fn validate_password_strength(password: &str) -> Result<(), ApiError> {
    let len = password.chars().count();
    if !(12..=256).contains(&len) {
        return Err(ApiError::BadRequest(
            "password_length: password must be between 12 and 256 characters".into(),
        ));
    }
    Ok(())
}
