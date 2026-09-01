use crate::error::ApiError;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use std::sync::OnceLock;

/// `Argon2::default()` fija los parámetros que este binario PERSISTE en `users.password_hash`:
/// m=19456, t=2, p=1, salida 32 B. Se dejan en el default a propósito —no se fijan a mano— porque
/// argon2 0.6 mantiene `DEFAULT_M_COST = 19 * 1024`, `DEFAULT_T_COST = 2`, `DEFAULT_P_COST = 1` y
/// `DEFAULT_OUTPUT_LEN = 32`, idénticos a 0.5.3. El cinturón de vectores congelados
/// (`tests/crypto_frozen_vectors.rs`) vigila justo eso: si un bump futuro moviera esos defaults,
/// `el_registro_de_hoy_conserva_los_parametros_del_hash_dorado` se pone rojo y obliga a fijarlos
/// explícitos aquí en vez de dejar que la postura de seguridad cambie en silencio.
pub fn hash_password(password: &str) -> Result<String, ApiError> {
    validate_password_strength(password)?;
    let argon2 = Argon2::default();
    // password-hash 0.6 retiró `SaltString` y genera la sal DENTRO de `hash_password`
    // (16 bytes de `getrandom`, la misma longitud que producía `SaltString::generate(&mut OsRng)`).
    Ok(argon2
        .hash_password(password.as_bytes())
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
/// pool de blocking, aislado del reactor. Pero ese pool tiene 512 hilos por defecto, así que
/// mandarlo ahí sin más subía el techo de Argon2 concurrente de `num_cpus` a 512 — de ~38 MiB
/// a ~9,5 GiB de reserva— y convertía una caída recuperable en un OOM-kill del contenedor con
/// PostgreSQL dentro. Por eso va por `heavy::run_password_kdf`, que además lo acota.
pub async fn hash_password_blocking(password: &str) -> Result<String, ApiError> {
    let password = password.to_owned();
    crate::heavy::run_password_kdf(move || hash_password(&password)).await?
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
    crate::heavy::run_password_kdf(move || match stored {
        Some(hash) => verify_password(&password, &hash),
        None => {
            let _ = verify_password(&password, dummy_hash());
            Err(ApiError::Unauthorized)
        }
    })
    .await?
}

/// Hash PHC de descarte con los mismos parámetros que los reales, para que la rama
/// «usuario inexistente» cueste lo mismo que la rama «contraseña incorrecta».
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        Argon2::default()
            .hash_password(b"timing-equalizer, never a real credential")
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
