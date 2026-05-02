use crate::error::ApiError;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand_core::OsRng;

pub fn hash_password(password: &str) -> Result<String, ApiError> {
    validate_password_strength(password)?;
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    Ok(argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| ApiError::BadRequest("could not hash password".into()))?
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

fn validate_password_strength(password: &str) -> Result<(), ApiError> {
    let len = password.chars().count();
    if !(12..=256).contains(&len) {
        return Err(ApiError::BadRequest(
            "password must be between 12 and 256 characters".into(),
        ));
    }
    Ok(())
}
