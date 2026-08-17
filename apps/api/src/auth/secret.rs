//! Generación y hashing de secretos opacos (tokens de API y credenciales OAuth).
//!
//! Contrato D14 compartido por ambos esquemas: el secreto viaja UNA vez y solo se
//! persiste su SHA-256 hex; el lookup es O(1) por hash exacto y no hay comparación de
//! secretos en Rust (cero superficie de timing). Consumido por `handlers/api_tokens.rs`
//! (`ffp_`) y por `oauth/` (`ffc_`/`ffcs_`/`ffo_`/`ffr_`).

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use base64::Engine;
use rand_core::{OsRng, RngCore};

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// `{prefix}{base64url_no_pad(32 bytes de OsRng)}` — 43 chars de secreto, ~256 bits.
pub fn generate_opaque_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", B64URL.encode(bytes))
}

/// Identificadores públicos (p.ej. `client_id`): 16 bytes → 22 chars. No son secretos,
/// pero la aleatoriedad evita colisiones y enumeración.
pub fn generate_opaque_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", B64URL.encode(bytes))
}
