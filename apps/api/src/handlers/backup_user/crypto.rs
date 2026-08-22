//! KDF (Argon2id), AES-256-GCM, and magic-header framing for `.ffbackup`.
//!
//! File layout:
//! ```text
//! [magic "FFBK" 4 bytes][format_version u8][manifest_len u32 LE][manifest_json][ciphertext]
//! ```

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

use super::schema::{MAGIC, SUPPORTED_FORMAT_VERSION};

const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

const KDF_M_COST: u32 = 19_456;
const KDF_T_COST: u32 = 2;
const KDF_P_COST: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestKdf {
    pub alg: String,
    pub salt_b64: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub out_len: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestCipher {
    pub alg: String,
    pub nonce_b64: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub user_id_original: String,
    pub username_original: String,
    pub kdf: ManifestKdf,
    pub cipher: ManifestCipher,
    pub compression: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("bad input: {0}")]
    Bad(String),
    #[error("decryption failed (wrong password or corrupted file)")]
    Decrypt,
    #[error("internal")]
    Internal,
}

fn derive_key(password: &str, salt: &[u8], params: &Params) -> Result<[u8; KEY_LEN], CryptoError> {
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone());
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|_| CryptoError::Internal)?;
    Ok(out)
}

fn kdf_params() -> Params {
    Params::new(KDF_M_COST, KDF_T_COST, KDF_P_COST, Some(KEY_LEN))
        .expect("argon2 params constants are valid")
}

fn gzip_compress(plain: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(plain).map_err(|_| CryptoError::Internal)?;
    enc.finish().map_err(|_| CryptoError::Internal)
}

fn gzip_decompress(zipped: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let mut dec = GzDecoder::new(zipped);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|_| CryptoError::Bad("backup_file_corrupt: payload decompression failed".into()))?;
    Ok(out)
}

pub struct EncryptOutput {
    pub manifest: Manifest,
    pub ciphertext: Vec<u8>,
}

pub fn encrypt_payload(
    plaintext_json: &[u8],
    password: &str,
    app_version: &str,
    schema_version: u32,
    user_id_original: &str,
    username_original: &str,
    exported_at_iso: &str,
) -> Result<EncryptOutput, CryptoError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let params = kdf_params();
    let key = derive_key(password, &salt, &params)?;

    let compressed = gzip_compress(plaintext_json)?;

    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| CryptoError::Internal)?;
    let aad = build_aad(schema_version, user_id_original, exported_at_iso);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &compressed,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::Internal)?;

    let manifest = Manifest {
        schema_version,
        app_version: app_version.into(),
        exported_at: exported_at_iso.into(),
        user_id_original: user_id_original.into(),
        username_original: username_original.into(),
        kdf: ManifestKdf {
            alg: "argon2id".into(),
            salt_b64: B64.encode(salt),
            m_cost: KDF_M_COST,
            t_cost: KDF_T_COST,
            p_cost: KDF_P_COST,
            out_len: KEY_LEN as u32,
        },
        cipher: ManifestCipher {
            alg: "aes-256-gcm".into(),
            nonce_b64: B64.encode(nonce),
        },
        compression: "gzip".into(),
    };

    Ok(EncryptOutput { manifest, ciphertext })
}

pub fn frame_file(manifest: &Manifest, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let manifest_json = serde_json::to_vec(manifest).map_err(|_| CryptoError::Internal)?;
    let manifest_len = u32::try_from(manifest_json.len())
        .map_err(|_| CryptoError::Internal)?;
    let mut out =
        Vec::with_capacity(4 + 1 + 4 + manifest_json.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.push(SUPPORTED_FORMAT_VERSION);
    out.extend_from_slice(&manifest_len.to_le_bytes());
    out.extend_from_slice(&manifest_json);
    out.extend_from_slice(ciphertext);
    Ok(out)
}

pub struct ParsedFrame {
    pub manifest: Manifest,
    pub ciphertext: Vec<u8>,
}

pub fn parse_frame(bytes: &[u8]) -> Result<ParsedFrame, CryptoError> {
    if bytes.len() < 4 + 1 + 4 {
        return Err(CryptoError::Bad("backup_file_corrupt: file too short".into()));
    }
    if &bytes[0..4] != MAGIC {
        return Err(CryptoError::Bad("backup_not_a_ffbackup_file: not a .ffbackup file (bad magic)".into()));
    }
    let fmt = bytes[4];
    if fmt > SUPPORTED_FORMAT_VERSION {
        return Err(CryptoError::Bad(format!(
            "backup_format_version_unsupported: file format version {fmt} not supported"
        )));
    }
    let manifest_len = u32::from_le_bytes(
        bytes[5..9]
            .try_into()
            .map_err(|_| CryptoError::Bad("backup_file_corrupt: malformed length prefix".into()))?,
    ) as usize;
    let start: usize = 9;
    let end = start
        .checked_add(manifest_len)
        .ok_or_else(|| CryptoError::Bad("backup_file_corrupt: manifest length overflows".into()))?;
    if end > bytes.len() {
        return Err(CryptoError::Bad("backup_file_corrupt: manifest length exceeds file".into()));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes[start..end])
        .map_err(|e| CryptoError::Bad(format!("backup_file_corrupt: manifest invalid: {e}")))?;
    let ciphertext = bytes[end..].to_vec();
    Ok(ParsedFrame { manifest, ciphertext })
}

pub fn decrypt_payload(parsed: &ParsedFrame, password: &str) -> Result<Vec<u8>, CryptoError> {
    if parsed.manifest.kdf.alg != "argon2id" {
        return Err(CryptoError::Bad("backup_crypto_params_unsupported: unsupported KDF".into()));
    }
    if parsed.manifest.cipher.alg != "aes-256-gcm" {
        return Err(CryptoError::Bad("backup_crypto_params_unsupported: unsupported cipher".into()));
    }
    if parsed.manifest.compression != "gzip" {
        return Err(CryptoError::Bad("backup_crypto_params_unsupported: unsupported compression".into()));
    }
    let salt = B64
        .decode(&parsed.manifest.kdf.salt_b64)
        .map_err(|_| CryptoError::Bad("backup_file_corrupt: salt is not valid base64".into()))?;
    let nonce = B64
        .decode(&parsed.manifest.cipher.nonce_b64)
        .map_err(|_| CryptoError::Bad("backup_file_corrupt: nonce is not valid base64".into()))?;
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::Bad("backup_file_corrupt: nonce length invalid".into()));
    }

    let params = Params::new(
        parsed.manifest.kdf.m_cost,
        parsed.manifest.kdf.t_cost,
        parsed.manifest.kdf.p_cost,
        Some(parsed.manifest.kdf.out_len as usize),
    )
    .map_err(|_| CryptoError::Bad("backup_file_corrupt: kdf params invalid".into()))?;
    let key = derive_key(password, &salt, &params)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| CryptoError::Internal)?;
    let aad = build_aad(
        parsed.manifest.schema_version,
        &parsed.manifest.user_id_original,
        &parsed.manifest.exported_at,
    );
    let compressed = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &parsed.ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;
    gzip_decompress(&compressed)
}

fn build_aad(schema_version: u32, user_id_original: &str, exported_at: &str) -> String {
    format!("ffbk:v{schema_version}:{user_id_original}:{exported_at}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_with(password_enc: &str, password_dec: &str) -> Result<Vec<u8>, CryptoError> {
        let plain = b"{\"hello\":\"world\"}".to_vec();
        let enc = encrypt_payload(
            &plain,
            password_enc,
            "1.0.0-test",
            1,
            "00000000-0000-0000-0000-000000000000",
            "alice",
            "2026-05-14T00:00:00Z",
        )?;
        let framed = frame_file(&enc.manifest, &enc.ciphertext)?;
        let parsed = parse_frame(&framed)?;
        decrypt_payload(&parsed, password_dec)
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let out = roundtrip_with("correct horse battery staple", "correct horse battery staple")
            .unwrap();
        assert_eq!(out, b"{\"hello\":\"world\"}");
    }

    #[test]
    fn wrong_password_fails() {
        let err = roundtrip_with("rightpw1234567", "wrongpw1234567").unwrap_err();
        matches!(err, CryptoError::Decrypt);
    }

    #[test]
    fn tamper_detection() {
        let plain = b"payload".to_vec();
        let enc = encrypt_payload(
            &plain,
            "passpasspass",
            "v",
            1,
            "uid",
            "u",
            "2026-05-14T00:00:00Z",
        )
        .unwrap();
        let mut framed = frame_file(&enc.manifest, &enc.ciphertext).unwrap();
        let last = framed.len() - 1;
        framed[last] ^= 0x01;
        let parsed = parse_frame(&framed).unwrap();
        let err = decrypt_payload(&parsed, "passpasspass").unwrap_err();
        matches!(err, CryptoError::Decrypt);
    }

    #[test]
    fn rejects_bad_magic() {
        let out = parse_frame(b"NOPEv1xx");
        assert!(out.is_err());
    }
}
